//! 测试用的本地假中继：一个最小的 Nostr 中继，只在测试里编译。
//!
//! 实现流程：
//! 1. 在 127.0.0.1 的随机端口上监听 WebSocket，每个连接一个任务。
//! 2. `REQ` 记下订阅（kind 和 `#x`），`CLOSE` 删除订阅。
//! 3. `EVENT` 先回 `OK`，接受的事件转发给所有匹配的订阅（包括发送者自己，这就是「回显」）。
//! 4. `Ping` 回 `Pong`（真中继都回；客户端靠它判断连接还活着）。
//! 5. 可配置的异常行为：不转发（测回显超时）、前 N 条回 `rate-limited`、一律用指定原因拒绝、
//!    收到事件就掐断连接（测「连接真的死了要重连」）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::crypto::{RoomId, hex};

#[derive(Clone, Debug, Default)]
pub struct Behavior {
    /// 接受事件但不转发给任何订阅（连发送者自己也不回显）。
    pub silent: bool,
    /// 前几条事件回 `rate-limited`。
    pub rate_limit_first: usize,
    /// 设置后所有事件都用这个原因拒绝，比如 `"blocked: no"`。
    pub reject: Option<String>,
    /// 收到事件后直接掐断连接（测「连接真的死了要重连」）。
    pub kill_on_event: bool,
}

struct Sub {
    id: String,
    kinds: Vec<u64>,
    rooms: Vec<String>,
}

struct Conn {
    tx: mpsc::UnboundedSender<String>,
    subs: Vec<Sub>,
}

#[derive(Default)]
struct State {
    conns: HashMap<u64, Conn>,
    next_id: u64,
    total_connections: usize,
    attempts: usize,
    accepted: usize,
}

pub struct FakeRelay {
    pub url: String,
    state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}

impl Drop for FakeRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeRelay {
    pub async fn start(behavior: Behavior) -> FakeRelay {
        Self::start_on("127.0.0.1:0".parse().unwrap(), behavior).await
    }

    /// 在指定地址上线（测「网络恢复后立即重连」用）。
    pub async fn start_on(addr: std::net::SocketAddr, behavior: Behavior) -> FakeRelay {
        let listener = TcpListener::bind(addr).await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, shared.clone(), behavior.clone()));
            }
        });
        FakeRelay { url, state, task }
    }

    /// 当前打开的连接数。
    pub fn connections(&self) -> usize {
        self.state.lock().unwrap().conns.len()
    }

    /// 启动以来一共接受过的连接数（重连会让它增加）。
    pub fn total_connections(&self) -> usize {
        self.state.lock().unwrap().total_connections
    }

    /// 订阅了这个房间的连接数。
    pub fn subscribers(&self, room: &RoomId) -> usize {
        let room = hex(room);
        let s = self.state.lock().unwrap();
        s.conns
            .values()
            .filter(|c| c.subs.iter().any(|sub| sub.rooms.contains(&room)))
            .count()
    }

    /// 接受（回了 `OK true`）的事件数。
    pub fn accepted(&self) -> usize {
        self.state.lock().unwrap().accepted
    }
}

async fn serve(stream: tokio::net::TcpStream, state: Arc<Mutex<State>>, behavior: Behavior) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let id = {
        let mut s = state.lock().unwrap();
        s.next_id += 1;
        s.total_connections += 1;
        let id = s.next_id;
        s.conns.insert(
            id,
            Conn {
                tx,
                subs: Vec::new(),
            },
        );
        id
    };
    // 收和发在同一个任务里（和真中继一样）：这样收到 Ping 才能回 Pong。
    loop {
        tokio::select! {
            out = rx.recv() => {
                let Some(text) = out else { break };
                if sink.send(Message::text(text)).await.is_err() { break; }
            }
            msg = source.next() => {
                let Some(Ok(msg)) = msg else { break };
                match msg {
                    Message::Text(t) => {
                        let Ok(Value::Array(a)) = serde_json::from_str::<Value>(t.as_str()) else {
                            continue;
                        };
                        let is_event = a.first().and_then(Value::as_str) == Some("EVENT");
                        handle(&state, id, &a, &behavior);
                        if is_event && behavior.kill_on_event {
                            break; // 连接被掐断
                        }
                    }
                    Message::Ping(data) => {
                        let _ = sink.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }
    state.lock().unwrap().conns.remove(&id);
}

fn handle(state: &Mutex<State>, conn: u64, a: &[Value], behavior: &Behavior) {
    let mut s = state.lock().unwrap();
    match a.first().and_then(Value::as_str) {
        Some("REQ") => {
            let (Some(sub), Some(filter)) = (a.get(1).and_then(Value::as_str), a.get(2)) else {
                return;
            };
            let strings = |k: &str| -> Vec<String> {
                filter[k]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let kinds = filter["kinds"]
                .as_array()
                .map(|v| v.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default();
            let new = Sub {
                id: sub.to_string(),
                kinds,
                rooms: strings("#x"),
            };
            if let Some(c) = s.conns.get_mut(&conn) {
                c.subs.retain(|x| x.id != new.id);
                c.subs.push(new);
            }
        }
        Some("CLOSE") => {
            let sub = a.get(1).and_then(Value::as_str).unwrap_or("");
            if let Some(c) = s.conns.get_mut(&conn) {
                c.subs.retain(|x| x.id != sub);
            }
        }
        Some("EVENT") => {
            let Some(event) = a.get(1) else {
                return;
            };
            let id = event["id"].as_str().unwrap_or("").to_string();
            s.attempts += 1;
            let reply = |ok: bool, msg: &str| json!(["OK", id, ok, msg]).to_string();
            let answer = if s.attempts <= behavior.rate_limit_first {
                reply(false, "rate-limited: slow down")
            } else if let Some(reason) = &behavior.reject {
                reply(false, reason)
            } else {
                s.accepted += 1;
                reply(true, "")
            };
            let accepted = answer.contains("true");
            if let Some(c) = s.conns.get(&conn) {
                let _ = c.tx.send(answer);
            }
            if !accepted || behavior.silent {
                return;
            }
            let kind = event["kind"].as_u64();
            let room = event["tags"]
                .as_array()
                .and_then(|tags| {
                    tags.iter()
                        .find(|t| t[0] == "x")
                        .and_then(|t| t[1].as_str())
                })
                .unwrap_or("")
                .to_string();
            for c in s.conns.values() {
                for sub in &c.subs {
                    if kind.is_some_and(|k| sub.kinds.contains(&k)) && sub.rooms.contains(&room) {
                        let _ = c.tx.send(json!(["EVENT", sub.id, event]).to_string());
                    }
                }
            }
        }
        _ => {}
    }
}
