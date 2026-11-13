//! 短码 HTTP API：建队（分配 6 位短码 + 铸造 GT2 长码）、进队（短码换长码）、心跳（上报人数）、查码。
//!
//! 短码是「别名」：服务端把它映射到 room_secret + owner_tag，铸成 GT2 长码交给客户端；
//! 客户端用 GT2 长码像往常一样派生 room_key / room_id。服务端据此真正拥有并回收队伍码。

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::relay::RelayServer;
use crate::store::Store;

/// 限流：同一 IP 每小时最多 30 次建队，防有人把短码池占满。
static RATE: OnceLock<Mutex<std::collections::HashMap<String, (i64, u32)>>> = OnceLock::new();

/// 管理后台登录凭证（自用后台，硬编码即可）。
const ADMIN_USER: &str = "zhaopeng";
const ADMIN_PASS: &str = "000000";
/// 管理会话 token：启动时随机生成，登录成功后发给前端。
static ADMIN_TOKEN: OnceLock<String> = OnceLock::new();

fn admin_token() -> &'static str {
    ADMIN_TOKEN.get_or_init(|| {
        let b: [u8; 16] = rand::random();
        b.iter().map(|x| format!("{x:02x}")).collect()
    })
}

/// 检查请求是否带了有效的管理 token（从 Authorization: Bearer 或 ?token= 取）。
fn check_admin(path: &str, body: &[u8]) -> bool {
    // 先从 query string 取
    if let Some(q) = path.split('?').nth(1) {
        for p in q.split('&') {
            if let Some(t) = p.strip_prefix("token=") {
                if t == admin_token() {
                    return true;
                }
            }
        }
    }
    // 再从 body 取（POST 时前端用 JSON 传 token）
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        if v.get("token").and_then(|x| x.as_str()) == Some(admin_token()) {
            return true;
        }
    }
    false
}

/// 短码字母表：大写字母数字，去掉 0/O/1/I 等易混字符（32 个字符）。
const SHORT_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

fn gen_short() -> String {
    (0..6)
        .map(|_| {
            let i = (rand::random::<u32>() % SHORT_ALPHABET.len() as u32) as usize;
            SHORT_ALPHABET[i] as char
        })
        .collect()
}

