//! Nostr 风格中继 + 房间活跃登记表。
//!
//! 中继协议（和客户端 `transport/nostr.rs` 逐一对齐）：事件用 kind 24810，标签 `x` 为 room_id 十六进制，
//! content 为包的 Base64；支持 `EVENT` / `REQ` / `CLOSE`，回复 `OK` / `EOSE`。客户端只按房间过滤订阅，
//! 不验证中继层签名（包本身用房间密钥认证加密），所以我们原样转发收到的事件。
//!
//! 回收：中继在收到 `EVENT` 或心跳时更新房间的活跃时间与成员，登记在 `Registry`；回收任务据此解散死队。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::store::Store;

const KIND: u16 = 24810;
const MAX_PACKET_LEN: usize = 1200;
const EVENT_CAP: usize = 256;
/// 一个包 Base64 后最多 1600 字符（1200 字节上限）。
const MAX_CONTENT_LEN: usize = 1600;

/// 房间活跃登记：中继只看到 room_id（事件的 `x` 标签），看不到明文成员，
/// 所以人数来自客户端的心跳上报。
pub struct Registry {
    /// 每个房间最近一次有活动（EVENT 或心跳）的 unix 秒。
    pub activity: Mutex<HashMap<String, i64>>,
    /// 每个房间最近一次心跳的成员及其时间（unix 秒）。
    pub members: Mutex<HashMap<String, HashMap<String, i64>>>,
}

impl Registry {
    fn load(
        activity: HashMap<String, i64>,
        members: HashMap<String, HashMap<String, i64>>,
    ) -> Self {
        Registry {
            activity: Mutex::new(activity),
            members: Mutex::new(members),
        }
    }
}

type Tx = mpsc::UnboundedSender<String>;

struct Subscription {
    sub_id: String,
    rooms: HashSet<String>,
}

pub struct RelayServer {
    registry: Arc<Registry>,
    store: Arc<Store>,
    blacklist: Arc<Mutex<HashSet<String>>>,
    conns: Mutex<HashMap<usize, Tx>>,
    subs: Mutex<HashMap<usize, Vec<Subscription>>>,
    events: Mutex<HashMap<String, VecDeque<Value>>>,
    next_id: AtomicUsize,
}

