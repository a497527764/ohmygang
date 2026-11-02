//! 界面桥（接口文档第 12 节）：Tauri 命令和事件。
//!
//! 实现流程：
//! 1. 每个命令通过 `NodeHandle::call` 到节点任务里执行，业务逻辑全在 Rust 侧；新写的事件交给节点广播。
//! 2. 返回给界面的都是 DTO：ID 用十六进制字符串，字段名用驼峰（JS 习惯），状态用小写英文单词。
//! 3. 错误转成中文提示字符串，界面直接显示。
//! 4. 节点的通知转成 `EventDto`，以 Tauri 事件 `hs://event` 发给界面，`type` 字段区分种类。
//!
//! 这些是应用自己定义的命令，Tauri 2 默认允许本应用窗口调用，不需要改 capabilities，
//! 也没有把文件系统或网络能力开放给前端。

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::app_core::{
    Core, CoreEvent, LocalSettings, MemberView, RoomStatus, RoomSummary, RuleParams, SettingsPatch,
};
use crate::chat::{NoticeKind, TimelineCursor, TimelineItem};
use crate::error::AppResult;
use crate::identity::IdentityView;
use crate::node::{NodeEvent, NodeHandle, Outbox, now_ms};
use crate::storage::events::Delivery;
use crate::sync::path::PathKind;
use crate::transport::relay::{RelayHealth, RelayState};

/// 建房时用的默认规则（5 秒内 3 条，冷却 30 秒）。改规则的入口在 M3。
const DEFAULT_RULE: RuleParams = RuleParams {
    window_ms: 5000,
    threshold: 3,
    cooldown_ms: 30_000,
};

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex<const N: usize>(s: &str) -> Result<[u8; N], String> {
    if s.len() != N * 2 {
        return Err("ID 长度不对".into());
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "ID 格式不对".to_string())?;
    }
    Ok(out)
}

