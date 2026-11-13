//! 云端服务客户端（部署方法见 server/README.md）：短码建队、进队、队伍心跳。
//!
//! 实现流程：
//! 1. 地址常量在本文件顶部：`CLOUD_API`（HTTP 接口）和 `CLOUD_RELAY`（WebSocket 中继）。
//!    服务部署在自有服务器上，走服务器 IP + 转发端口直连（无需公开域名/实名认证）。
//!    换服务器时把两处地址改掉即可。局域网已去掉，云端是唯一路径。
//! 2. 接口都是 POST JSON：`/api/team/create`（拿 6 位短码和铸造好的 GT2 长码）、
//!    `/api/team/join`（短码换长码）、`/api/team/heartbeat`（保活，服务端靠它判断队伍活跃）。
//! 3. 传输是手写的最小 HTTP 客户端（tokio）：支持 http://（明文）和 https://（TLS）两种地址，
//!    发一个 HTTP/1.1 请求、读到连接关闭，支持 Content-Length 和 chunked 两种响应体。15 秒超时。
//! 4. 心跳：启动后每 10 分钟给所有活跃队伍报一次（room_id + member_id）；
//!    建队/进队成功后也会立刻报一次。服务端超过 5 天没活动（或连续 5 天不足 2 人）会回收队伍码。

use std::time::Duration;

use serde_json::{Value, json};

use crate::crypto::hex;
use crate::node::NodeHandle;

/// 自有云端服务的 HTTP 接口（Zeabur 端口转发：服务器 IP + 转发端口，明文直连）。
pub const CLOUD_API: &str = "http://14.103.234.253:31818";
/// 自有云端服务的 WebSocket 中继（同服务器、同端口，ws:// 明文）。
pub const CLOUD_RELAY: &str = "ws://14.103.234.253:31818";

const TIMEOUT: Duration = Duration::from_secs(15);
/// 心跳周期：服务端 5 天没活动才回收，10 分钟一次足够。
const HEARTBEAT_EVERY: Duration = Duration::from_secs(600);

/// 建队成功：6 位短码（给朋友的那个）、铸造好的 GT2 长码、room_id（十六进制）。
pub struct TeamCreated {
    pub short_code: String,
    pub code: String,
}

/// 进队成功：换回的 GT2 长码、room_id（十六进制）、队名（服务器上存的）。
pub struct TeamJoined {
    pub code: String,
    pub title: Option<String>,
}

/// 建队：服务器分配 6 位短码并铸造 GT2 长码（owner_tag 由本人公钥生成）。队名一起存到服务器。
pub async fn team_create(member_id_hex: &str, title: &str) -> Result<TeamCreated, String> {
    let v = post_json(
        "/api/team/create",
        &json!({ "member_id": member_id_hex, "title": title }),
    )
    .await?;
    Ok(TeamCreated {
        short_code: str_field(&v, "short_code")?,
        code: str_field(&v, "code")?,
    })
}

/// 进队：6 位短码换回 GT2 长码。短码不存在或已回收时返回错误文案。
pub async fn team_join(short_code: &str) -> Result<TeamJoined, String> {
    let v = post_json("/api/team/join", &json!({ "code": short_code })).await?;
    Ok(TeamJoined {
        code: str_field(&v, "code")?,
        title: v
            .get("title")
            .and_then(|x| x.as_str())
            .map(String::from)
            .filter(|s| !s.is_empty()),
    })
}

/// 改队名：房主改名后同步给服务器（别人进队时从服务器拿名字）。
pub async fn team_rename(room_id_hex: &str, title: &str) -> Result<(), String> {
    post_json(
        "/api/team/rename",
        &json!({ "room_id": room_id_hex, "title": title }),
    )
    .await?;
    Ok(())
}

/// 队伍心跳（保活）。队伍已被回收时返回错误。
pub async fn team_heartbeat(room_id_hex: &str, member_id_hex: &str) -> Result<(), String> {
    post_json(
        "/api/team/heartbeat",
        &json!({ "room_id": room_id_hex, "member_id": member_id_hex }),
    )
    .await?;
    Ok(())
}

