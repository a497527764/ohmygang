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

use tauri::Manager;

use crate::app_core::{
    BackupKind, Core, CoreEvent, Draft, LocalSettings, MemberView, RoomStatus, RoomSummary,
    RuleParams, SettingsPatch,
};
use crate::chat::{NoticeKind, TimelineCursor, TimelineItem};
use crate::crypto::MemberId;
use crate::error::AppResult;
use crate::identity::IdentityView;
use crate::node::{NodeEvent, NodeHandle, Outbox, now_ms};
use crate::protocol::body::{Avatar, image_mime};
use crate::settings::DEFAULT_RELAYS;
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

fn base64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn unbase64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|_| "数据格式不对".to_string())
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
pub struct IdentityDto {
    pub member_id: String,
    pub nickname: String,
    pub fingerprint: String,
    pub avatar: Option<AvatarDto>,
}

impl From<&IdentityView> for IdentityDto {
    fn from(v: &IdentityView) -> Self {
        IdentityDto {
            member_id: hex(&v.member_id),
            nickname: v.nickname.clone(),
            fingerprint: v.fingerprint.clone(),
            avatar: avatar_dto(&v.avatar),
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
    pub muted: bool,
    pub pinned: bool,
    /// 最后一条消息（「乙：……」）。
    pub preview: Option<String>,
    /// 未读里有人 @ 我。
    pub mentioned: bool,
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
            muted: r.muted,
            pinned: r.pinned,
            preview: r.preview.clone(),
            mentioned: r.mentioned,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntryDto {
    pub member_id: String,
    pub stacks: u32,
    /// 名次从 1 开始，同层同名次（1、1、3）。
    pub rank: u32,
}

/// 排行榜已按层数降序排好（同层按成员 ID），这里只算名次。
fn leaderboard_dto(board: &[(MemberId, u32)]) -> Vec<LeaderboardEntryDto> {
    let mut out: Vec<LeaderboardEntryDto> = Vec::with_capacity(board.len());
    for (i, (id, stacks)) in board.iter().enumerate() {
        let rank = match out.last() {
            Some(prev) if prev.stacks == *stacks => prev.rank,
            _ => i as u32 + 1,
        };
        out.push(LeaderboardEntryDto {
            member_id: hex(id),
            stacks: *stacks,
            rank,
        });
    }
    out
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
    pub avatar: Option<AvatarDto>,
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
            avatar: avatar_dto(&m.avatar),
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
    pub sound_enabled: bool,
    pub volume: u8,
    pub history_limit_lan: u32,
    pub history_limit_relay: u32,
    /// system、light 或 dark。
    pub theme: String,
    /// 内置默认中继，设置页「恢复默认」用。
    pub default_relays: Vec<String>,
}

impl From<&LocalSettings> for SettingsDto {
    fn from(s: &LocalSettings) -> Self {
        SettingsDto {
            internet_allowed: s.internet_allowed,
            relays: s.relays.clone(),
            sound_enabled: s.sound_enabled,
            volume: s.volume,
            history_limit_lan: s.history_limit_lan,
            history_limit_relay: s.history_limit_relay,
            theme: s.theme.clone(),
            default_relays: DEFAULT_RELAYS.map(String::from).to_vec(),
        }
    }
}

/// 要修改的设置项，没给的不改。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatchDto {
    pub internet_allowed: Option<bool>,
    pub relays: Option<Vec<String>>,
    pub sound_enabled: Option<bool>,
    pub volume: Option<u8>,
    pub history_limit_lan: Option<u32>,
    pub history_limit_relay: Option<u32>,
    pub theme: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorDto {
    pub display_ts: u64,
    pub event_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyDto {
    pub event_id: String,
    pub excerpt: String,
    /// 原消息不在本机时为 null。
    pub author_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDto {
    pub width: u16,
    pub height: u16,
    /// 各片都到齐了，可以用 chat_image 取图。
    pub ready: bool,
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
    /// 文字消息的文字；图片和系统提示为 null。
    pub text: Option<String>,
    pub delivery: Option<&'static str>,
    pub notice: Option<&'static str>,
    pub title: Option<String>,
    pub reply: Option<ReplyDto>,
    /// 被 @ 的人：公钥前 8 字节的十六进制（即成员 ID 的前 16 位）。
    pub mentions: Vec<String>,
    pub image: Option<ImageDto>,
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
                text: m.image.is_none().then(|| m.text.clone()),
                delivery: m.delivery.map(|d| match d {
                    Delivery::Pending => "pending",
                    Delivery::Sent => "sent",
                }),
                notice: None,
                title: None,
                reply: m.reply.as_ref().map(|r| ReplyDto {
                    event_id: hex(&r.event_id),
                    excerpt: r.excerpt.clone(),
                    author_id: r.author_id.map(|a| hex(&a)),
                }),
                mentions: m.mentions.iter().map(|s| hex(s)).collect(),
                image: m.image.map(|i| ImageDto {
                    width: i.width,
                    height: i.height,
                    ready: i.ready,
                }),
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
                reply: None,
                mentions: Vec::new(),
                image: None,
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
        message: Box<MessageDto>,
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
    /// 本人这些消息已被同伴确认持有。
    Delivered {
        room_id: String,
        event_ids: Vec<String>,
    },
    /// 这张图片的各片收齐了，可以取图。
    ImageReady {
        room_id: String,
        event_id: String,
    },
}

impl From<NodeEvent> for EventDto {
    fn from(e: NodeEvent) -> Self {
        match e {
            NodeEvent::Core(CoreEvent::TimelineItemAdded {
                room_id,
                item,
                is_live,
                ..
            }) => EventDto::Timeline {
                room_id: hex(&room_id),
                message: Box::new(MessageDto::from(&item)),
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
            NodeEvent::Core(CoreEvent::ImageReady { room_id, event_id }) => EventDto::ImageReady {
                room_id: hex(&room_id),
                event_id: hex(&event_id),
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
            NodeEvent::Delivered { room_id, event_ids } => EventDto::Delivered {
                room_id: hex(&room_id),
                event_ids: event_ids.iter().map(|id| hex(id)).collect(),
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

/// 改昵称和头像（avatar 为 null 表示用自动生成的头像）。
#[tauri::command]
pub async fn identity_set_profile(
    node: State<'_, NodeHandle>,
    nickname: String,
    avatar: Option<AvatarInput>,
) -> Result<IdentityDto, String> {
    let avatar = match avatar {
        None => None,
        Some(AvatarInput::Preset { icon, color }) => Some(Avatar::Preset { icon, color }),
        Some(AvatarInput::Image { data }) => Some(Avatar::Image(unbase64(&data)?)),
    };
    on_node(&node, move |core, out| {
        for e in core.set_profile(nickname.trim(), avatar, now_ms())? {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatusDto {
    pub lan_port: u16,
    /// 局域网端口被占，用的是随机端口，收不到别人的局域网广播。
    pub lan_degraded: bool,
}

#[tauri::command]
pub async fn network_status(node: State<'_, NodeHandle>) -> Result<NetworkStatusDto, String> {
    node.inspect(|n| {
        let s = n.lan_status();
        NetworkStatusDto {
            lan_port: s.port,
            lan_degraded: s.degraded,
        }
    })
    .await
    .ok_or_else(|| "后台服务已停止，请重启应用".to_string())
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

/// 房间的本机偏好（免打扰、置顶），没给的不改。
#[tauri::command]
pub async fn rooms_set_prefs(
    node: State<'_, NodeHandle>,
    room_id: String,
    muted: Option<bool>,
    pinned: Option<bool>,
) -> Result<RoomDto, String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| {
        Ok(RoomDto::from(&core.set_room_prefs(&room, muted, pinned)?))
    })
    .await
}

/// 心之钢排行榜：只列当前成员，层数降序。
#[tauri::command]
pub async fn heartsteel_leaderboard(
    node: State<'_, NodeHandle>,
    room_id: String,
) -> Result<Vec<LeaderboardEntryDto>, String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| {
        Ok(leaderboard_dto(&core.leaderboard(&room)?))
    })
    .await
}

/// 界面当前打开的房间（null 表示没打开）。这个房间同步得更勤，其余降频（架构 7.2）。
#[tauri::command]
pub async fn rooms_set_active(
    node: State<'_, NodeHandle>,
    room_id: Option<String>,
) -> Result<(), String> {
    let room = room_id.map(|r| unhex::<16>(&r)).transpose()?;
    node.set_active_room(room).await;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleDto {
    pub epoch: u32,
    pub window_ms: u32,
    pub threshold: u32,
    pub cooldown_ms: u32,
}

/// 房间当前的心之钢规则；还没同步到建房事件时为 null。
#[tauri::command]
pub async fn rooms_rule(
    node: State<'_, NodeHandle>,
    room_id: String,
) -> Result<Option<RuleDto>, String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| {
        Ok(core.room_rule(&room)?.map(|r| RuleDto {
            epoch: r.rule_epoch,
            window_ms: r.window_ms,
            threshold: r.threshold,
            cooldown_ms: r.cooldown_ms,
        }))
    })
    .await
}

/// 房主改群名。
#[tauri::command]
pub async fn rooms_rename(
    node: State<'_, NodeHandle>,
    room_id: String,
    title: String,
) -> Result<(), String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, out| {
        out.broadcast(room, core.rename_room(&room, &title, now_ms())?);
        Ok(())
    })
    .await
}

/// 房主改心之钢规则（窗口 1–60 秒、条数 2–10、冷却 1 秒–1 小时）。
#[tauri::command]
pub async fn rooms_set_rule(
    node: State<'_, NodeHandle>,
    room_id: String,
    window_ms: u32,
    threshold: u32,
    cooldown_ms: u32,
) -> Result<(), String> {
    let room = unhex::<16>(&room_id)?;
    let params = RuleParams {
        window_ms,
        threshold,
        cooldown_ms,
    };
    on_node(&node, move |core, out| {
        out.broadcast(room, core.set_room_rule(&room, params, now_ms())?);
        Ok(())
    })
    .await
}

/// 退出房间：聊天记录保留只读，层数保留。
#[tauri::command]
pub async fn rooms_leave(node: State<'_, NodeHandle>, room_id: String) -> Result<(), String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, out| {
        out.broadcast(room, core.leave_room(&room, now_ms())?);
        Ok(())
    })
    .await
}

/// 房主解散房间，不可撤销。
#[tauri::command]
pub async fn rooms_dissolve(node: State<'_, NodeHandle>, room_id: String) -> Result<(), String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, out| {
        out.broadcast(room, core.dissolve_room(&room, now_ms())?);
        Ok(())
    })
    .await
}

/// 取消正在加入的房间。
#[tauri::command]
pub async fn rooms_cancel_join(node: State<'_, NodeHandle>, room_id: String) -> Result<(), String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| core.cancel_join(&room)).await
}

#[tauri::command]
pub async fn rooms_mark_read(node: State<'_, NodeHandle>, room_id: String) -> Result<(), String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| core.mark_read(&room)).await
}

/// 发文字消息。`reply_to`：引用的消息 ID；`mentions`：被 @ 的成员 ID。
#[tauri::command]
pub async fn chat_send(
    node: State<'_, NodeHandle>,
    room_id: String,
    text: String,
    reply_to: Option<String>,
    mentions: Vec<String>,
) -> Result<MessageDto, String> {
    let room = unhex::<16>(&room_id)?;
    let reply_to = reply_to.map(|r| unhex::<32>(&r)).transpose()?;
    let mentions = mentions
        .iter()
        .map(|m| unhex::<32>(m))
        .collect::<Result<Vec<_>, _>>()?;
    on_node(&node, move |core, out| {
        let draft = Draft {
            text: &text,
            reply_to,
            mentions: &mentions,
        };
        let sent = core.send_chat(&room, &draft, now_ms())?;
        out.broadcast(room, sent.events);
        out.notes
            .extend(sent.notes.into_iter().map(NodeEvent::Core));
        Ok(MessageDto::from(&TimelineItem::Chat(sent.message)))
    })
    .await
}

/// 发图片。`data` 是界面压缩好的图片的 Base64。
#[tauri::command]
pub async fn chat_send_image(
    node: State<'_, NodeHandle>,
    room_id: String,
    data: String,
    width: u16,
    height: u16,
) -> Result<MessageDto, String> {
    let room = unhex::<16>(&room_id)?;
    let bytes = unbase64(&data)?;
    on_node(&node, move |core, out| {
        let sent = core.send_image(&room, &bytes, (width, height), now_ms())?;
        out.broadcast(room, sent.events);
        Ok(MessageDto::from(&TimelineItem::Chat(sent.message)))
    })
    .await
}

/// 取图片：收齐时返回 data URL，还没收齐返回 null。
#[tauri::command]
pub async fn chat_image(
    node: State<'_, NodeHandle>,
    room_id: String,
    event_id: String,
) -> Result<Option<String>, String> {
    let room = unhex::<16>(&room_id)?;
    let id = unhex::<32>(&event_id)?;
    let data = on_node(&node, move |core, _| core.image_data(&room, &id)).await?;
    Ok(data.and_then(|d| Some(format!("data:{};base64,{}", image_mime(&d)?, base64(&d)))))
}

/// 在当前群的文字消息里搜索，从新到旧最多 50 条。
#[tauri::command]
pub async fn chat_search(
    node: State<'_, NodeHandle>,
    room_id: String,
    query: String,
) -> Result<Vec<MessageDto>, String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| {
        Ok(core
            .search_chat(&room, &query)?
            .iter()
            .map(MessageDto::from)
            .collect())
    })
    .await
}

/// 删除聊天（只影响本机）：还在的群藏起来、来新消息再出现，已退出或已解散的删除本机记录。
#[tauri::command]
pub async fn rooms_delete_chat(node: State<'_, NodeHandle>, room_id: String) -> Result<(), String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| core.delete_chat(&room, now_ms())).await
}

/// 清空本机的聊天记录显示（只影响本机）。
#[tauri::command]
pub async fn rooms_clear_history(
    node: State<'_, NodeHandle>,
    room_id: String,
) -> Result<RoomDto, String> {
    let room = unhex::<16>(&room_id)?;
    on_node(&node, move |core, _| {
        Ok(RoomDto::from(&core.clear_history(&room, now_ms())?))
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

/// 导出备份，写到「文档\钢一下备份」，返回完整路径。`stamp` 是界面给的本地时间（如 20261103-1530），只用来起文件名。
/// 写文件在 Rust 侧做，不需要给前端开文件系统权限。
#[tauri::command]
pub async fn backup_export(
    app: tauri::AppHandle,
    node: State<'_, NodeHandle>,
    kind: String,
    passphrase: String,
    stamp: String,
) -> Result<String, String> {
    let (kind, label) = match kind.as_str() {
        "identity" => (BackupKind::IdentityOnly, "身份"),
        "full" => (BackupKind::Full, "完整"),
        _ => return Err("备份种类不对".into()),
    };
    if stamp.is_empty() || !stamp.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err("时间格式不对".into());
    }
    let bytes = on_node(&node, move |core, _| core.export_backup(kind, &passphrase)).await?;
    let dir = app
        .path()
        .document_dir()
        .map_err(|e| format!("找不到文档目录：{e}"))?
        .join("钢一下备份");
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建备份目录：{e}"))?;
    let path = dir.join(format!("钢一下备份-{label}-{stamp}.hsbk"));
    std::fs::write(&path, bytes).map_err(|e| format!("写入备份文件失败：{e}"))?;
    Ok(path.display().to_string())
}

/// 导入备份。`data` 是文件内容的 Base64（界面用文件选择框读出来）。本机已有房间时需要 `overwrite` 确认。
#[tauri::command]
pub async fn backup_import(
    node: State<'_, NodeHandle>,
    data: String,
    passphrase: String,
    overwrite: bool,
) -> Result<IdentityDto, String> {
    let bytes = unbase64(&data).map_err(|_| "读取备份文件失败".to_string())?;
    on_node(&node, move |core, _| {
        let view = core.import_backup(&bytes, &passphrase, overwrite, now_ms())?;
        Ok(IdentityDto::from(&view))
    })
    .await
}

#[tauri::command]
pub async fn settings_get(node: State<'_, NodeHandle>) -> Result<SettingsDto, String> {
    on_node(&node, |core, _| Ok(SettingsDto::from(&core.settings()?))).await
}

/// 窗口标题栏跟着界面主题：浅色、深色，跟随系统时交给 Windows。只在 Rust 侧调用，不给前端开窗口权限。
pub fn apply_window_theme(app: &tauri::AppHandle, theme: &str) {
    let t = match theme {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_theme(t);
    }
}

#[tauri::command]
pub async fn settings_update(
    app: tauri::AppHandle,
    node: State<'_, NodeHandle>,
    patch: SettingsPatchDto,
) -> Result<SettingsDto, String> {
    let theme_changed = patch.theme.is_some();
    let net_changed = patch.internet_allowed.is_some() || patch.relays.is_some();
    let dto = on_node(&node, move |core, out| {
        let s = core.update_settings(SettingsPatch {
            internet_allowed: patch.internet_allowed,
            relays: patch.relays,
            sound_enabled: patch.sound_enabled,
            volume: patch.volume,
            history_limit_lan: patch.history_limit_lan,
            history_limit_relay: patch.history_limit_relay,
            theme: patch.theme,
        })?;
        // 只有网络相关的项变了才需要重建中继连接池
        out.settings_changed = net_changed;
        Ok(SettingsDto::from(&s))
    })
    .await?;
    if theme_changed {
        apply_window_theme(&app, &dto.theme);
    }
    Ok(dto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatMessage, NoticeKind, RoomNotice, TimelineItem};
    use crate::storage::events::Delivery;

    #[test]
    fn leaderboard_ranks_share_ties() {
        let board = vec![
            ([1u8; 32], 3),
            ([2u8; 32], 3),
            ([3u8; 32], 1),
            ([4u8; 32], 0),
        ];
        let dto = leaderboard_dto(&board);
        assert_eq!(
            dto.iter().map(|e| e.rank).collect::<Vec<_>>(),
            vec![1, 1, 3, 4]
        );
        assert_eq!(dto[2].member_id, hex(&[3u8; 32]));
        assert_eq!(dto[2].stacks, 1);
    }

    #[test]
    fn room_dto_carries_local_prefs() {
        let r = RoomSummary {
            room_id: [1u8; 16],
            title: None,
            status: RoomStatus::Active,
            is_owner: false,
            unread: 0,
            last_active: 0,
            muted: true,
            pinned: false,
            preview: None,
            mentioned: false,
        };
        let d = RoomDto::from(&r);
        assert!(d.muted && !d.pinned);
    }

    #[test]
    fn member_path_and_relay_state_names() {
        let m = MemberView {
            member_id: [1; 32],
            fingerprint: "abcdef".into(),
            nickname: None,
            stacks: 2,
            is_owner: false,
            in_room: true,
            avatar: None,
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
    fn avatars_become_dtos() {
        assert_eq!(avatar_dto(&None), None);
        assert_eq!(
            avatar_dto(&Some(Avatar::Preset { icon: 3, color: 4 })),
            Some(AvatarDto::Preset { icon: 3, color: 4 })
        );
        let png = b"\x89PNG\r\n".to_vec();
        let Some(AvatarDto::Image { data_url }) = avatar_dto(&Some(Avatar::Image(png))) else {
            panic!("应当是图片");
        };
        assert!(data_url.starts_with("data:image/png;base64,"));
        // 认不出格式的图片不给界面
        assert_eq!(avatar_dto(&Some(Avatar::Image(b"<svg>".to_vec()))), None);
        let json = serde_json::to_value(AvatarDto::Preset { icon: 1, color: 2 }).unwrap();
        assert_eq!(json["kind"], "preset");
        let input: AvatarInput = serde_json::from_str(r#"{"kind":"image","data":"AAEC"}"#).unwrap();
        assert!(matches!(input, AvatarInput::Image { data } if data == "AAEC"));
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
            reply: None,
            mentions: vec![[0xab; 8]],
            image: None,
        });
        let dto = MessageDto::from(&chat);
        assert_eq!(
            (dto.kind, dto.text.as_deref(), dto.display_ts),
            ("chat", Some("你好"), 11)
        );
        assert_eq!(dto.delivery, Some("pending"));
        assert_eq!(dto.mentions, vec!["ab".repeat(8)]);
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
