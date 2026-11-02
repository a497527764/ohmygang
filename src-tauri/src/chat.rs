//! 聊天：显示时间和聊天窗口条目（架构 4.5、8.1）。发言、消息列表和未读也放在这里。
//!
//! 实现流程：
//! 1. `display_ts`：显示顺序是本机视图，不参与层数计算。实时收到且和本机时间相差超过 2 分钟，
//!    或补齐收到且时间在本机 2 分钟之后，按本机时间显示；其余按作者时间。
//! 2. `to_item`：把一条入库事件转成聊天窗口条目。聊天解码出文字；Join、Leave、Rename、ConfigNotice、Dissolve
//!    成为系统提示，其中改名、改规则、解散只显示房主发的（别人的不生效）；解散时间之后的聊天不显示。
//! 3. `send_text`（架构 5.3）：只有 Active 房间能发言，文本不超过 900 字节。在调用方的事务里写入聊天事件，
//!    紧接着判定叠层；触发就再写一条叠层事件并重算本人层数。叠层判定只走这一条路径。

use crate::crypto::{EventId, Identity, MemberId, RoomId};
use crate::error::{AppError, AppResult};
use crate::heartsteel::service::{self as hs, Replayed};
use crate::protocol::body::{self, CHAT_TEXT_MAX_BYTES, ChatBody, RenameBody};
use crate::protocol::event::{SignedEvent, Stream, kind};
use crate::protocol::room_code::owner_tag_of;
use crate::storage::Connection;
use crate::storage::events::{self, Delivery, StoredEvent};
use crate::storage::rooms::{self as room_store, RoomStatus};

pub const CLOCK_SKEW_MS: u64 = 120_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub event_id: EventId,
    pub author_id: MemberId,
    pub ts_ms: u64,
    pub display_ts: u64,
    pub text: String,
    /// 只对本人消息有值。
    pub delivery: Option<Delivery>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Join,
    Leave,
    Rename,
    ConfigNotice,
    Dissolve,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomNotice {
    pub event_id: EventId,
    pub kind: NoticeKind,
    pub author_id: MemberId,
    pub ts_ms: u64,
    pub display_ts: u64,
    /// 改名提示带上新群名。
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelineItem {
    Chat(ChatMessage),
    Notice(RoomNotice),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineCursor {
    pub display_ts: u64,
    pub event_id: EventId,
}

pub fn display_ts(ts_ms: u64, now_ms: u64, is_live: bool) -> u64 {
    let skewed = if is_live {
        ts_ms.abs_diff(now_ms) > CLOCK_SKEW_MS
    } else {
        ts_ms > now_ms + CLOCK_SKEW_MS
    };
    if skewed { now_ms } else { ts_ms }
}

pub fn to_item(
    s: &StoredEvent,
    owner_tag: &[u8; 10],
    dissolve_ts: Option<u64>,
) -> Option<TimelineItem> {
    let e = &s.event.event;
    if e.kind == kind::CHAT {
        if e.stream != Stream::Chat || dissolve_ts.is_some_and(|d| e.ts_ms > d) {
            return None;
        }
        let text = body::decode::<ChatBody>(&e.body).ok()?.text;
        return Some(TimelineItem::Chat(ChatMessage {
            event_id: s.event.event_id,
            author_id: e.author_id,
            ts_ms: e.ts_ms,
            display_ts: s.display_ts,
            text,
            delivery: s.delivery,
        }));
    }
    if e.stream != Stream::State {
        return None;
    }
    let by_owner = owner_tag_of(&e.author_id) == *owner_tag;
    let (kind, title) = match e.kind {
        kind::JOIN => (NoticeKind::Join, None),
        kind::LEAVE => (NoticeKind::Leave, None),
        kind::RENAME if by_owner => (
            NoticeKind::Rename,
            Some(body::decode::<RenameBody>(&e.body).ok()?.title),
        ),
        kind::CONFIG_NOTICE if by_owner => (NoticeKind::ConfigNotice, None),
        kind::DISSOLVE if by_owner => (NoticeKind::Dissolve, None),
        _ => return None,
    };
    Some(TimelineItem::Notice(RoomNotice {
        event_id: s.event.event_id,
        kind,
        author_id: e.author_id,
        ts_ms: e.ts_ms,
        display_ts: s.display_ts,
        title,
    }))
}

/// 本次发言触发的叠层：重放结果和叠层事件的 ID。
pub struct Stacked {
    pub replayed: Replayed,
    pub event_id: EventId,
}

/// 返回 (聊天消息, 本次新写的事件, 触发的叠层)。
pub fn send_text(
    conn: &Connection,
    me: &Identity,
    room: &RoomId,
    text: &str,
    now_ms: u64,
) -> AppResult<(ChatMessage, Vec<SignedEvent>, Option<Stacked>)> {
    let row = room_store::get_room(conn, room)?.ok_or(AppError::RoomNotFound)?;
    if row.status != RoomStatus::Active {
        return Err(AppError::RoomNotActive);
    }
    if text.len() > CHAT_TEXT_MAX_BYTES {
        return Err(AppError::TextTooLong);
    }
    let body = body::encode(&ChatBody {
        text: text.to_string(),
    });
    let chat_ev = events::append_own(conn, me, room, Stream::Chat, kind::CHAT, body, now_ms)?;
    let message = ChatMessage {
        event_id: chat_ev.event_id,
        author_id: me.member_id(),
        ts_ms: chat_ev.event.ts_ms,
        display_ts: chat_ev.event.ts_ms,
        text: text.to_string(),
        delivery: Some(Delivery::Pending),
    };
    let mut written = vec![chat_ev];
    let mut stacked = None;
    if let Some(b) = hs::evaluate_on_send(conn, &me.member_id(), room)? {
        let hs_ev = events::append_own(
            conn,
            me,
            room,
            Stream::State,
            kind::HEARTSTEEL,
            body::encode(&b),
            now_ms,
        )?;
        let replayed = hs::recompute_author(conn, room, &me.member_id())?;
        stacked = Some(Stacked {
            replayed,
            event_id: hs_ev.event_id,
        });
        written.push(hs_ev);
    }
    Ok((message, written, stacked))
}
