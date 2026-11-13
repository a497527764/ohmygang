//! OH-MY-GANG! 自有云端服务入口。
//!
//! 一个端口同时承担两件事：Nostr 风格中继（WebSocket，客户端直接把 relay 指向这里）和短码 API（HTTP）。
//! 回收任务每 10 分钟扫一遍：超过 5 天无活动、或连续 5 天不足 2 人的队伍，解散并回收其短码。

mod api;
mod net;
mod protocol;
mod relay;
mod store;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::TcpListener;

use relay::RelayServer;
use store::Store;

const DAY: i64 = 86400;
const IDLE_DAYS: i64 = 5; // 超过这么久无活动 → 回收
const FEW_DAYS: i64 = 5; // 连续这么久不足 2 人 → 回收

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let db_path = std::env::var("DATA_DIR").unwrap_or_else(|_| "ohmygang.db".to_string());

    let store = Arc::new(Store::open(&db_path)?);
    let blacklist: HashSet<String> = store.load_blacklist()?.into_iter().collect();
    let blacklist = Arc::new(Mutex::new(blacklist));
    let registry = relay::registry_from_store(&store);
    let server = RelayServer::new(registry, store.clone(), blacklist);

    // 回收任务：每 10 分钟扫一遍
    tokio::spawn(run_reaper(server.clone(), store.clone()));

    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    eprintln!("OH-MY-GANG! 云端服务已启动，监听 :{port}");
    net::serve(listener, server, store).await?;
    Ok(())
}

async fn run_reaper(server: Arc<RelayServer>, store: Arc<Store>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(600));
    loop {
        ticker.tick().await;
        reap(&server, &store).await;
    }
}

