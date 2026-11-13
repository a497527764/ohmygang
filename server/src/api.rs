//! 短码 HTTP API：建队（分配 6 位短码 + 铸造 GT2 长码）、进队（短码换长码）、心跳（上报人数）、查码。
//!
//! 短码是「别名」：服务端把它映射到 room_secret + owner_tag，铸成 GT2 长码交给客户端；
//! 客户端用 GT2 长码像往常一样派生 room_key / room_id。服务端据此真正拥有并回收队伍码。

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::relay::RelayServer;
use crate::store::Store;

/// 分接口的限流桶：按 (接口, IP) 固定窗口计数。
///
/// 每个接口单独计数——防止有人拿「刷 join」当跳板把服务端打满，
/// 也不会因为某个接口正常用得多就把另一个接口的额度吃掉。
static RATE: OnceLock<Mutex<std::collections::HashMap<(String, &'static str), Bucket>>> =
    OnceLock::new();

struct Bucket {
    start: i64,
    count: u32,
    window: i64,
}

/// 一个接口的限流额度。
struct Limit {
    bucket: &'static str,
    max: u32,
    window: i64,
}

/// 建队：同一 IP 一小时最多 30 次（设备维度的额度之外再加一层，防单点刷）。
const L_CREATE: Limit = Limit {
    bucket: "create",
    max: 30,
    window: 3600,
};
/// 进队：换码。正常用户偶尔用一次，一分钟 60 次足够。
const L_JOIN: Limit = Limit {
    bucket: "join",
    max: 60,
    window: 60,
};
/// 心跳：客户端每 10 分钟一次，一分钟 120 次是极宽松的上限。
const L_HEARTBEAT: Limit = Limit {
    bucket: "heartbeat",
    max: 120,
    window: 60,
};
/// 在线人数查询。
const L_STATS: Limit = Limit {
    bucket: "stats",
    max: 60,
    window: 60,
};
/// 改队名。
const L_RENAME: Limit = Limit {
    bucket: "rename",
    max: 20,
    window: 60,
};
/// 按 room_id 反查短码。
const L_CODE: Limit = Limit {
    bucket: "code",
    max: 60,
    window: 60,
};
/// 后台登录：密码只有 6 位，爆破成本极低，必须卡死。一分钟 10 次足够手输。
const L_ADMIN_LOGIN: Limit = Limit {
    bucket: "admin_login",
    max: 10,
    window: 60,
};
/// 后台接口本身（列表 / 删除 / 批量）。
const L_ADMIN: Limit = Limit {
    bucket: "admin",
    max: 120,
    window: 60,
};

/// 建队额度。两层，都能用环境变量调，不用改代码：
/// - 设备层：同一台设备（按 `member_id` 认设备）一小时 10 支、24 小时 20 支。
///   认设备不认 IP——换网络、换梯子出口，额度还是你的。
/// - 全局层：整个服务器合计一小时 200 支、24 小时 1000 支。
///   设备层挡不住「每次换一个 member_id」的脚本，全局层才是防攻击的底线。
fn create_quota() -> (u32, u32, u32, u32) {
    let env = |k: &str, d: u32| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    (
        env("DEVICE_PER_HOUR", 10),
        env("DEVICE_PER_DAY", 20),
        env("CREATE_PER_HOUR", 200),
        env("CREATE_PER_DAY", 1000),
    )
}

/// 短码有效期（秒）：默认 30 天。
/// 队伍里有人在用（最近 24 小时内 2 个以上成员报过心跳）就自动续期；
/// 一个人建了队再也没人用，到期就把短码收回来，别让码白占着。
pub fn code_ttl() -> i64 {
    std::env::var("CODE_TTL_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(30)
        * 86400
}

/// 短码复用冷却期（秒）：回收下来的码先在池子里放 7 天再发出去。
/// 免得「刚过期的码马上被别人捡走」，拿着旧码的队友会稀里糊涂进到新队伍里。
const CODE_REUSE_COOLDOWN: i64 = 7 * 86400;

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
            if let Some(t) = p.strip_prefix("token=")
                && t == admin_token()
            {
                return true;
            }
        }
    }
    // 再从 body 取（POST 时前端用 JSON 传 token）
    if let Ok(v) = serde_json::from_slice::<Value>(body)
        && v.get("token").and_then(|x| x.as_str()) == Some(admin_token())
    {
        return true;
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

/// 生成一个当前没被占用的短码。
///
/// 撞码的概率极低（32 个字符选 6 位），但万一撞了还得能继续，所以重试有上限——
/// 原来是无条件死循环，数据库一读失败就卡住整个建队接口。
fn gen_unique_short(store: &Arc<Store>) -> String {
    for _ in 0..64 {
        let c = gen_short();
        if !store.team_exists(&c).unwrap_or(true) {
            return c;
        }
    }
    // 64 次都撞上（或数据库读不出来）：随机换掉其中一位再来一轮。
    for _ in 0..64 {
        let mut chars: Vec<char> = gen_short().chars().collect();
        let pos = (rand::random::<u32>() % 6) as usize;
        chars[pos] =
            SHORT_ALPHABET[(rand::random::<u32>() % SHORT_ALPHABET.len() as u32) as usize] as char;
        let c: String = chars.into_iter().collect();
        if !store.team_exists(&c).unwrap_or(true) {
            return c;
        }
    }
    gen_short()
}

/// 记一次请求并判断是否超额。超额时返回 false，且不计数（放行额度内的部分）。
fn rate_ok(peer: &str, lim: &Limit) -> bool {
    let now = now_secs();
    let map = RATE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut map = map.lock().expect("限流锁没有中毒");
    // 桶会随 IP 数量增长，顺手清掉过期桶，别让它无限长。
    if map.len() > 4096 {
        map.retain(|_, b| now - b.start < b.window);
    }
    let e = map.entry((peer.to_string(), lim.bucket)).or_insert(Bucket {
        start: now,
        count: 0,
        window: lim.window,
    });
    if now - e.start >= e.window {
        *e = Bucket {
            start: now,
            count: 0,
            window: lim.window,
        };
    }
    if e.count >= lim.max {
        false
    } else {
        e.count += 1;
        true
    }
}

/// 超额时的统一响应。带一个 retry_after，前端可以照着提示用户。
fn too_many(lim: &Limit) -> (u16, Value) {
    (
        429,
        json!({
            "error": "请求过于频繁，请稍后再试",
            "retry_after_seconds": lim.window,
        }),
    )
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
#[allow(dead_code)] // 只有测试在用，正式构建里没人调它
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
    // 分接口的 IP 限流：在进业务逻辑之前统一拦掉，省得每个 handler 各写一遍。
    let limit = match (method, route) {
        ("POST", "/api/team/create") => Some(&L_CREATE),
        ("POST", "/api/team/join") => Some(&L_JOIN),
        ("POST", "/api/team/heartbeat") => Some(&L_HEARTBEAT),
        ("POST", "/api/team/stats") => Some(&L_STATS),
        ("POST", "/api/team/rename") => Some(&L_RENAME),
        ("GET", "/api/team/code") => Some(&L_CODE),
        ("POST", "/api/admin/login") => Some(&L_ADMIN_LOGIN),
        ("GET", "/api/admin/teams")
        | ("POST", "/api/admin/teams/delete")
        | ("POST", "/api/admin/teams/delete_batch") => Some(&L_ADMIN),
        _ => None,
    };
    match limit {
        Some(lim) if !rate_ok(peer, lim) => return too_many(lim),
        _ => {}
    }
    match (method, route) {
        ("GET", "/") | ("GET", "/health") => (200, json!({ "ok": true })),
        ("POST", "/api/team/create") => create(body, store, server, peer).await,
        ("POST", "/api/team/join") => join(body, store).await,
        ("POST", "/api/team/heartbeat") => heartbeat(body, store, server).await,
        ("POST", "/api/team/stats") => stats(body, store).await,
        ("POST", "/api/team/rename") => rename(body, store).await,
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
        // 批量删除：一次删多支队伍
        ("POST", "/api/admin/teams/delete_batch") => {
            if check_admin(path, body) {
                admin_delete_batch(body, store, server)
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
    // 建队额度。两层：设备（member_id）和整个服务器。
    // 校验通过才记账，参数都错就没必要占额度。
    let (dev_hour, dev_day, glob_hour, glob_day) = create_quota();
    match store.reserve_create(
        now_secs(),
        peer,
        &member_hex,
        crate::store::CreateLimits {
            device_hour: dev_hour,
            device_day: dev_day,
            global_hour: glob_hour,
            global_day: glob_day,
        },
    ) {
        Ok(crate::store::Quota::Ok) => {}
        Ok(crate::store::Quota::Device) => {
            return (
                429,
                json!({
                    "error": format!(
                        "这台设备的建队额度已用尽（1 小时 {dev_hour} 支 / 24 小时 {dev_day} 支），请稍后再试"
                    ),
                    "retry_after_seconds": 3600,
                }),
            );
        }
        Ok(crate::store::Quota::Global) => {
            return (
                429,
                json!({
                    "error": format!(
                        "服务器建队额度已用尽（1 小时 {glob_hour} 支 / 24 小时 {glob_day} 支），请稍后再试"
                    ),
                    "retry_after_seconds": 3600,
                }),
            );
        }
        Err(_) => return (500, json!({ "error": "internal" })),
    }
    // 铸造房间密钥材料
    let secret: [u8; 16] = rand::random();
    let owner_tag = crate::protocol::owner_tag_of(&member_id);
    let (_key, room_id) = crate::protocol::derive_room_keys(&secret, &owner_tag);
    let room_id_hex = crate::protocol::hex(&room_id);
    let now = now_secs();
    // 短码：优先从回收池里取（省得白浪费码空间），池子里没有就现生成一个。
    let short = match store.take_free_code(now, CODE_REUSE_COOLDOWN) {
        Ok(Some(c)) if !store.team_exists(&c).unwrap_or(true) => c,
        _ => gen_unique_short(store),
    };
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().chars().take(64).collect::<String>())
        .filter(|s| !s.is_empty());
    if store
        .create_team(
            &short,
            &secret,
            &owner_tag,
            &room_id_hex,
            now,
            &member_hex,
            title.as_deref(),
            now + code_ttl(),
        )
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
        Ok(Some((code_str, room_id_hex, title))) => (
            200,
            json!({ "code": code_str, "room_id": room_id_hex, "title": title }),
        ),
        Ok(None) => (404, json!({ "error": "队伍码不存在或已过期" })),
        Err(_) => (500, json!({ "error": "internal" })),
    }
}

/// 房主改队名：存到服务器（客户端进队时从服务器拿名字，中继重启也不丢）。
async fn rename(body: &[u8], store: &Arc<Store>) -> (u16, Value) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({ "error": "bad json" })),
    };
    let Some(room_id) = v.get("room_id").and_then(|x| x.as_str()) else {
        return (400, json!({ "error": "缺少 room_id" }));
    };
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .chars()
        .take(64)
        .collect::<String>();
    if title.is_empty() {
        return (400, json!({ "error": "队名为空" }));
    }
    match store.set_team_title(room_id, &title) {
        Ok(_) => (200, json!({ "ok": true })),
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
/// 顺带把每个房间的短码状态带回去：短码会被回收（到期没人用），
/// 客户端要知道「这个码已经失效了」，不然用户还拿着旧码去分享。
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
    let mut codes = serde_json::Map::new();
    for r in rooms {
        let Some(room) = r.as_str() else { continue };
        if parse_hex(room, 16).is_none() {
            continue;
        }
        if let Ok(n) = store.room_online(room, since) {
            out.insert(room.to_string(), json!(n));
        }
        // null = 这个房间的短码已经没了（回收或过期）；有值 = 还能用。
        match store.short_by_room(room) {
            Ok(Some(short)) => {
                codes.insert(room.to_string(), json!(short));
            }
            Ok(None) => {
                codes.insert(room.to_string(), Value::Null);
            }
            Err(_) => {}
        }
    }
    (
        200,
        json!({ "stats": Value::Object(out), "codes": Value::Object(codes) }),
    )
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
                        "code": t.code,
                        "title": t.title,
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

/// 回收一支队伍并清掉内存登记表（管理后台删除用）。
fn dissolve_short(store: &Arc<Store>, server: &Arc<RelayServer>, short: &str) -> bool {
    // 先查 room_id 用于清理内存登记表
    let room_id = store
        .list_all_teams()
        .ok()
        .and_then(|list| list.into_iter().find(|t| t.short_code == short))
        .map(|t| t.room_id_hex);
    match store.dissolve_by_short(short) {
        Ok(true) => {
            if let Some(room) = room_id {
                if let Ok(mut a) = server.registry().activity.lock() {
                    a.remove(&room);
                }
                if let Ok(mut m) = server.registry().members.lock() {
                    m.remove(&room);
                }
            }
            true
        }
        _ => false,
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
    if dissolve_short(store, server, &short) {
        (200, json!({ "ok": true, "deleted": 1 }))
    } else {
        (404, json!({ "error": "队伍码不存在" }))
    }
}

/// 批量删除：一次处理多支队伍。已不存在的不算失败，单独列出来给前端提示。
fn admin_delete_batch(body: &[u8], store: &Arc<Store>, server: &Arc<RelayServer>) -> (u16, Value) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({ "error": "bad json" })),
    };
    let codes = match v.get("short_codes").and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return (400, json!({ "error": "缺少 short_codes" })),
    };
    if codes.is_empty() {
        return (400, json!({ "error": "没有选中任何队伍" }));
    }
    if codes.len() > 500 {
        return (400, json!({ "error": "一次最多删 500 支" }));
    }
    let mut deleted: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for c in codes {
        let Some(code) = c.as_str() else { continue };
        let code = code.trim().to_uppercase();
        if code.is_empty() {
            continue;
        }
        if dissolve_short(store, server, &code) {
            deleted.push(code);
        } else {
            missing.push(code);
        }
    }
    (
        200,
        json!({
            "ok": true,
            "deleted": deleted.len(),
            "deleted_codes": deleted,
            "missing": missing,
        }),
    )
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
