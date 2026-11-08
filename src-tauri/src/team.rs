//! 队伍的界面接口：控制面板的「队伍」页和个人资料调这里；节点交出来的通知也在这里分发。
//!
//! 实现流程：
//! 1. 命令都是异步的：在节点任务里执行（`NodeHandle::call`），完了返回整份 `TeamView`，控制面板直接换上。
//!    退出队伍时先发一帧 Bye 再写 Leave，队友马上就能看到人走了。
//! 2. 节点的通知（`on_node_event`）在节点任务里调用，不能等节点：
//!    队友的信号、表情、文字交给透明窗口，导入表情的传输帧交给 `emotes::net`，投票交给 `panel`；在线状态记进 `TeamState`；队伍有变化（进队、退出、改名、资料、解散）就
//!    另开一个异步任务把队员名字交给透明窗口、通知控制面板刷新。
//! 3. 本人的在线状态（`refresh_presence`）：信号关着是「暂停」，5 分钟没碰键鼠是「离开」，否则「在线」。
//!    变了才交给节点，节点自己按周期重复发。
//! 4. 队友的在线状态 15 秒（中继 3 个周期）没更新就当作没有。投票人是本人加上在线状态新鲜、没关投票的队友。
//! 5. 进了队伍或不在队伍里了（`refresh_names` 查到的状态变了），就让手势重新拿配置：文字快捷键和面板键只在队伍里拦。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::api::AppState;
use crate::app_core::{AppError, Core, MemberView, RoomStatus, RoomSummary};
use crate::crypto::{MemberId, RoomId};
use crate::emotes::{self, net::NetMsg};
use crate::identity::IdentityView;
use crate::node::{NodeEvent, Outbox, now_ms};
use crate::overlay::Msg;
use crate::panel::PanelMsg;
use crate::protocol::body::{Avatar, image_mime};
use crate::protocol::live::{LiveBody, OFF_VOTES, Status};
use crate::sync::path::PathKind;
use crate::transport::relay::RelayState;
use crate::votes::VoteKind;

/// 多久没碰键鼠算离开电脑。
const AWAY_AFTER_MS: u64 = 5 * 60 * 1000;
/// 队友的在线状态多久没更新就不算了。
const PRESENCE_STALE: Duration = Duration::from_secs(15);

/// 队友最近一次的在线状态。
struct Seen {
    status: Status,
    platform: String,
    /// 关掉了的功能（`OFF_TEXT`、`OFF_VOTES`）。
    off: u8,
    at: Instant,
}

#[derive(Default)]
pub struct TeamState {
    presence: Mutex<HashMap<MemberId, Seen>>,
    /// 最近一次交给节点的本人在线状态。
    mine: Mutex<Option<LiveBody>>,
    /// 在一个已加入的队伍里。
    pub active: AtomicBool,
}

/// 投票人：本人加上在线、没关投票的队友。
pub fn voters(app: &AppHandle, me: MemberId) -> Vec<MemberId> {
    let mut out = vec![me];
    if let Some(state) = app.try_state::<AppState>() {
        let presence = state.team.presence.lock().expect("在线状态锁没有中毒");
        out.extend(
            presence
                .iter()
                .filter(|(m, s)| {
                    **m != me && s.at.elapsed() <= PRESENCE_STALE && s.off & OFF_VOTES == 0
                })
                .map(|(m, _)| *m),
        );
    }
    out
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn base64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn unbase64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|_| "图片数据格式不对".to_string())
}

/// 给界面的头像：内置的给下标，图片给能直接放进 <img> 的 data URL。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AvatarDto {
    Preset { icon: u16, color: u8 },
    Image { data_url: String },
}

fn avatar_dto(a: &Option<Avatar>) -> Option<AvatarDto> {
    match a.as_ref()? {
        Avatar::Preset { icon, color } => Some(AvatarDto::Preset {
            icon: *icon,
            color: *color,
        }),
        Avatar::Image(b) => Some(AvatarDto::Image {
            data_url: format!("data:{};base64,{}", image_mime(b)?, base64(b)),
        }),
    }
}

