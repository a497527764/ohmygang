//! 拼队伍页：身份、队伍、队员（头像、走哪条路、在线状态），给控制面板和队伍面板用。见 `team` 文件头第 1、4 条。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::PRESENCE_STALE;
use crate::api::AppState;
use crate::app_core::{AppError, Core, MemberView, RoomStatus, RoomSummary};
use crate::crypto::hex;
use crate::identity::IdentityView;
use crate::node::Outbox;
use crate::protocol::body::{Avatar, image_mime};
use crate::protocol::live::Status;
use crate::sync::path::PathKind;
use crate::transport::relay::{RelayHealth, RelayState};

fn base64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

pub(super) fn unbase64(s: &str) -> Result<Vec<u8>, String> {
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
    /// relay、offline；自己是 null。
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
    /// 连着几个中继、一共几个；关了互联网时都是 0。
    pub relays_ok: usize,
    pub relays_total: usize,
    /// 每个中继的连接状态和回显统计（3.2.0），队伍页「中继」那一行的说明里显示。
    pub relays: Vec<RelayDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentTeamDto {
    pub code: String,
    /// 云端分配的 6 位短码（显示用）；老队伍没有。
    pub short_code: Option<String>,
    pub title: Option<String>,
    pub is_owner: bool,
    /// joining、active、left。
    pub status: &'static str,
    /// 推导出还在队里的人数（不是实时在线数）。
    pub members: usize,
    /// 云端统计的现在在线人数（15 分钟内报过心跳）；问不到云端时是 None，前端显示「—」。
    pub online: Option<usize>,
}

/// 主页历史队伍列表 + 当前队伍的在线人数（云端查的）。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentTeams {
    /// 加入过的队伍（不含当前队伍）。
    pub rooms: Vec<RecentTeamDto>,
    /// 当前队伍现在几个人在线；问不到云端时是 None，前端显示「—」。
    pub current_online: Option<usize>,
}

/// 主页历史队伍列表：明文队伍码只在内存和这一层出现，落盘的仍是密文。
pub(super) fn recent_dto(r: &crate::app_core::RecentRoom) -> RecentTeamDto {
    RecentTeamDto {
        code: r.code.clone(),
        short_code: r.short_code.clone(),
        title: r.title.clone(),
        is_owner: r.is_owner,
        status: match r.status {
            crate::app_core::RoomStatus::Joining => "joining",
            crate::app_core::RoomStatus::Active => "active",
            _ => "left",
        },
        members: r.members,
        // 在线数由命令层问完云端再填（recent_dto 拿不到网络）
        online: None,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayDto {
    /// 地址去掉 `wss://`。
    pub host: String,
    /// ok、connecting、cooling、retired。
    pub state: &'static str,
    /// 最近 50 个事件的回显成功率；还没发过时是 null。
    pub echo_pct: Option<u8>,
    /// 平均回显往返时间（毫秒）。
    pub echo_ms: Option<u64>,
    /// 现在的包发给它（只发回显最好的 3 个）。
    pub sending: bool,
}

fn relay_dto(r: &RelayHealth) -> RelayDto {
    RelayDto {
        host: r.url.strip_prefix("wss://").unwrap_or(&r.url).to_string(),
        state: match r.state {
            RelayState::Ok => "ok",
            RelayState::Connecting => "connecting",
            RelayState::Cooling => "cooling",
            RelayState::Retired => "retired",
        },
        echo_pct: r.echo_pct,
        echo_ms: r.echo_avg_ms,
        sending: r.sending,
    }
}

/// 大头像（200×200）：哈希对应的文件在本机（自己的，或者收到队友的）才有 data URL。
fn big_avatar(state: &AppState, hash: Option<&str>) -> Option<AvatarDto> {
    hash.and_then(|h| state.emotes.avatar_data_url(h))
        .map(|data_url| AvatarDto::Image { data_url })
}

fn me_dto(v: &IdentityView, big: Option<AvatarDto>) -> MeDto {
    MeDto {
        member_id: hex(&v.member_id),
        nickname: v.nickname.clone(),
        fingerprint: v.fingerprint.clone(),
        avatar: big.or_else(|| avatar_dto(&v.avatar)),
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
    let code = core.display_code(&team.room_id)?;
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
    let (paths, relays) = node
        .inspect(move |n| {
            (
                room.map(|r| n.member_paths(&r)).unwrap_or_default(),
                n.relay_status(),
            )
        })
        .await
        .ok_or("网络模块停了")?;
    let presence = state.team.presence.lock().expect("在线状态锁没有中毒");
    let me_big = big_avatar(&state, me.avatar_hash.as_deref());
    let mut members: Vec<MemberDto> = members
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
                avatar: big_avatar(&state, m.avatar_hash.as_deref())
                    .or_else(|| avatar_dto(&m.avatar)),
                path: (!is_me).then_some(match path {
                    Some(PathKind::Relay) => "relay",
                    _ => "offline",
                }),
                status: seen.map(|s| status_name(s.status)),
                platform: seen.map(|s| s.platform.clone()),
            }
        })
        .collect();
    drop(presence);
    // 加入中：事件还没同步过来，成员列表是空的。把自己摆进去，
    // 队伍面板显示和进了队一样（我的那一行），别让用户以为卡在申请里没成功。
    if team
        .as_ref()
        .is_some_and(|(t, _)| t.status == RoomStatus::Joining)
        && members.is_empty()
    {
        members.push(MemberDto {
            member_id: hex(&me.member_id),
            nickname: (!me.nickname.is_empty()).then(|| me.nickname.clone()),
            fingerprint: me.fingerprint.clone(),
            is_owner: false,
            is_me: true,
            avatar: me_big
                .clone()
                .or_else(|| avatar_dto(&me.avatar)),
            path: None,
            status: None,
            platform: None,
        });
    }
    let relays = relays.unwrap_or_default();
    Ok(TeamView {
        me: me_dto(&me, me_big),
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
        relays_ok: relays.iter().filter(|r| r.state == RelayState::Ok).count(),
        relays_total: relays.len(),
        relays: relays.iter().map(relay_dto).collect(),
    })
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
