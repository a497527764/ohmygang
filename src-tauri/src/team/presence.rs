//! 在线状态：本人的（`refresh_presence`）和队伍页上没有事件、却会变的东西（`watch`）。见 `team` 文件头第 3、6 条。

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use super::{AWAY_AFTER_MS, PRESENCE_STALE};
use crate::api::{AppState, CLOUD_CONNECTING, CLOUD_DOWN, CLOUD_OK, cloud_state_name};
use crate::crypto::MemberId;
use crate::emotes;
use crate::protocol::live::{LiveBody, Status};
use crate::transport::relay::RelayState;

/// 启动后的宽限期：这期间连不上云端只显示「连接中」，8 秒后才说「未连接」。
const CONNECT_GRACE: Duration = Duration::from_secs(8);

/// 队伍页上会变、却没有事件的东西，变了才通知控制面板（文件头第 6 条）。在线状态线程里调用，可以等节点。
pub fn watch(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let room = *state.team.room.lock().expect("队伍锁没有中毒");
    let node = state.node.clone();
    let (paths, relay_ok, relay_total, rtt) =
        tauri::async_runtime::block_on(node.inspect(move |n| {
            let relays = n.relay_status().unwrap_or_default();
            // Ok 是正常连着；Cooling 是被中继限流（连接还在，只是暂时不发了）——都算连着。
            let connected = relays
                .iter()
                .filter(|r| matches!(r.state, RelayState::Ok | RelayState::Cooling))
                .count();
            (
                room.map(|r| n.member_paths(&r)).unwrap_or_default(),
                connected,
                relays.len(),
                relays
                    .iter()
                    .filter_map(|r| r.echo_avg_ms)
                    .min()
                    .unwrap_or(0),
            )
        }))
        .unwrap_or((Vec::new(), 0, 0, 0));

    // 云端连接状态（三态）：
    // - 一个中继都没配（关了互联网）→ 不算断，用户自己关的不说成掉线。
    // - 连上了 → ok。
    // - 没连上：刚启动的头 8 秒一律「连接中」；之后要连续两次（10 秒）探不到才算「未连接」，
    //   一次没探到先按原来的算——重连的那几秒不该让界面来回跳。
    let cloud = if relay_ok > 0 || relay_total == 0 {
        // 连上了，或者一个中继都没配（关了互联网）：都不算断，用户自己关的不说成掉线。
        state.cloud_miss.store(0, Ordering::Relaxed);
        CLOUD_OK
    } else {
        let miss = state
            .cloud_miss
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if state.started_at.elapsed() < CONNECT_GRACE {
            CLOUD_CONNECTING
        } else if miss >= 2 {
            CLOUD_DOWN
        } else {
            CLOUD_OK
        }
    };
    state.cloud_rtt_ms.store(rtt, Ordering::Relaxed);
    if state.cloud_state.swap(cloud, Ordering::Relaxed) != cloud {
        let _ = app.emit_to("main", "status://changed", cloud_state_name(cloud));
    }
    let mut fresh: Vec<(MemberId, bool)> = state
        .team
        .presence
        .lock()
        .expect("在线状态锁没有中毒")
        .iter()
        .map(|(m, s)| (*m, s.at.elapsed() < PRESENCE_STALE))
        .collect();
    fresh.sort();
    let now = format!("{relay_ok:?}{paths:?}{fresh:?}");
    let mut watched = state.team.watched.lock().expect("队伍锁没有中毒");
    if watched.as_deref() != Some(now.as_str()) {
        *watched = Some(now);
        let _ = app.emit_to("main", "team://changed", ());
    }
}

/// 本人的在线状态，变了才交给节点。
pub fn refresh_presence(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let status = if !state.enabled.load(Ordering::Relaxed) {
        Status::Paused
    } else if crate::input::idle_ms().is_some_and(|ms| ms > AWAY_AFTER_MS) {
        Status::Away
    } else {
        Status::Online
    };
    let (announced, off) = {
        let s = state.settings.lock().expect("设置锁没有中毒");
        (s.announced_emotes(), s.features_off())
    };
    // 大头像的哈希报在在线状态里：本机有这张图（文件在）才报，队友没有就会来要。
    // 只读缓存（启动和改资料时更新）：这个函数在异步命令里也会被调，不能在这里等节点。
    let avatar = state
        .team
        .avatar_hash
        .lock()
        .expect("头像哈希锁没有中毒")
        .clone()
        .filter(|h| state.emotes.has_avatar(h));
    let body = LiveBody::Presence {
        status,
        laning: false,
        version: app.package_info().version.to_string(),
        platform: platform_name().into(),
        emotes: emotes::join_announced(&announced),
        off,
        avatar,
    };
    let mut mine = state.team.mine.lock().expect("在线状态锁没有中毒");
    if mine.as_ref() != Some(&body) {
        *mine = Some(body.clone());
        state.node.set_presence(body);
    }
}

fn platform_name() -> &'static str {
    if cfg!(windows) {
        "win"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    }
}