fn status_name(s: RoomStatus) -> &'static str {
    match s {
        RoomStatus::Joining => "joining",
        RoomStatus::Active => "active",
        RoomStatus::Left => "left",
        RoomStatus::Dissolved => "dissolved",
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDto {
    pub member_id: String,
    pub nickname: String,
    pub fingerprint: String,
}

impl From<&IdentityView> for IdentityDto {
    fn from(v: &IdentityView) -> Self {
        IdentityDto {
            member_id: hex(&v.member_id),
            nickname: v.nickname.clone(),
            fingerprint: v.fingerprint.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomDto {
    pub room_id: String,
    pub title: Option<String>,
    pub status: &'static str,
    pub is_owner: bool,
    pub unread: u32,
    pub last_active: u64,
}

impl From<&RoomSummary> for RoomDto {
    fn from(r: &RoomSummary) -> Self {
        RoomDto {
            room_id: hex(&r.room_id),
            title: r.title.clone(),
            status: status_name(r.status),
            is_owner: r.is_owner,
            unread: r.unread,
            last_active: r.last_active,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CreatedDto {
    pub room: RoomDto,
    pub code: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDto {
    pub member_id: String,
    pub fingerprint: String,
    pub nickname: Option<String>,
    pub stacks: u32,
    pub is_owner: bool,
    pub in_room: bool,
    /// 现在经哪条路径能联系上：lan、relay、offline（自己也是 offline，界面不显示）。
    pub path: &'static str,
}

impl MemberDto {
    fn new(m: &MemberView, path: PathKind) -> Self {
        MemberDto {
            member_id: hex(&m.member_id),
            fingerprint: m.fingerprint.clone(),
            nickname: m.nickname.clone(),
            stacks: m.stacks,
            is_owner: m.is_owner,
            in_room: m.in_room,
            path: match path {
                PathKind::Lan => "lan",
                PathKind::Relay => "relay",
                PathKind::Unknown => "offline",
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayHealthDto {
    pub url: String,
    /// connecting、ok、cooling、retired。
    pub state: &'static str,
    pub last_echo_ms: Option<u64>,
}

impl From<&RelayHealth> for RelayHealthDto {
    fn from(h: &RelayHealth) -> Self {
        RelayHealthDto {
            url: h.url.clone(),
            state: match h.state {
                RelayState::Connecting => "connecting",
                RelayState::Ok => "ok",
                RelayState::Cooling => "cooling",
                RelayState::Retired => "retired",
            },
            last_echo_ms: h.last_echo_ms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub internet_allowed: bool,
    pub relays: Vec<String>,
}

impl From<&LocalSettings> for SettingsDto {
    fn from(s: &LocalSettings) -> Self {
        SettingsDto {
            internet_allowed: s.internet_allowed,
            relays: s.relays.clone(),
        }
    }
}

/// 要修改的设置项，没给的不改。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatchDto {
    pub internet_allowed: Option<bool>,
    pub relays: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorDto {
    pub display_ts: u64,
    pub event_id: String,
}

/// 聊天窗口的一条：聊天或系统提示。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    pub kind: &'static str,
    pub event_id: String,
    pub author_id: String,
    pub ts_ms: u64,
    pub display_ts: u64,
    pub text: Option<String>,
    pub delivery: Option<&'static str>,
    pub notice: Option<&'static str>,
    pub title: Option<String>,
}

impl MessageDto {
    pub fn cursor(&self) -> CursorDto {
        CursorDto {
            display_ts: self.display_ts,
            event_id: self.event_id.clone(),
        }
    }
}

impl From<&TimelineItem> for MessageDto {
    fn from(item: &TimelineItem) -> Self {
        match item {
            TimelineItem::Chat(m) => MessageDto {
                kind: "chat",
                event_id: hex(&m.event_id),
                author_id: hex(&m.author_id),
                ts_ms: m.ts_ms,
                display_ts: m.display_ts,
                text: Some(m.text.clone()),
                delivery: m.delivery.map(|d| match d {
                    Delivery::Pending => "pending",
                    Delivery::Sent => "sent",
                }),
                notice: None,
                title: None,
            },
            TimelineItem::Notice(n) => MessageDto {
                kind: "notice",
                event_id: hex(&n.event_id),
                author_id: hex(&n.author_id),
                ts_ms: n.ts_ms,
                display_ts: n.display_ts,
                text: None,
                delivery: None,
                notice: Some(match n.kind {
                    NoticeKind::Join => "join",
                    NoticeKind::Leave => "leave",
                    NoticeKind::Rename => "rename",
                    NoticeKind::ConfigNotice => "config",
                    NoticeKind::Dissolve => "dissolve",
                }),
                title: n.title.clone(),
            },
        }
    }
}

/// 发给界面的通知，`type` 区分种类。
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EventDto {
    Timeline {
        room_id: String,
        message: MessageDto,
        is_live: bool,
    },
    Stacks {
        room_id: String,
        changes: Vec<(String, u32)>,
    },
    Live {
        room_id: String,
        member_id: String,
        new_stacks: u32,
    },
    Room {
        room_id: String,
    },
    Fork {
        room_id: String,
        member_id: String,
    },
    Status {
        room_id: String,
        status: &'static str,
    },
    Mismatch {
        room_id: String,
        remote_proto: u8,
    },
}

impl From<NodeEvent> for EventDto {
    fn from(e: NodeEvent) -> Self {
        match e {
            NodeEvent::Core(CoreEvent::TimelineItemAdded {
                room_id,
                item,
                is_live,
            }) => EventDto::Timeline {
                room_id: hex(&room_id),
                message: MessageDto::from(&item),
                is_live,
            },
            NodeEvent::Core(CoreEvent::StacksChanged { room_id, changes }) => EventDto::Stacks {
                room_id: hex(&room_id),
                changes: changes.iter().map(|(m, s)| (hex(m), *s)).collect(),
            },
            NodeEvent::Core(CoreEvent::HeartsteelLive {
                room_id,
                member_id,
                new_stacks,
                ..
            }) => EventDto::Live {
                room_id: hex(&room_id),
                member_id: hex(&member_id),
                new_stacks,
            },
            NodeEvent::Core(CoreEvent::RoomUpdated { room_id }) => EventDto::Room {
                room_id: hex(&room_id),
            },
            NodeEvent::Core(CoreEvent::ForkDetected { room_id, member_id }) => EventDto::Fork {
                room_id: hex(&room_id),
                member_id: hex(&member_id),
            },
            NodeEvent::RoomStatusChanged { room_id, status } => EventDto::Status {
                room_id: hex(&room_id),
                status: status_name(status),
            },
            NodeEvent::ProtocolMismatch {
                room_id,
                remote_proto,
            } => EventDto::Mismatch {
                room_id: hex(&room_id),
                remote_proto,
            },
        }
    }
}

/// 在节点里执行一个可能失败的操作，错误转成中文提示。
async fn on_node<R, F>(node: &NodeHandle, f: F) -> Result<R, String>
where
    F: FnOnce(&mut Core, &mut Outbox) -> AppResult<R> + Send + 'static,
    R: Send + 'static,
{
    match node.call(f).await {
        Some(r) => r.map_err(|e| e.to_string()),
        None => Err("后台服务已停止，请重启应用".into()),
    }
}

#[tauri::command]
pub async fn identity_get(node: State<'_, NodeHandle>) -> Result<IdentityDto, String> {
    on_node(&node, |core, _| Ok(IdentityDto::from(core.identity()))).await
}

#[tauri::command]
pub async fn identity_set_nickname(
    node: State<'_, NodeHandle>,
    nickname: String,
) -> Result<IdentityDto, String> {
    on_node(&node, move |core, out| {
        for e in core.set_nickname(nickname.trim(), now_ms())? {
            out.broadcast(e.event.room_id, vec![e]);
        }
        Ok(IdentityDto::from(core.identity()))
    })
    .await
}

#[tauri::command]
pub async fn rooms_list(node: State<'_, NodeHandle>) -> Result<Vec<RoomDto>, String> {
    on_node(&node, |core, _| {
        Ok(core.list_rooms()?.iter().map(RoomDto::from).collect())
    })
    .await
}

#[tauri::command]
pub async fn rooms_create(
    node: State<'_, NodeHandle>,
    title: String,
) -> Result<CreatedDto, String> {
    on_node(&node, move |core, out| {
        let c = core.create_room(&title, DEFAULT_RULE, now_ms())?;
        out.broadcast(c.summary.room_id, c.events);
        Ok(CreatedDto {
            room: RoomDto::from(&c.summary),
            code: c.code,
        })
    })
    .await
}

#[tauri::command]
pub async fn rooms_join(node: State<'_, NodeHandle>, code: String) -> Result<RoomDto, String> {
    on_node(&node, move |core, _| {
        Ok(RoomDto::from(&core.join_room(&code, now_ms())?))
    })
    .await
}

#[tauri::command]
pub async fn rooms_get_code(
    node: State<'_, NodeHandle>,
    room_id: String,
) -> Result<String, String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| core.room_code(&room)).await
}

#[tauri::command]
pub async fn rooms_members(
    node: State<'_, NodeHandle>,
    room_id: String,
) -> Result<Vec<MemberDto>, String> {
    let room = unhex::<16>(&room_id)?;
    let members = on_node(&node, move |core, _| core.list_members(&room)).await?;
    let paths = node
        .inspect(move |n| n.member_paths(&room))
        .await
        .unwrap_or_default();
    Ok(members
        .iter()
        .map(|m| {
            let path = paths
                .iter()
                .find(|(id, _)| *id == m.member_id)
                .map_or(PathKind::Unknown, |(_, p)| *p);
            MemberDto::new(m, path)
        })
        .collect())
}

/// 各中继的连接状态；关闭「允许互联网」时返回 null。
#[tauri::command]
pub async fn relay_status(
    node: State<'_, NodeHandle>,
) -> Result<Option<Vec<RelayHealthDto>>, String> {
    node.inspect(|n| {
        n.relay_status()
            .map(|list| list.iter().map(RelayHealthDto::from).collect())
    })
    .await
    .ok_or_else(|| "后台服务已停止，请重启应用".to_string())
}

#[tauri::command]
pub async fn rooms_mark_read(node: State<'_, NodeHandle>, room_id: String) -> Result<(), String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| core.mark_read(&room)).await
}

#[tauri::command]
pub async fn chat_send_text(
    node: State<'_, NodeHandle>,
    room_id: String,
    text: String,
) -> Result<MessageDto, String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, out| {
        let sent = core.send_text(&room, &text, now_ms())?;
        out.broadcast(room, sent.events);
        out.notes
            .extend(sent.notes.into_iter().map(NodeEvent::Core));
        Ok(MessageDto::from(&TimelineItem::Chat(sent.message)))
    })
    .await
}

#[tauri::command]
pub async fn chat_list(
    node: State<'_, NodeHandle>,
    room_id: String,
    before: Option<CursorDto>,
    limit: u32,
) -> Result<Vec<MessageDto>, String> {
    let room = unhex::<16>(&room_id)?;
    let before = match before {
        Some(c) => Some(TimelineCursor {
            display_ts: c.display_ts,
            event_id: unhex::<32>(&c.event_id)?,
        }),
        None => None,
    };
    on_node(&node, move |core, _| {
        Ok(core
            .list_messages(&room, before, limit)?
            .iter()
            .map(MessageDto::from)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn settings_get(node: State<'_, NodeHandle>) -> Result<SettingsDto, String> {
    on_node(&node, |core, _| Ok(SettingsDto::from(&core.settings()?))).await
}

#[tauri::command]
pub async fn settings_update(
    node: State<'_, NodeHandle>,
    patch: SettingsPatchDto,
) -> Result<SettingsDto, String> {
    on_node(&node, move |core, out| {
        let s = core.update_settings(SettingsPatch {
            internet_allowed: patch.internet_allowed,
            relays: patch.relays,
        })?;
        out.settings_changed = true;
        Ok(SettingsDto::from(&s))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatMessage, NoticeKind, RoomNotice, TimelineItem};
    use crate::storage::events::Delivery;

    #[test]
    fn member_path_and_relay_state_names() {
        let m = MemberView {
            member_id: [1; 32],
            fingerprint: "abcdef".into(),
            nickname: None,
            stacks: 2,
            is_owner: false,
            in_room: true,
        };
        let names =
            [PathKind::Lan, PathKind::Relay, PathKind::Unknown].map(|p| MemberDto::new(&m, p).path);
        assert_eq!(names, ["lan", "relay", "offline"]);

        let h = RelayHealthDto::from(&RelayHealth {
            url: "wss://a.example".into(),
            state: RelayState::Cooling,
            last_echo_ms: Some(120),
        });
        assert_eq!(h.state, "cooling");
        let json = serde_json::to_value(&h).unwrap();
        assert_eq!(json["lastEchoMs"], 120);
    }

    #[test]
    fn hex_roundtrip_and_errors() {
        let id = [0xabu8; 16];
        let s = hex(&id);
        assert_eq!(s.len(), 32);
        assert_eq!(unhex::<16>(&s), Ok(id));
        assert!(unhex::<16>("abc").is_err());
        assert!(unhex::<16>(&"zz".repeat(16)).is_err());
    }

    #[test]
    fn timeline_items_become_message_dtos() {
        let chat = TimelineItem::Chat(ChatMessage {
            event_id: [1; 32],
            author_id: [2; 32],
            ts_ms: 10,
            display_ts: 11,
            text: "你好".into(),
            delivery: Some(Delivery::Pending),
        });
        let dto = MessageDto::from(&chat);
        assert_eq!(
            (dto.kind, dto.text.as_deref(), dto.display_ts),
            ("chat", Some("你好"), 11)
        );
        assert_eq!(dto.delivery, Some("pending"));
        assert_eq!(
            dto.cursor(),
            CursorDto {
                display_ts: 11,
                event_id: hex(&[1; 32])
            }
        );

        let notice = TimelineItem::Notice(RoomNotice {
            event_id: [3; 32],
            kind: NoticeKind::Rename,
            author_id: [2; 32],
            ts_ms: 20,
            display_ts: 20,
            title: Some("新群名".into()),
        });
        let dto = MessageDto::from(&notice);
        assert_eq!(
            (dto.kind, dto.notice, dto.title.as_deref()),
            ("notice", Some("rename"), Some("新群名"))
        );
    }

    #[test]
    fn events_serialize_with_a_type_tag() {
        let e = EventDto::from(NodeEvent::RoomStatusChanged {
            room_id: [5; 16],
            status: RoomStatus::Active,
        });
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["type"], "status");
        assert_eq!(json["roomId"], hex(&[5; 16]));
        assert_eq!(json["status"], "active");
    }
}