impl RelayServer {
    pub fn new(
        registry: Arc<Registry>,
        store: Arc<Store>,
        blacklist: Arc<Mutex<HashSet<String>>>,
    ) -> Arc<Self> {
        Arc::new(RelayServer {
            registry,
            store,
            blacklist,
            conns: Mutex::new(HashMap::new()),
            subs: Mutex::new(HashMap::new()),
            events: Mutex::new(HashMap::new()),
            next_id: AtomicUsize::new(1),
        })
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn is_blacklisted(&self, room_hex: &str) -> bool {
        self.blacklist
            .lock()
            .map(|b| b.contains(room_hex))
            .unwrap_or(false)
    }

    /// 一支队伍被回收：落盘删除、从登记表移除、room_id 进黑名单、清空内存事件。
    pub fn dissolve_room(&self, room_hex: &str) {
        let _ = self.store.dissolve(room_hex);
        if let Ok(mut b) = self.blacklist.lock() {
            b.insert(room_hex.to_string());
        }
        if let Ok(mut a) = self.registry.activity.lock() {
            a.remove(room_hex);
        }
        if let Ok(mut m) = self.registry.members.lock() {
            m.remove(room_hex);
        }
        if let Ok(mut e) = self.events.lock() {
            e.remove(room_hex);
        }
    }

    /// 收到一条事件：登记活跃、入内存缓冲，转发给订阅了该房间的其它连接。
    /// 返回是否接受（被黑名单拒收返回 false）。
    fn publish(&self, from: usize, room_hex: &str, event: Value) -> bool {
        if self.is_blacklisted(room_hex) {
            return false;
        }
        if let Ok(mut a) = self.registry.activity.lock() {
            a.insert(room_hex.to_string(), Self::now_secs());
        }
        if let Ok(mut e) = self.events.lock() {
            let buf = e.entry(room_hex.to_string()).or_default();
            buf.push_back(event.clone());
            if buf.len() > EVENT_CAP {
                buf.pop_front();
            }
        }
        if let (Ok(conns), Ok(subs)) = (self.conns.lock(), self.subs.lock()) {
            for (cid, tx) in conns.iter() {
                if *cid == from {
                    continue; // 自己的事件不回显（客户端也会按公钥去重，这里顺手省带宽）
                }
                if let Some(list) = subs.get(cid) {
                    for s in list {
                        if s.rooms.contains(room_hex) {
                            let msg = json!(["EVENT", s.sub_id, event]).to_string();
                            let _ = tx.send(msg);
                        }
                    }
                }
            }
        }
        true
    }

    fn handle_req(&self, conn_id: usize, sub_id: &str, filter: &Value) {
        let rooms = extract_rooms(filter);
        // 记录订阅
        if let Ok(mut subs) = self.subs.lock() {
            let list = subs.entry(conn_id).or_default();
            list.retain(|s| s.sub_id != sub_id); // 同名订阅替换
            list.push(Subscription {
                sub_id: sub_id.to_string(),
                rooms: rooms.clone(),
            });
        }
        // 回放内存里该房间的存量事件（EOSE 前补发），再发 EOSE
        if let Ok(events) = self.events.lock() {
            for room in &rooms {
                if let Some(buf) = events.get(room) {
                    for ev in buf {
                        if let Some(tx) = self
                            .conns
                            .lock()
                            .ok()
                            .and_then(|c| c.get(&conn_id).cloned())
                        {
                            let msg = json!(["EVENT", sub_id, ev]).to_string();
                            let _ = tx.send(msg);
                        }
                    }
                }
            }
        }
        if let Some(tx) = self
            .conns
            .lock()
            .ok()
            .and_then(|c| c.get(&conn_id).cloned())
        {
            let _ = tx.send(json!(["EOSE", sub_id]).to_string());
        }
    }

    fn handle_close(&self, conn_id: usize, sub_id: &str) {
        if let Ok(mut subs) = self.subs.lock()
            && let Some(list) = subs.get_mut(&conn_id)
        {
            list.retain(|s| s.sub_id != sub_id);
        }
    }

    /// 处理一个连接：读入站 WS 消息（EVENT/REQ/CLOSE），把出站消息（EVENT/EOSE/OK/NOTICE）写回。
    pub async fn run_ws<S>(self: &Arc<Self>, ws: WebSocketStream<S>)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let conn_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (mut sink, mut source) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        self.conns.lock().map(|mut c| c.insert(conn_id, tx)).ok();

        loop {
            tokio::select! {
                // 出站：中继/回放/OK/NOTICE
                out = rx.recv() => {
                    let Some(msg) = out else { break };
                    if sink.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                // 入站：客户端发来的 EVENT/REQ/CLOSE
                msg = source.next() => {
                    let Some(frame) = msg else { break };
                    match frame {
                        Ok(Message::Text(t)) => {
                            let text = t.to_string();
                            if text.is_empty() { continue; }
                            self.on_client_text(conn_id, &text, &mut sink).await;
                        }
                        // 心跳：回一个 Pong。客户端靠它判断这条连接还活着（梯子/NAT 会掐空闲连接）
                        Ok(Message::Ping(data)) => {
                            if sink.send(Message::Pong(data)).await.is_err() { break; }
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            }
        }
        // 连接结束：清掉它
        if let Ok(mut c) = self.conns.lock() {
            c.remove(&conn_id);
        }
        if let Ok(mut s) = self.subs.lock() {
            s.remove(&conn_id);
        }
    }

    async fn on_client_text<S>(&self, conn_id: usize, text: &str, sink: &mut S)
    where
        S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let Some(cmd) = v.get(0).and_then(|c| c.as_str()) else {
            return;
        };
        match cmd {
            "EVENT" => {
                let Some(event) = v.get(1) else {
                    return;
                };
                let Some(id) = event.get("id").and_then(|x| x.as_str()) else {
                    return;
                };
                let Some(room_hex) = extract_room(event) else {
                    let _ = sink
                        .send(Message::Text(
                            json!(["OK", id, false, "invalid: missing room tag"])
                                .to_string()
                                .into(),
                        ))
                        .await;
                    return;
                };
                if event.get("kind").and_then(|k| k.as_i64()) != Some(KIND as i64) {
                    let _ = sink
                        .send(Message::Text(
                            json!(["OK", id, false, "invalid: wrong kind"])
                                .to_string()
                                .into(),
                        ))
                        .await;
                    return;
                }
                if !valid_content(event) {
                    let _ = sink
                        .send(Message::Text(
                            json!(["OK", id, false, "invalid: bad packet"])
                                .to_string()
                                .into(),
                        ))
                        .await;
                    return;
                }
                let accepted = self.publish(conn_id, &room_hex, event.clone());
                let notice = if accepted {
                    None
                } else {
                    Some("队伍已解散，无法发送")
                };
                let _ = sink
                    .send(Message::Text(
                        json!([
                            "OK",
                            id,
                            accepted,
                            if accepted {
                                ""
                            } else {
                                "blocked: room dissolved"
                            }
                        ])
                        .to_string()
                        .into(),
                    ))
                    .await;
                if let Some(n) = notice {
                    let _ = sink
                        .send(Message::Text(json!(["NOTICE", n]).to_string().into()))
                        .await;
                }
            }
            "REQ" => {
                let Some(sub_id) = v.get(1).and_then(|x| x.as_str()) else {
                    return;
                };
                let filter = v.get(2).cloned().unwrap_or(Value::Null);
                self.handle_req(conn_id, sub_id, &filter);
            }
            "CLOSE" => {
                let Some(sub_id) = v.get(1).and_then(|x| x.as_str()) else {
                    return;
                };
                self.handle_close(conn_id, sub_id);
            }
            _ => {}
        }
    }
}

/// 从事件标签里取 room_id 十六进制（`x`）。
fn extract_room(event: &Value) -> Option<String> {
    let tags = event.get("tags")?.as_array()?;
    for t in tags {
        let a = t.as_array()?;
        if a.len() >= 2 && a[0].as_str() == Some("x") {
            return a[1].as_str().map(|s| s.to_lowercase());
        }
    }
    None
}

/// 订阅过滤器里的 `#x`（房间列表）。
fn extract_rooms(filter: &Value) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Some(arr) = filter.get("#x").and_then(|x| x.as_array()) {
        for r in arr {
            if let Some(s) = r.as_str() {
                out.insert(s.to_lowercase());
            }
        }
    }
    out
}

/// 校验 content 是合法 Base64 且解码后不超过包上限。
fn valid_content(event: &Value) -> bool {
    let Some(content) = event.get("content").and_then(|c| c.as_str()) else {
        return false;
    };
    if content.len() > MAX_CONTENT_LEN {
        return false;
    }
    match base64_decode(content) {
        Some(bytes) => bytes.len() <= MAX_PACKET_LEN,
        None => false,
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .ok()
        .or_else(|| {
            // Nostr 中继常用 URL-safe 字母表，一并兼容
            base64::engine::general_purpose::URL_SAFE
                .decode(s.trim())
                .ok()
        })
}

/// 由持久化数据重建登记表（进程重启后回收规则仍可正确运行）。
pub fn registry_from_store(store: &Store) -> Arc<Registry> {
    let activity = store.load_activity().unwrap_or_default();
    let members = store.load_members().unwrap_or_default();
    Arc::new(Registry::load(activity, members))
}