fn rate_ok(peer: &str) -> bool {
    const LIMIT: u32 = 30;
    const WINDOW: i64 = 3600;
    let now = now_secs();
    let map = RATE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut map = map.lock().expect("限流锁没有中毒");
    let e = map.entry(peer.to_string()).or_insert((now, 0));
    if now - e.0 >= WINDOW {
        *e = (now, 0);
    }
    if e.1 >= LIMIT {
        false
    } else {
        e.1 += 1;
        true
    }
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 网络层入口：返回完整的 HTTP 响应字符串（含状态行与头部）。
pub async fn handle(
    method: &str,
    path: &str,
    body: &[u8],
    server: &Arc<RelayServer>,
    store: &Arc<Store>,
    peer: &str,
) -> String {
    let route_path = path.split('?').next().unwrap_or(path);
    // 管理页面直接返回 HTML
    if method == "GET" && route_path == "/admin" {
        let html = ADMIN_PAGE;
        return format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
    }
    let (status, body) = route(method, path, body, server, store, peer).await;
    json_resp(status, body)
}

/// 测试 / 内部入口：只返回 JSON 结构体与状态码，不含 HTTP 头部。
pub async fn handle_json(
    method: &str,
    path: &str,
    body: &[u8],
    server: &Arc<RelayServer>,
    store: &Arc<Store>,
    peer: &str,
) -> (u16, Value) {
    route(method, path, body, server, store, peer).await
}

async fn route(
    method: &str,
    path: &str,
    body: &[u8],
    server: &Arc<RelayServer>,
    store: &Arc<Store>,
    peer: &str,
) -> (u16, Value) {
    let route = path.split('?').next().unwrap_or(path);
    match (method, route) {
        ("GET", "/") | ("GET", "/health") => (200, json!({ "ok": true })),
        ("POST", "/api/team/create") => create(body, store, server, peer).await,
        ("POST", "/api/team/join") => join(body, store).await,
        ("POST", "/api/team/heartbeat") => heartbeat(body, store, server).await,
        ("POST", "/api/team/stats") => stats(body, store).await,
        ("GET", "/api/team/code") => code_lookup(path, store).await,
        // 管理后台 API
        ("POST", "/api/admin/login") => admin_login(body),
        ("GET", "/api/admin/teams") => {
            if check_admin(path, body) {
                admin_teams(store)
            } else {
                (401, json!({ "error": "未登录" }))
            }
        }
        ("POST", "/api/admin/teams/delete") => {
            if check_admin(path, body) {
                admin_delete(body, store, server)
            } else {
                (401, json!({ "error": "未登录" }))
            }
        }
        // 更新包上传（发版后由 CI 或本机推送）：body 是原始文件字节，token 在环境变量 UPLOAD_TOKEN
        ("POST", "/api/admin/update") => upload_update(path, body),
        _ => (404, json!({ "error": "not found" })),
    }
}

fn parse_hex(s: &str, len: usize) -> Option<Vec<u8>> {
    let b = crate::protocol::unhex(s)?;
    if b.len() == len { Some(b) } else { None }
}

async fn create(
    body: &[u8],
    store: &Arc<Store>,
    server: &Arc<RelayServer>,
    peer: &str,
) -> (u16, Value) {
    if !rate_ok(peer) {
        return (429, json!({ "error": "请求过于频繁，请稍后再试" }));
    }
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({ "error": "bad json" })),
    };
    let member_hex = match v.get("member_id").and_then(|x| x.as_str()) {
        Some(s) => s.to_string(),
        None => return (400, json!({ "error": "缺少 member_id" })),
    };
    let member_id: [u8; 32] = match parse_hex(&member_hex, 32) {
        Some(b) => b.try_into().unwrap(),
        None => return (400, json!({ "error": "member_id 非法" })),
    };
    // 铸造房间密钥材料
    let secret: [u8; 16] = rand::random();
    let owner_tag = crate::protocol::owner_tag_of(&member_id);
    let (_key, room_id) = crate::protocol::derive_room_keys(&secret, &owner_tag);
    let room_id_hex = crate::protocol::hex(&room_id);
    // 6 位大写字母数字短码（去掉 0/O/1/I 等易混字符），确保不重复（短码池很大，冲突概率极低）
    let short = loop {
        let c = gen_short();
        if !store.team_exists(&c).unwrap_or(true) {
            break c;
        }
    };
    let now = now_secs();
    if store
        .create_team(&short, &secret, &owner_tag, &room_id_hex, now, &member_hex)
        .is_err()
    {
        return (500, json!({ "error": "internal" }));
    }
    // 同步内存登记表：创建者即首名成员，且刚活跃
    if let Ok(mut a) = server.registry().activity.lock() {
        a.insert(room_id_hex.clone(), now);
    }
    if let Ok(mut m) = server.registry().members.lock() {
        m.entry(room_id_hex.clone())
            .or_default()
            .insert(member_hex, now);
    }
    let code = crate::protocol::format_code(&secret, &owner_tag);
    (
        200,
        json!({ "short_code": short, "code": code, "room_id": room_id_hex }),
    )
}

async fn join(body: &[u8], store: &Arc<Store>) -> (u16, Value) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({ "error": "bad json" })),
    };
    let code = match v.get("code").and_then(|x| x.as_str()) {
        Some(s) => s.to_string(),
        None => return (400, json!({ "error": "缺少 code" })),
    };
    let normalized: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if normalized.len() != 6 || !normalized.bytes().all(|b| SHORT_ALPHABET.contains(&b)) {
        return (400, json!({ "error": "队伍码应为 6 位字母数字" }));
    }
    match store.team_by_short(&normalized) {
        Ok(Some((code_str, room_id_hex))) => {
            (200, json!({ "code": code_str, "room_id": room_id_hex }))
        }
        Ok(None) => (404, json!({ "error": "队伍码不存在或已过期" })),
        Err(_) => (500, json!({ "error": "internal" })),
    }
}