/// 一批队伍现在各有多少人在线（15 分钟内报过心跳）。给主页「加入过的队伍」显示 在线/总数。
pub async fn team_stats(
    room_ids: &[String],
) -> Result<std::collections::HashMap<String, usize>, String> {
    let v = post_json("/api/team/stats", &json!({ "room_ids": room_ids })).await?;
    let Some(map) = v.get("stats").and_then(|s| s.as_object()) else {
        return Err("云端响应格式不对".to_string());
    };
    Ok(map
        .into_iter()
        .filter_map(|(k, x)| x.as_u64().map(|n| (k.clone(), n as usize)))
        .collect())
}

/// 给所有活跃队伍报一次心跳。
pub async fn heartbeat_all(node: &NodeHandle) {
    let list = node
        .call(|core, _| {
            let me = hex(&core.identity().member_id);
            core.sync_rooms()
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, s)| *s == crate::app_core::RoomStatus::Active)
                .map(|(r, _)| (hex(&r), me.clone()))
                .collect::<Vec<_>>()
        })
        .await;
    let Some(list) = list else { return };
    for (room, me) in list {
        let _ = team_heartbeat(&room, &me).await;
    }
}

/// 启动后台心跳任务：每 10 分钟给所有活跃队伍报一次。
pub fn spawn_heartbeat(node: NodeHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(HEARTBEAT_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            heartbeat_all(&node).await;
        }
    });
}

fn str_field(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(String::from)
        .ok_or_else(|| "云端响应格式不对".to_string())
}

/// POST 一个 JSON 到云端接口，返回响应里的 JSON。
async fn post_json(path: &str, body: &Value) -> Result<Value, String> {
    // 地址形如 http://host:port 或 https://host（端口可省略）。
    let base = CLOUD_API.trim_end_matches('/');
    let (use_tls, rest) = match base.split_once("://") {
        Some(("https", r)) => (true, r),
        Some(("http", r)) => (false, r),
        _ => return Err("云端地址格式不对".to_string()),
    };
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| "云端地址格式不对".to_string())?,
        ),
        None => (
            rest.split('/').next().unwrap_or("").to_string(),
            if use_tls { 443 } else { 80 },
        ),
    };
    let body_text = body.to_string();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         Accept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_text}",
        body_text.len()
    );
    let exchange = async {
        let tcp = tokio::net::TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| format!("连不上云端服务：{e}"))?;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut raw = Vec::new();
        if use_tls {
            let connector =
                native_tls::TlsConnector::new().map_err(|e| format!("安全连接初始化失败：{e}"))?;
            let connector = tokio_native_tls::TlsConnector::from(connector);
            let mut stream = connector
                .connect(&host, tcp)
                .await
                .map_err(|e| format!("安全连接失败：{e}"))?;
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| format!("发送请求失败：{e}"))?;
            stream
                .read_to_end(&mut raw)
                .await
                .map_err(|e| format!("读取响应失败：{e}"))?;
        } else {
            let mut stream = tcp;
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| format!("发送请求失败：{e}"))?;
            stream
                .read_to_end(&mut raw)
                .await
                .map_err(|e| format!("读取响应失败：{e}"))?;
        }
        Ok::<Vec<u8>, String>(raw)
    };
    let raw = tokio::time::timeout(TIMEOUT, exchange)
        .await
        .map_err(|_| "云端服务响应超时".to_string())??;
    parse_response(&raw)
}

/// 解析 HTTP 响应：状态码 + JSON 体（兼容 Content-Length 和 chunked）。
fn parse_response(raw: &[u8]) -> Result<Value, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "云端响应格式不对".to_string())?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "云端响应格式不对".to_string())?;
    let plain = &raw[split + 4..];
    let dechunked;
    let body: &[u8] = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        dechunked = dechunk(plain);
        &dechunked
    } else {
        plain
    };
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        Ok(v)
    } else {
        Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("云端服务返回 {status}")))
    }
}

/// 去掉 HTTP chunked 分块封装。
fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut rest = body;
    let mut out = Vec::new();
    while let Some(pos) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(
            std::str::from_utf8(&rest[..pos])
                .unwrap_or("")
                .trim()
                .split(';')
                .next()
                .unwrap_or(""),
            16,
        )
        .unwrap_or(0);
        if size == 0 || rest.len() < pos + 2 + size {
            break;
        }
        rest = &rest[pos + 2..];
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size..];
        if rest.starts_with(b"\r\n") {
            rest = &rest[2..];
        }
    }
    out
}
