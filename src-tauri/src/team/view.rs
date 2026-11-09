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
use crate::transport::relay::RelayState;

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