/// 界面传来的头像：内置的给下标，图片给 Base64。
#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AvatarInput {
    Preset { icon: u16, color: u8 },
    Image { data: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeDto {
    pub member_id: String,
    pub nickname: String,
    pub fingerprint: String,
    pub avatar: Option<AvatarDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamDto {
    pub title: Option<String>,
    /// joining、active。
    pub status: &'static str,
    pub is_owner: bool,
    pub code: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDto {
    pub member_id: String,
    pub nickname: Option<String>,
    pub fingerprint: String,
    pub is_owner: bool,
    pub is_me: bool,
    pub avatar: Option<AvatarDto>,
    /// lan、relay、offline；自己是 null。
    pub path: Option<&'static str>,
    /// online、paused、away；没收到在线状态时是 null。
    pub status: Option<&'static str>,
    pub platform: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamView {
    pub me: MeDto,
    pub team: Option<TeamDto>,
    pub members: Vec<MemberDto>,
    /// 局域网端口被占，只能经中继联系。
    pub lan_degraded: bool,
    /// 连着几个中继、一共几个；关了互联网时都是 0。
    pub relays_ok: usize,
    pub relays_total: usize,
}

fn me_dto(v: &IdentityView) -> MeDto {
    MeDto {
        member_id: hex(&v.member_id),
        nickname: v.nickname.clone(),
        fingerprint: v.fingerprint.clone(),
        avatar: avatar_dto(&v.avatar),
    }
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Online => "online",
        Status::Paused => "paused",
        Status::Away => "away",
    }
}

/// 在节点任务里取的那一半：身份、当前队伍、队伍码、队员。
type CoreSnapshot = (IdentityView, Option<(RoomSummary, String)>, Vec<MemberView>);

fn snapshot(core: &mut Core, _: &mut Outbox) -> Result<CoreSnapshot, AppError> {
    let me = core.identity().clone();
    let Some(team) = core.current_team()? else {
        return Ok((me, None, Vec::new()));
    };
    let code = core.room_code(&team.room_id)?;
    let members = core
        .list_members(&team.room_id)?
        .into_iter()
        .filter(|m| m.in_room)
        .collect();
    Ok((me, Some((team, code)), members))
}

/// 拼出整份队伍页。
pub async fn view(app: &AppHandle) -> Result<TeamView, String> {
    let state = app.state::<AppState>();
    let node = state.node.clone();
    let (me, team, members) = node
        .call(snapshot)
        .await
        .ok_or("网络模块停了")?
        .map_err(|e| e.to_string())?;
    let room = team.as_ref().map(|(t, _)| t.room_id);
    let (paths, lan, relays) = node
        .inspect(move |n| {
            (
                room.map(|r| n.member_paths(&r)).unwrap_or_default(),
                n.lan_status(),
                n.relay_status(),
            )
        })
        .await
        .ok_or("网络模块停了")?;
    let presence = state.team.presence.lock().expect("在线状态锁没有中毒");
    let members = members
        .into_iter()
        .map(|m| {
            let is_me = m.member_id == me.member_id;
            let path = paths
                .iter()
                .find(|(id, _)| *id == m.member_id)
                .map(|(_, p)| *p);
            let seen = presence
                .get(&m.member_id)
                .filter(|s| s.at.elapsed() < PRESENCE_STALE);
            MemberDto {
                member_id: hex(&m.member_id),
                nickname: m.nickname,
                fingerprint: m.fingerprint,
                is_owner: m.is_owner,
                is_me,
                avatar: avatar_dto(&m.avatar),
                path: (!is_me).then_some(match path {
                    Some(PathKind::Lan) => "lan",
                    Some(PathKind::Relay) => "relay",
                    _ => "offline",
                }),
                status: seen.map(|s| status_name(s.status)),
                platform: seen.map(|s| s.platform.clone()),
            }
        })
        .collect();
    let relays = relays.unwrap_or_default();
    Ok(TeamView {
        me: me_dto(&me),
        team: team.map(|(t, code)| TeamDto {
            title: t.title,
            status: if t.status == RoomStatus::Joining {
                "joining"
            } else {
                "active"
            },
            is_owner: t.is_owner,
            code,
        }),
        members,
        lan_degraded: lan.degraded,
        relays_ok: relays.iter().filter(|r| r.state == RelayState::Ok).count(),
        relays_total: relays.len(),
    })
}

/// 在节点任务里改队伍，再返回整份队伍页。
async fn act<F>(app: &AppHandle, f: F) -> Result<TeamView, String>
where
    F: FnOnce(&mut Core, &mut Outbox) -> Result<(), AppError> + Send + 'static,
{
    let node = app.state::<AppState>().node.clone();
    node.call(f)
        .await
        .ok_or("网络模块停了")?
        .map_err(|e| e.to_string())?;
    refresh_names(app);
    refresh_presence(app);
    view(app).await
}

fn current(core: &Core) -> Result<RoomId, AppError> {
    core.current_team()?
        .map(|t| t.room_id)
        .ok_or(AppError::RoomNotFound)
}

#[tauri::command]
pub async fn team_view(app: AppHandle) -> Result<TeamView, String> {
    view(&app).await
}

#[tauri::command]
pub async fn team_create(app: AppHandle, title: String) -> Result<TeamView, String> {
    act(&app, move |core, out| {
        let c = core.create_room(&title, now_ms())?;
        out.broadcast(c.summary.room_id, c.events);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn team_join(app: AppHandle, code: String) -> Result<TeamView, String> {
    act(&app, move |core, _| {
        core.join_room(&code, now_ms()).map(|_| ())
    })
    .await
}

/// 正在加入时取消。
#[tauri::command]
pub async fn team_cancel(app: AppHandle) -> Result<TeamView, String> {
    act(&app, |core, _| {
        let room = current(core)?;
        core.cancel_join(&room)
    })
    .await
}

/// 退出：先发 Bye，再写 Leave。
#[tauri::command]
pub async fn team_leave(app: AppHandle) -> Result<TeamView, String> {
    let view = act(&app, |core, out| {
        let room = current(core)?;
        out.live(room, LiveBody::Bye);
        let written = core.leave_room(&room, now_ms())?;
        out.broadcast(room, written);
        Ok(())
    })
    .await;
    app.state::<AppState>()
        .team
        .presence
        .lock()
        .expect("在线状态锁没有中毒")
        .clear();
    view
}

#[tauri::command]
pub async fn team_dissolve(app: AppHandle) -> Result<TeamView, String> {
    act(&app, |core, out| {
        let room = current(core)?;
        let written = core.dissolve_room(&room, now_ms())?;
        out.broadcast(room, written);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn team_rename(app: AppHandle, title: String) -> Result<TeamView, String> {
    act(&app, move |core, out| {
        let room = current(core)?;
        let written = core.rename_room(&room, &title, now_ms())?;
        out.broadcast(room, written);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn profile_set(
    app: AppHandle,
    nickname: String,
    avatar: Option<AvatarInput>,
) -> Result<TeamView, String> {
    let avatar = match avatar {
        None => None,
        Some(AvatarInput::Preset { icon, color }) => Some(Avatar::Preset { icon, color }),
        Some(AvatarInput::Image { data }) => Some(Avatar::Image(unbase64(&data)?)),
    };
    act(&app, move |core, out| {
        let written = core.set_profile(nickname.trim(), avatar, now_ms())?;
        if let Some(room) = core.current_team()?.map(|t| t.room_id) {
            out.broadcast(room, written);
        }
        Ok(())
    })
    .await
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
    let body = LiveBody::Presence {
        status,
        laning: false,
        version: app.package_info().version.to_string(),
        platform: platform_name().into(),
        emotes: emotes::join_announced(&announced),
        off,
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

/// 把队员名字交给透明窗口（信号下面的标签用），并通知控制面板刷新。不在节点任务里等。
pub fn refresh_names(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let node = state.node.clone();
        let names = node
            .call(
                |core, _| -> Result<(bool, Vec<(MemberId, String)>), AppError> {
                    let Some(team) = core.current_team()? else {
                        return Ok((false, Vec::new()));
                    };
                    let active = team.status == RoomStatus::Active;
                    Ok((
                        active,
                        core.list_members(&team.room_id)?
                            .into_iter()
                            .map(|m| {
                                (
                                    m.member_id,
                                    m.nickname
                                        .filter(|n| !n.is_empty())
                                        .unwrap_or(m.fingerprint),
                                )
                            })
                            .collect(),
                    ))
                },
            )
            .await;
        if let Some(Ok((active, names))) = names {
            state.panel.send(PanelMsg::Names(names.clone()));
            state.overlays.send(Msg::Names(names));
            if state.team.active.swap(active, Ordering::Relaxed) != active {
                state.panel.send(PanelMsg::InTeam(active));
                crate::api::push_gesture_config(&app);
            }
        }
        let _ = app.emit_to("main", "team://changed", ());
    });
}

/// 节点交出来的通知。在节点任务里调用：只做不等节点的事。
pub fn on_node_event(app: &AppHandle, e: NodeEvent) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match e {
        NodeEvent::Live { from, body, .. } => match body {
            LiveBody::Ping { display, x, y, id } => state.overlays.send(Msg::RemotePing {
                from,
                display,
                x,
                y,
                id,
            }),
            LiveBody::Emote {
                display,
                x,
                y,
                emote,
                fx,
            } => {
                if emotes::is_ref_shape(&emote) && fx < emotes::FX_VARIANTS {
                    state.overlays.send(Msg::RemoteEmote {
                        from,
                        display,
                        x,
                        y,
                        emote,
                        fx,
                    });
                }
            }
            LiveBody::Text {
                display,
                x,
                y,
                text,
            } => {
                let on = state.settings.lock().expect("设置锁没有中毒").team_text;
                let text = crate::team_text::sanitize(&text);
                if on && !text.is_empty() {
                    state.overlays.send(Msg::Text {
                        from: Some(from),
                        display,
                        x,
                        y,
                        text,
                    });
                }
            }
            LiveBody::Vote { id, kind, target } => {
                if let Some(kind) = VoteKind::from_code(kind) {
                    state.panel.send(PanelMsg::RemoteVote {
                        from,
                        id,
                        kind,
                        target,
                    });
                }
            }
            LiveBody::Ballot { id, yes } => {
                state.panel.send(PanelMsg::RemoteBallot { from, id, yes })
            }
            LiveBody::Want { to, hash, need } => state.emote_net.send(NetMsg::Want {
                from,
                to,
                hash: hex(&hash),
                need,
            }),
            LiveBody::Chunk { hash, i, n, bytes } => state.emote_net.send(NetMsg::Chunk {
                from,
                hash: hex(&hash),
                i,
                n,
                bytes,
            }),
            LiveBody::Presence {
                status,
                platform,
                emotes: announced,
                off,
                ..
            } => {
                state.emote_net.send(NetMsg::Announced {
                    from,
                    hashes: emotes::split_announced(&announced),
                });
                let mut presence = state.team.presence.lock().expect("在线状态锁没有中毒");
                let changed = presence
                    .get(&from)
                    .is_none_or(|s| s.status != status || s.at.elapsed() > PRESENCE_STALE);
                presence.insert(
                    from,
                    Seen {
                        status,
                        platform,
                        off,
                        at: Instant::now(),
                    },
                );
                drop(presence);
                // 只在状态变了时刷新控制面板，免得每 2 秒刷一次
                if changed {
                    let _ = app.emit_to("main", "team://changed", ());
                }
            }
            LiveBody::Bye => {
                state
                    .team
                    .presence
                    .lock()
                    .expect("在线状态锁没有中毒")
                    .remove(&from);
                state.emote_net.send(NetMsg::Gone(from));
                state.panel.send(PanelMsg::Gone(from));
                let _ = app.emit_to("main", "team://changed", ());
            }
        },
        NodeEvent::ProtocolMismatch { remote_proto, .. } => {
            let _ = app.emit_to("main", "team://mismatch", remote_proto);
        }
        NodeEvent::Core(_) | NodeEvent::RoomStatusChanged { .. } => refresh_names(app),
        NodeEvent::Delivered { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_become_dtos() {
        assert_eq!(
            avatar_dto(&Some(Avatar::Preset { icon: 3, color: 4 })),
            Some(AvatarDto::Preset { icon: 3, color: 4 })
        );
        let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
        let Some(AvatarDto::Image { data_url }) = avatar_dto(&Some(Avatar::Image(png))) else {
            panic!("应是图片");
        };
        assert!(data_url.starts_with("data:image/png;base64,"));
        assert_eq!(avatar_dto(&Some(Avatar::Image(b"<svg>".to_vec()))), None);
        let json = serde_json::to_value(AvatarDto::Preset { icon: 1, color: 2 }).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "preset", "icon": 1, "color": 2 })
        );
        let input: AvatarInput = serde_json::from_str(r#"{"kind":"image","data":"AAEC"}"#).unwrap();
        assert!(matches!(input, AvatarInput::Image { data } if data == "AAEC"));
    }
}