async fn heartbeat(body: &[u8], store: &Arc<Store>, server: &Arc<RelayServer>) -> (u16, Value) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({ "error": "bad json" })),
    };
    let room_id = match v.get("room_id").and_then(|x| x.as_str()) {
        Some(s) => s.to_string(),
        None => return (400, json!({ "error": "缺少 room_id" })),
    };
    let member_hex = match v.get("member_id").and_then(|x| x.as_str()) {
        Some(s) => s.to_string(),
        None => return (400, json!({ "error": "缺少 member_id" })),
    };
    if parse_hex(&room_id, 16).is_none() || parse_hex(&member_hex, 32).is_none() {
        return (400, json!({ "error": "参数非法" }));
    }
    // 只对仍存在的队伍计数，避免给已回收的房间续命
    let exists = store.short_by_room(&room_id).ok().flatten().is_some();
    if !exists {
        return (404, json!({ "error": "队伍不存在或已过期" }));
    }
    let now = now_secs();
    let _ = store.upsert_member(&room_id, &member_hex, now);
    if let Ok(mut a) = server.registry().activity.lock() {
        a.insert(room_id.clone(), now);
    }
    if let Ok(mut m) = server.registry().members.lock() {
        m.entry(room_id).or_default().insert(member_hex, now);
    }
    (200, json!({ "ok": true }))
}

/// 主页「加入过的队伍」的在线人数：15 分钟内报过心跳（每 10 分钟一次）就算在线。
async fn stats(body: &[u8], store: &Arc<Store>) -> (u16, Value) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({ "error": "bad json" })),
    };
    let Some(rooms) = v.get("room_ids").and_then(|x| x.as_array()) else {
        return (400, json!({ "error": "缺少 room_ids" }));
    };
    if rooms.len() > 20 {
        return (400, json!({ "error": "room_ids 太多" }));
    }
    let since = now_secs() - 15 * 60;
    let mut out = serde_json::Map::new();
    for r in rooms {
        let Some(room) = r.as_str() else { continue };
        if parse_hex(room, 16).is_none() {
            continue;
        }
        if let Ok(n) = store.room_online(room, since) {
            out.insert(room.to_string(), json!(n));
        }
    }
    (200, json!({ "stats": Value::Object(out) }))
}

async fn code_lookup(path: &str, store: &Arc<Store>) -> (u16, Value) {
    let room = path
        .split('?')
        .nth(1)
        .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("room=")))
        .unwrap_or("");
    match store.short_by_room(room).ok().flatten() {
        Some(short) => (200, json!({ "short_code": short })),
        None => (404, json!({ "error": "not found" })),
    }
}

fn json_resp(status: u16, body: Value) -> String {
    let s = body.to_string();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{s}",
        s.len()
    )
}

// ---------- 管理后台 ----------

fn admin_login(body: &[u8]) -> (u16, Value) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({ "error": "bad json" })),
    };
    let user = v.get("username").and_then(|x| x.as_str()).unwrap_or("");
    let pass = v.get("password").and_then(|x| x.as_str()).unwrap_or("");
    if user == ADMIN_USER && pass == ADMIN_PASS {
        (200, json!({ "token": admin_token() }))
    } else {
        (401, json!({ "error": "账号或密码错误" }))
    }
}