async fn reap(server: &Arc<RelayServer>, store: &Arc<Store>) {
    let now = api::now_secs();
    // 建队配额只看最近 24 小时，更早的流水清掉，表才不会一直长。
    let _ = store.prune_create_events(now - DAY);
    let teams = store.list_teams().unwrap_or_default();
    for t in teams {
        let activity = server
            .registry()
            .activity
            .lock()
            .ok()
            .and_then(|a| a.get(&t.room_id_hex).copied())
            .unwrap_or(t.last_active);
        // 规则一：长期无活动
        if now - activity > IDLE_DAYS * DAY {
            server.dissolve_room(&t.room_id_hex);
            eprintln!("回收（{IDLE_DAYS} 天无活动）：{room}", room = t.room_id_hex);
            continue;
        }
        // 规则二：连续不足 2 人
        let members = server
            .registry()
            .members
            .lock()
            .ok()
            .and_then(|m| m.get(&t.room_id_hex).cloned())
            .unwrap_or_default();
        let recent = members
            .values()
            .filter(|&&seen| now - seen < FEW_DAYS * DAY)
            .count();
        if recent < 2 {
            match t.low_since {
                None => {
                    let _ = store.update_team_state(&t.room_id_hex, activity, Some(now));
                }
                Some(s) if now - s > FEW_DAYS * DAY => {
                    server.dissolve_room(&t.room_id_hex);
                    eprintln!(
                        "回收（连续 {FEW_DAYS} 天不足 2 人）：{room}",
                        room = t.room_id_hex
                    );
                    continue;
                }
                Some(_) => {}
            }
        } else if t.low_since.is_some() {
            let _ = store.update_team_state(&t.room_id_hex, activity, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    fn hex(n: usize) -> String {
        "ab".repeat(n)
    }

    async fn test_server() -> (Arc<RelayServer>, Arc<Store>, std::net::SocketAddr) {
        let store = Arc::new(Store::open(":memory:").unwrap());
        let blacklist: StdMutex<HashSet<String>> = StdMutex::new(HashSet::new());
        let registry = relay::registry_from_store(&store);
        let server = RelayServer::new(registry, store.clone(), Arc::new(blacklist));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(net::serve(listener, server.clone(), store.clone()));
        (server, store, addr)
    }

    #[tokio::test]
    async fn api_create_join_heartbeat() {
        let (server, store, _addr) = test_server().await;
        let me = hex(32);
        let (status, v) = api::handle_json(
            "POST",
            "/api/team/create",
            serde_json::to_vec(&json!({ "member_id": me }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        assert_eq!(status, 200);
        let short = v["short_code"].as_str().unwrap().to_string();
        assert_eq!(short.len(), 6);
        assert!(
            short
                .bytes()
                .all(|b| b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ".contains(&b)),
            "短码应为大写字母数字"
        );
        assert!(v["code"].as_str().unwrap().starts_with("GT2-"));
        let room = v["room_id"].as_str().unwrap().to_string();
        assert_eq!(room.len(), 32);

        let (status2, v2) = api::handle_json(
            "POST",
            "/api/team/join",
            serde_json::to_vec(&json!({ "code": v["short_code"] }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        assert_eq!(status2, 200);
        assert_eq!(v2["room_id"], room);
        assert_eq!(v2["code"], v["code"]);

        let (status3, _v3) = api::handle_json(
            "POST",
            "/api/team/join",
            serde_json::to_vec(&json!({ "code": "AAAAAA" }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        assert_eq!(status3, 404, "不存在的短码应 404");

        let (status3b, _v3b) = api::handle_json(
            "POST",
            "/api/team/join",
            serde_json::to_vec(&json!({ "code": "IIIIII" }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        assert_eq!(status3b, 400, "含易混字符的码应 400");

        let (status4, v4) = api::handle_json(
            "POST",
            "/api/team/heartbeat",
            serde_json::to_vec(&json!({ "room_id": room, "member_id": me }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        let ok = v4.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
        assert!(status4 == 200 && ok, "心跳应成功");
    }

    /// 队名要存在服务器上：中继的事件缓存是内存的，服务器一重启，房主的建队事件就没了，
    /// 新加入的人拿不到队名。名字落库，进队时再发给客户端。
    #[tokio::test]
    async fn team_title_survives_on_the_server() {
        let (server, store, _addr) = test_server().await;
        let me = hex(32);
        let (status, v) = api::handle_json(
            "POST",
            "/api/team/create",
            serde_json::to_vec(&json!({ "member_id": me, "title": "开黑小队" }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        assert_eq!(status, 200);
        let short = v["short_code"].as_str().unwrap().to_string();

        // 换个人用短码进来，队名要跟着下发
        let (status2, v2) = api::handle_json(
            "POST",
            "/api/team/join",
            serde_json::to_vec(&json!({ "code": short }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        assert_eq!(status2, 200);
        assert_eq!(v2["title"].as_str(), Some("开黑小队"));

        // 房主改队名，服务器上要跟着变
        let room = v["room_id"].as_str().unwrap().to_string();
        let (status3, _v3) = api::handle_json(
            "POST",
            "/api/team/rename",
            serde_json::to_vec(&json!({ "room_id": room, "title": "开黑小队 2" }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        assert_eq!(status3, 200);

        let (status4, v4) = api::handle_json(
            "POST",
            "/api/team/join",
            serde_json::to_vec(&json!({ "code": short }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "9.9.9.9",
        )
        .await;
        assert_eq!(status4, 200);
        assert_eq!(v4["title"].as_str(), Some("开黑小队 2"));
    }

    #[tokio::test]
    async fn reaper_dissolves_idle_team() {
        let (server, store, _addr) = test_server().await;
        let me_bytes: [u8; 32] = rand::random();
        let me_hex = crate::protocol::hex(&me_bytes);
        let secret: [u8; 16] = rand::random();
        let owner = crate::protocol::owner_tag_of(&me_bytes);
        let (_k, room_id) = crate::protocol::derive_room_keys(&secret, &owner);
        let room_hex = crate::protocol::hex(&room_id);
        // 6 天前就无活动
        let old = api::now_secs() - 6 * DAY;
        store
            .create_team("123456", &secret, &owner, &room_hex, old, &me_hex, None)
            .unwrap();
        reap(&server, &store).await;
        assert!(
            store.short_by_room(&room_hex).unwrap().is_none(),
            "长期无活动的队伍应被回收"
        );
    }

    #[tokio::test]
    async fn ws_relay_roundtrip() {
        let (server, store, addr) = test_server().await;
        let url = format!("ws://{addr}");
        let me = hex(32);
        let (_status, v) = api::handle_json(
            "POST",
            "/api/team/create",
            serde_json::to_vec(&json!({ "member_id": me }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "1.1.1.1",
        )
        .await;
        let room = v["room_id"].as_str().unwrap().to_string();

        let (b_ws, _) = connect_async(url.clone()).await.unwrap();
        let (mut b_tx, mut b_rx) = b_ws.split();
        b_tx.send(
            json!(["REQ", "s1", { "kinds": [24810], "#x": [room] }])
                .to_string()
                .into(),
        )
        .await
        .unwrap();

        let (a_ws, _) = connect_async(url.clone()).await.unwrap();
        let (mut a_tx, mut _a_rx) = a_ws.split();
        let event = json!({
            "id": "a1", "pubkey": "aa", "created_at": 1, "kind": 24810,
            "tags": [["x", room]], "content": "AAAA", "sig": "00"
        });
        a_tx.send(json!(["EVENT", event]).to_string().into())
            .await
            .unwrap();

        let mut got = false;
        for _ in 0..30 {
            match b_rx.next().await {
                Some(Ok(Message::Text(t))) => {
                    if t.to_string().contains("EVENT") {
                        got = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(got, "订阅方 B 应收到发布方 A 的事件");
    }

    // ---------- 防攻击与配额 ----------

    /// 全局建队配额：一小时最多 10 支。第 11 支必须被拒，哪怕来自完全不同的 IP。
    #[tokio::test]
    async fn create_quota_blocks_the_eleventh_team() {
        let (server, store, _addr) = test_server().await;
        let me = hex(32);
        // 每个请求用不同的 IP：这样卡住的才是全局配额，而不是单 IP 限流。
        for i in 0..10 {
            let peer = format!("10.0.0.{i}");
            let (status, _v) = api::handle_json(
                "POST",
                "/api/team/create",
                serde_json::to_vec(&json!({ "member_id": me }))
                    .unwrap()
                    .as_slice(),
                &server,
                &store,
                &peer,
            )
            .await;
            assert_eq!(status, 200, "第 {} 支应建得出来", i + 1);
        }
        let (status, v) = api::handle_json(
            "POST",
            "/api/team/create",
            serde_json::to_vec(&json!({ "member_id": me }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "10.0.0.200",
        )
        .await;
        assert_eq!(status, 429, "第 11 支应被配额拦下");
        assert!(
            v["error"].as_str().unwrap_or("").contains("额度"),
            "错误提示要说清是额度用尽：{}",
            v["error"]
        );
    }

    /// 单个 IP 反复建队要被打掉（IP 限流比全局配额更早触发）。
    #[tokio::test]
    async fn create_flood_from_one_ip_is_blocked() {
        let (server, store, _addr) = test_server().await;
        let me = hex(32);
        let mut last = 200;
        for _ in 0..12 {
            let (status, _v) = api::handle_json(
                "POST",
                "/api/team/create",
                serde_json::to_vec(&json!({ "member_id": me }))
                    .unwrap()
                    .as_slice(),
                &server,
                &store,
                "172.16.0.9",
            )
            .await;
            if status != 200 {
                last = status;
                break;
            }
        }
        assert_eq!(last, 429, "同一 IP 狂建队应被限流");
    }

    /// 配额是记账的：队伍被回收删除也不能把额度退回来。
    #[tokio::test]
    async fn dissolving_a_team_does_not_refund_the_quota() {
        let (server, store, _addr) = test_server().await;
        let me = hex(32);
        let mut codes = Vec::new();
        for i in 0..10 {
            let peer = format!("10.1.0.{i}");
            let (_s, v) = api::handle_json(
                "POST",
                "/api/team/create",
                serde_json::to_vec(&json!({ "member_id": me }))
                    .unwrap()
                    .as_slice(),
                &server,
                &store,
                &peer,
            )
            .await;
            codes.push(v["short_code"].as_str().unwrap().to_string());
        }
        // 删掉一支，额度不还
        store.dissolve_by_short(&codes[0]).unwrap();
        let (status, _v) = api::handle_json(
            "POST",
            "/api/team/create",
            serde_json::to_vec(&json!({ "member_id": me }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "10.1.0.200",
        )
        .await;
        assert_eq!(status, 429, "删掉队伍不应把建队额度退回来");
    }

    /// 进队接口刷太多要被打掉，不能被当成放大器。
    #[tokio::test]
    async fn join_flood_is_blocked() {
        let (server, store, _addr) = test_server().await;
        let mut last = 200;
        for _ in 0..80 {
            let (status, _v) = api::handle_json(
                "POST",
                "/api/team/join",
                serde_json::to_vec(&json!({ "code": "ZZZZZZ" }))
                    .unwrap()
                    .as_slice(),
                &server,
                &store,
                "192.168.7.7",
            )
            .await;
            if status == 429 {
                last = status;
                break;
            }
        }
        assert_eq!(last, 429, "狂刷进队接口应被限流");
    }

    /// 后台登录要限流：密码只有 6 位，不能让人一路爆破。
    #[tokio::test]
    async fn admin_login_flood_is_blocked() {
        let (server, store, _addr) = test_server().await;
        let mut last = 200;
        for _ in 0..20 {
            let (status, _v) = api::handle_json(
                "POST",
                "/api/admin/login",
                serde_json::to_vec(&json!({ "username": "zhaopeng", "password": "000000" }))
                    .unwrap()
                    .as_slice(),
                &server,
                &store,
                "192.168.9.9",
            )
            .await;
            if status == 429 {
                last = status;
                break;
            }
        }
        assert_eq!(last, 429, "狂试后台登录应被限流");
    }

    // ---------- 后台批量管理 ----------

    /// 批量删除：一次删多支，删完列表里就没有了。
    #[tokio::test]
    async fn admin_batch_delete_removes_many_teams() {
        let (server, store, _addr) = test_server().await;
        let me = hex(32);
        let mut codes = Vec::new();
        for i in 0..3 {
            let peer = format!("10.2.0.{i}");
            let (_s, v) = api::handle_json(
                "POST",
                "/api/team/create",
                serde_json::to_vec(&json!({ "member_id": me }))
                    .unwrap()
                    .as_slice(),
                &server,
                &store,
                &peer,
            )
            .await;
            codes.push(v["short_code"].as_str().unwrap().to_string());
        }

        // 登录拿 token
        let (status, v) = api::handle_json(
            "POST",
            "/api/admin/login",
            serde_json::to_vec(&json!({ "username": "zhaopeng", "password": "000000" }))
                .unwrap()
                .as_slice(),
            &server,
            &store,
            "10.2.0.100",
        )
        .await;
        assert_eq!(status, 200);
        let token = v["token"].as_str().unwrap().to_string();

        // 列表里能看到队名和完整邀请码
        let path = format!("/api/admin/teams?token={token}");
        let (status, v) = api::handle_json("GET", &path, b"", &server, &store, "10.2.0.101").await;
        assert_eq!(status, 200);
        let teams = v["teams"].as_array().unwrap();
        assert_eq!(teams.len(), 3);
        assert!(
            teams[0]["code"].as_str().unwrap_or("").starts_with("GT2-"),
            "后台列表要带完整邀请码，方便一键复制"
        );

        // 批量删前两支
        let body = serde_json::to_vec(&json!({
            "short_codes": [codes[0], codes[1]],
            "token": token,
        }))
        .unwrap();
        let (status, v) = api::handle_json(
            "POST",
            "/api/admin/teams/delete_batch",
            body.as_slice(),
            &server,
            &store,
            "10.2.0.102",
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(v["deleted"], 2);

        // 只剩一支
        let path = format!("/api/admin/teams?token={token}");
        let (_s, v) = api::handle_json("GET", &path, b"", &server, &store, "10.2.0.103").await;
        assert_eq!(v["teams"].as_array().unwrap().len(), 1);
    }

    /// 批量删除必须校验 token，不能谁都能删。
    #[tokio::test]
    async fn admin_batch_delete_needs_token() {
        let (server, store, _addr) = test_server().await;
        let body = serde_json::to_vec(&json!({ "short_codes": ["AAAAAA"] })).unwrap();
        let (status, _v) = api::handle_json(
            "POST",
            "/api/admin/teams/delete_batch",
            body.as_slice(),
            &server,
            &store,
            "10.3.0.1",
        )
        .await;
        assert_eq!(status, 401, "没 token 不许批量删");
    }
}