fn admin_teams(store: &Arc<Store>) -> (u16, Value) {
    match store.list_all_teams() {
        Ok(teams) => {
            let list: Vec<Value> = teams
                .iter()
                .map(|t| {
                    json!({
                        "short_code": t.short_code,
                        "room_id": t.room_id_hex,
                        "created_at": t.created_at,
                        "last_active": t.last_active,
                        "member_count": t.members.len(),
                        "members": t.members.iter().map(|m| json!({
                            "member_hex": m.member_hex,
                            "last_seen": m.last_seen,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            (200, json!({ "teams": list }))
        }
        Err(_) => (500, json!({ "error": "internal" })),
    }
}

fn admin_delete(body: &[u8], store: &Arc<Store>, server: &Arc<RelayServer>) -> (u16, Value) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({ "error": "bad json" })),
    };
    let short = match v.get("short_code").and_then(|x| x.as_str()) {
        Some(s) => s.to_string(),
        None => return (400, json!({ "error": "缺少 short_code" })),
    };
    // 先查 room_id 用于清理内存登记表
    let room_id = store
        .list_all_teams()
        .ok()
        .and_then(|list| list.into_iter().find(|t| t.short_code == short))
        .map(|t| t.room_id_hex);
    match store.dissolve_by_short(&short) {
        Ok(true) => {
            if let Some(room) = room_id {
                if let Ok(mut a) = server.registry().activity.lock() {
                    a.remove(&room);
                }
                if let Ok(mut m) = server.registry().members.lock() {
                    m.remove(&room);
                }
            }
            (200, json!({ "ok": true }))
        }
        Ok(false) => (404, json!({ "error": "队伍码不存在" })),
        Err(_) => (500, json!({ "error": "internal" })),
    }
}

/// 管理后台的单页 HTML（登录 + 队伍列表 + 删除）。
const ADMIN_PAGE: &str = include_str!("admin.html");

// ---------- 更新包托管（国内直连的自有更新源） ----------
//
// GitHub 直连在国内时通时断，第三方镜像寿命不可控。服务器在国内可达的 IP 上，
// 顺便把更新清单和安装包也放这里：客户端的 updater 把这里排在 endpoints 第一位，
// 连不上再回落 GitHub 和镜像。安装包本身有 minisign 端到端签名（公钥编译进客户端），
// 明文 HTTP 传输被篡改也过不了校验，所以不走 TLS 也安全。

/// 更新文件目录：数据库同目录下的 `updates/`。
pub fn updates_dir() -> std::path::PathBuf {
    let db = std::env::var("DATA_DIR").unwrap_or_else(|_| "ohmygang.db".to_string());
    std::path::Path::new(&db)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| "updates".into(), |p| p.join("updates"))
}

/// 文件名白名单：字母数字加 .-_，不超过 80 个字符，不以点开头（防路径穿越和怪文件）。
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && !name.contains("..")
}

/// POST /api/admin/update?token=...&name=... ：上传一个更新文件，body 是原始字节。
fn upload_update(path: &str, body: &[u8]) -> (u16, Value) {
    let expect = std::env::var("UPLOAD_TOKEN").unwrap_or_default();
    upload_update_in(path, body, &expect, &updates_dir())
}

fn upload_update_in(path: &str, body: &[u8], expect: &str, dir: &std::path::Path) -> (u16, Value) {
    if expect.is_empty() {
        return (404, json!({ "error": "not found" }));
    }
    let q = path.split('?').nth(1).unwrap_or("");
    let mut token = "";
    let mut name = "";
    for p in q.split('&') {
        if let Some(t) = p.strip_prefix("token=") {
            token = t;
        }
        if let Some(n) = p.strip_prefix("name=") {
            name = n;
        }
    }
    if token != expect {
        return (403, json!({ "error": "token 不对" }));
    }
    if !safe_name(name) {
        return (400, json!({ "error": "文件名不合法" }));
    }
    if body.is_empty() {
        return (400, json!({ "error": "文件内容为空" }));
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        return (500, json!({ "error": format!("建目录失败: {e}") }));
    }
    if let Err(e) = std::fs::write(dir.join(name), body) {
        return (500, json!({ "error": format!("写文件失败: {e}") }));
    }
    eprintln!("更新文件已上传: {name} ({} 字节)", body.len());
    (200, json!({ "ok": true, "name": name, "size": body.len() }))
}

/// GET /update/<name> 的完整 HTTP 响应（二进制），由网络层直接分流到这里。
pub async fn serve_update_file(path: &str) -> Vec<u8> {
    serve_update_bytes(path, &updates_dir()).await
}

async fn serve_update_bytes(path: &str, dir: &std::path::Path) -> Vec<u8> {
    let name = path.trim_start_matches("/update/");
    let not_found = |msg: &str| {
        let s = format!("{{\"error\":\"{msg}\"}}");
        format!(
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{s}",
            s.len()
        )
        .into_bytes()
    };
    if !safe_name(name) {
        return not_found("文件名不合法");
    }
    let file = match tokio::fs::File::open(dir.join(name)).await {
        Ok(f) => f,
        Err(_) => return not_found("not found"),
    };
    let meta = match file.metadata().await {
        Ok(m) => m,
        Err(_) => return not_found("not found"),
    };
    let mut buf = Vec::with_capacity(meta.len() as usize + 512);
    use tokio::io::AsyncReadExt;
    let mut file = file;
    if file.read_to_end(&mut buf).await.is_err() {
        return not_found("读取失败");
    }
    let ctype = match name.rsplit('.').next().unwrap_or("") {
        "json" => "application/json",
        "exe" => "application/octet-stream",
        "dmg" => "application/octet-stream",
        "gz" => "application/gzip",
        "sig" => "text/plain",
        _ => "application/octet-stream",
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        buf.len()
    );
    let mut resp = head.into_bytes();
    resp.extend_from_slice(&buf);
    resp
}

#[cfg(test)]
mod update_tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("ohmygang-update-{tag}-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn upload_needs_token_and_safe_name() {
        let dir = tmpdir("auth");
        // 没配 token：接口整体关闭
        let (s, _) = upload_update_in(
            "/api/admin/update?token=anything&name=latest.json",
            b"{}",
            "",
            &dir,
        );
        assert_eq!(s, 404);
        // token 不对
        let (s, _) = upload_update_in(
            "/api/admin/update?token=wrong&name=latest.json",
            b"{}",
            "secret",
            &dir,
        );
        assert_eq!(s, 403);
        // 路径穿越
        for bad in ["../db", "a/b", ".hidden", "x..y", ""] {
            let (s, _) = upload_update_in(
                &format!("/api/admin/update?token=secret&name={bad}"),
                b"{}",
                "secret",
                &dir,
            );
            assert_eq!(s, 400, "文件名 {bad:?} 要拒绝");
        }
        // 正常上传
        let (s, v) = upload_update_in(
            "/api/admin/update?token=secret&name=latest.json",
            b"{\"version\":\"0.2.0\"}",
            "secret",
            &dir,
        );
        assert_eq!(s, 200);
        assert_eq!(v["name"], "latest.json");
        assert_eq!(
            std::fs::read(dir.join("latest.json")).unwrap(),
            b"{\"version\":\"0.2.0\"}"
        );
    }

    #[tokio::test]
    async fn uploaded_file_downloads_back() {
        let dir = tmpdir("roundtrip");
        let payload = b"binary installer bytes";
        upload_update_in(
            "/api/admin/update?token=t&name=lolgang_0.2.0_x64-setup.exe",
            payload,
            "t",
            &dir,
        );
        let resp = serve_update_bytes("/update/lolgang_0.2.0_x64-setup.exe", &dir).await;
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200 OK"));
        assert!(text.contains("Content-Type: application/octet-stream"));
        assert!(text.contains("Connection: close"));
        let body = &resp[resp.len() - payload.len()..];
        assert_eq!(body, payload);

        // 不存在的文件 404；怪名字也 404
        assert!(
            String::from_utf8_lossy(&serve_update_bytes("/update/nope.exe", &dir).await)
                .starts_with("HTTP/1.1 404")
        );
        assert!(
            String::from_utf8_lossy(&serve_update_bytes("/update/../secret", &dir).await)
                .starts_with("HTTP/1.1 404")
        );
    }
}
