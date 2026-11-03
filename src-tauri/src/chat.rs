//! 聊天：显示时间和聊天窗口条目（架构 4.5、8.1）。发言、发图、消息列表、未读、搜索、房间预览、清空记录也放在这里。
//!
//! 实现流程：
//! 1. `display_ts`：显示顺序是本机视图，不参与层数计算。实时收到且和本机时间相差超过 2 分钟，
//!    或补齐收到且时间在本机 2 分钟之后，按本机时间显示；其余按作者时间。
//! 2. `to_item`：把一条入库事件转成聊天窗口条目。文字消息解码出文字、引用和 @（引用的原消息在本机时补上原作者）；
//!    图片消息看前面的各片是否到齐；Join、Leave、Rename、ConfigNotice、Dissolve 成为系统提示，
//!    其中改名、改规则、解散只显示房主发的（别人的不生效）；解散时间之后的消息、本机清空之前的条目不显示。
//! 3. `send_chat`（架构 5.3）：只有 Active 房间能发言，文本不超过 900 字节；引用的必须是本房间的消息，
//!    摘要截到 60 字节（不切坏字符）；@ 去重、最多 20 人，存 8 字节短 ID；整条编码后不超过 1024 字节。
//!    在调用方的事务里写入聊天事件，紧接着判定叠层；触发就再写一条叠层事件并重算本人层数。叠层判定只走这一条路径。
//! 4. `send_image`：图片切成每片 900 字节的 IMAGE_CHUNK，紧接着一条 IMAGE（片数、字节数、SHA-256、宽高）。
//!    图片不参与叠层判定：层数由每个人各自重放，改变计数规则会和旧版本算出不同的层数。
//! 5. `image_data`：按作者和序号取回前面的 n 片，拼起来核对字节数、SHA-256 和文件头，都对才返回。
//! 6. 清空聊天记录只影响本机显示：记下当时最大的入库序号和时刻。之后入库序号不大于它的、
//!    或显示时间早于「清空时刻 − 2 分钟」的条目都不显示。前一条挡住已经在本机的；后一条挡住之后才补齐来的旧消息，
//!    又留出 2 分钟给时钟慢一点的群友，免得他们清空之后刚发的消息被藏起来。
//!    不真正删除：删掉本人的聊天会让以后的序号重复（变成分叉），删掉别人的会被同步补回来，层数也要靠它们重放。
//! 7. 搜索只搜看得见的文字消息，不分大小写，从新到旧；房间预览取最后一条看得见的消息；未读里有没有人 @ 我。

use crate::crypto::{self, EventId, Identity, MemberId, RoomId};
use crate::error::{AppError, AppResult};
use crate::heartsteel::service::{self as hs, Replayed};
use crate::protocol::body::{
    self, CHAT_TEXT_MAX_BYTES, ChatBody, IMAGE_CHUNK_BYTES, IMAGE_MAX_BYTES, IMAGE_MAX_CHUNKS,
    ImageBody, ImageChunkBody, MAX_MENTIONS, Mention, REPLY_EXCERPT_MAX_BYTES, RenameBody,
    ReplyRef, image_mime,
};
use crate::protocol::event::{MAX_BODY_LEN, SignedEvent, Stream, kind};
use crate::protocol::frame::{ShortId, short_id};
use crate::protocol::room_code::owner_tag_of;
use crate::storage::Connection;
use crate::storage::events::{self, Delivery, StoredEvent, Visible};
use crate::storage::rooms::{self as room_store, RoomRow, RoomStatus};

pub const CLOCK_SKEW_MS: u64 = 120_000;
/// 房间列表预览最多多少个字。
const PREVIEW_MAX_CHARS: usize = 40;
/// 看有没有人 @ 我时最多翻多少条未读。
const MENTION_SCAN: u32 = 200;
const IMAGE_PLACEHOLDER: &str = "[图片]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyView {
    pub event_id: EventId,
    pub excerpt: String,
    /// 原消息在本机时才知道是谁发的。
    pub author_id: Option<MemberId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageView {
    pub width: u16,
    pub height: u16,
    /// 各片都到齐了。
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub event_id: EventId,
    pub author_id: MemberId,
    pub ts_ms: u64,
    pub display_ts: u64,
    /// 图片消息为空。
    pub text: String,
    /// 只对本人消息有值。
    pub delivery: Option<Delivery>,
    pub reply: Option<Box<ReplyView>>,
    /// 被 @ 的人的短 ID。
    pub mentions: Vec<ShortId>,
    pub image: Option<ImageView>,
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

/// 发一条文字消息时界面给的内容。
#[derive(Clone, Copy, Debug, Default)]
pub struct Draft<'a> {
    pub text: &'a str,
    pub reply_to: Option<EventId>,
    pub mentions: &'a [MemberId],
}

pub fn display_ts(ts_ms: u64, now_ms: u64, is_live: bool) -> u64 {
    let skewed = if is_live {
        ts_ms.abs_diff(now_ms) > CLOCK_SKEW_MS
    } else {
        ts_ms > now_ms + CLOCK_SKEW_MS
    };
    if skewed { now_ms } else { ts_ms }
}

/// 本房间哪些条目看得见。聊天窗口不按解散时间过滤（解散之后的系统提示照常显示），由 `to_item` 只挡解散后的消息。
pub fn visible(row: &RoomRow, with_dissolve: bool) -> Visible {
    Visible {
        dissolve_ts: if with_dissolve { row.dissolve_ts } else { None },
        cleared_seq: row.cleared_seq,
        cleared_before: row.cleared_ts.map(|t| t.saturating_sub(CLOCK_SKEW_MS)),
    }
}

fn hidden_by_clear(s: &StoredEvent, row: &RoomRow) -> bool {
    let v = visible(row, false);
    v.cleared_seq.is_some_and(|c| s.local_seq <= c)
        || v.cleared_before.is_some_and(|c| s.display_ts <= c)
}

/// 引用摘要：换行变空格，按字符截到 60 字节以内。
fn excerpt(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars().map(|c| if c == '\n' { ' ' } else { c }) {
        if out.len() + ch.len_utf8() > REPLY_EXCERPT_MAX_BYTES {
            break;
        }
        out.push(ch);
    }
    out
}

/// 一条消息事件的文字（图片给占位文字），解不开时为 None。
fn message_text(e: &StoredEvent) -> Option<String> {
    let ev = &e.event.event;
    match ev.kind {
        kind::CHAT => body::decode::<ChatBody>(&ev.body).ok().map(|b| b.text),
        kind::IMAGE => Some(IMAGE_PLACEHOLDER.to_string()),
        _ => None,
    }
}

/// 图片 n 片的序号区间；片数不合理时为 None。
fn chunk_range(seq: u64, b: &ImageBody) -> Option<(u64, u64)> {
    let n = u64::from(b.chunks);
    if n == 0 || n > IMAGE_MAX_CHUNKS || seq <= n {
        return None;
    }
    Some((seq - n, seq - 1))
}

/// 前面的各片：每个序号恰好一片、都是 IMAGE_CHUNK 才算到齐。
fn image_parts(
    conn: &Connection,
    s: &StoredEvent,
    b: &ImageBody,
) -> AppResult<Option<Vec<StoredEvent>>> {
    let ev = &s.event.event;
    let Some((from, to)) = chunk_range(ev.seq, b) else {
        return Ok(None);
    };
    let parts = events::query_range(conn, &ev.room_id, &ev.author_id, Stream::Chat, from, to)?;
    let complete = parts.len() as u64 == to - from + 1
        && parts.iter().enumerate().all(|(i, p)| {
            p.event.event.seq == from + i as u64 && p.event.event.kind == kind::IMAGE_CHUNK
        });
    Ok(complete.then_some(parts))
}

pub fn to_item(
    conn: &Connection,
    s: &StoredEvent,
    row: &RoomRow,
) -> AppResult<Option<TimelineItem>> {
    let e = &s.event.event;
    if hidden_by_clear(s, row) {
        return Ok(None);
    }
    if e.kind == kind::CHAT || e.kind == kind::IMAGE {
        if e.stream != Stream::Chat || row.dissolve_ts.is_some_and(|d| e.ts_ms > d) {
            return Ok(None);
        }
        let mut m = ChatMessage {
            event_id: s.event.event_id,
            author_id: e.author_id,
            ts_ms: e.ts_ms,
            display_ts: s.display_ts,
            text: String::new(),
            delivery: s.delivery,
            reply: None,
            mentions: Vec::new(),
            image: None,
        };
        if e.kind == kind::CHAT {
            let Ok(b) = body::decode::<ChatBody>(&e.body) else {
                return Ok(None);
            };
            m.text = b.text;
            m.mentions = b.mentions.iter().map(|x| x.0).collect();
            if let Some(r) = b.reply {
                let author_id = events::get_in_room(conn, &row.room_id, &r.id)?
                    .map(|o| o.event.event.author_id);
                m.reply = Some(Box::new(ReplyView {
                    event_id: r.id,
                    excerpt: r.excerpt,
                    author_id,
                }));
            }
        } else {
            let Ok(b) = body::decode::<ImageBody>(&e.body) else {
                return Ok(None);
            };
            m.image = Some(ImageView {
                width: b.width,
                height: b.height,
                ready: image_parts(conn, s, &b)?.is_some(),
            });
        }
        return Ok(Some(TimelineItem::Chat(m)));
    }
    if e.stream != Stream::State {
        return Ok(None);
    }
    let by_owner = owner_tag_of(&e.author_id) == row.owner_tag;
    let (kind, title) = match e.kind {
        kind::JOIN => (NoticeKind::Join, None),
        kind::LEAVE => (NoticeKind::Leave, None),
        kind::RENAME if by_owner => match body::decode::<RenameBody>(&e.body) {
            Ok(b) => (NoticeKind::Rename, Some(b.title)),
            Err(_) => return Ok(None),
        },
        kind::CONFIG_NOTICE if by_owner => (NoticeKind::ConfigNotice, None),
        kind::DISSOLVE if by_owner => (NoticeKind::Dissolve, None),
        _ => return Ok(None),
    };
    Ok(Some(TimelineItem::Notice(RoomNotice {
        event_id: s.event.event_id,
        kind,
        author_id: e.author_id,
        ts_ms: e.ts_ms,
        display_ts: s.display_ts,
        title,
    })))
}

/// 本次发言触发的叠层：重放结果和叠层事件的 ID。
pub struct Stacked {
    pub replayed: Replayed,
    pub event_id: EventId,
}

fn active_room(conn: &Connection, room: &RoomId) -> AppResult<RoomRow> {
    let row = room_store::get_room(conn, room)?.ok_or(AppError::RoomNotFound)?;
    if row.status != RoomStatus::Active {
        return Err(AppError::RoomNotActive);
    }
    Ok(row)
}

/// 返回 (聊天消息, 本次新写的事件, 触发的叠层)。
pub fn send_chat(
    conn: &Connection,
    me: &Identity,
    room: &RoomId,
    d: &Draft,
    now_ms: u64,
) -> AppResult<(ChatMessage, Vec<SignedEvent>, Option<Stacked>)> {
    active_room(conn, room)?;
    if d.text.len() > CHAT_TEXT_MAX_BYTES {
        return Err(AppError::TextTooLong);
    }
    let reply = match d.reply_to {
        None => None,
        Some(id) => {
            let orig = events::get_in_room(conn, room, &id)?
                .filter(|o| o.event.event.stream == Stream::Chat)
                .ok_or(AppError::ReplyNotFound)?;
            let text = message_text(&orig).ok_or(AppError::ReplyNotFound)?;
            Some(ReplyRef {
                id,
                excerpt: excerpt(&text),
            })
        }
    };
    let mut mentions: Vec<ShortId> = Vec::new();
    for m in d.mentions {
        let s = short_id(m);
        if !mentions.contains(&s) && mentions.len() < MAX_MENTIONS {
            mentions.push(s);
        }
    }
    let body = body::encode(&ChatBody {
        text: d.text.to_string(),
        reply: reply.clone(),
        mentions: mentions.iter().map(|s| Mention(*s)).collect(),
    });
    if body.len() > MAX_BODY_LEN {
        return Err(AppError::TextTooLong);
    }
    let chat_ev = events::append_own(conn, me, room, Stream::Chat, kind::CHAT, body, now_ms)?;
    let author_of_reply = match &reply {
        Some(r) => events::get_in_room(conn, room, &r.id)?.map(|o| o.event.event.author_id),
        None => None,
    };
    let message = ChatMessage {
        event_id: chat_ev.event_id,
        author_id: me.member_id(),
        ts_ms: chat_ev.event.ts_ms,
        display_ts: chat_ev.event.ts_ms,
        text: d.text.to_string(),
        delivery: Some(Delivery::Pending),
        reply: reply.map(|r| {
            Box::new(ReplyView {
                event_id: r.id,
                excerpt: r.excerpt,
                author_id: author_of_reply,
            })
        }),
        mentions,
        image: None,
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

/// 发一张图片（界面已经压缩过）。返回 (图片消息, 本次新写的事件：各片在前，图片消息在最后)。
pub fn send_image(
    conn: &Connection,
    me: &Identity,
    room: &RoomId,
    data: &[u8],
    (width, height): (u16, u16),
    now_ms: u64,
) -> AppResult<(ChatMessage, Vec<SignedEvent>)> {
    active_room(conn, room)?;
    if data.is_empty() || image_mime(data).is_none() {
        return Err(AppError::InvalidImage);
    }
    if data.len() > IMAGE_MAX_BYTES {
        return Err(AppError::ImageTooLarge);
    }
    let mut written = Vec::new();
    for part in data.chunks(IMAGE_CHUNK_BYTES) {
        let b = body::encode(&ImageChunkBody {
            data: part.to_vec(),
        });
        written.push(events::append_own(
            conn,
            me,
            room,
            Stream::Chat,
            kind::IMAGE_CHUNK,
            b,
            now_ms,
        )?);
    }
    let head = body::encode(&ImageBody {
        chunks: written.len() as u16,
        size: data.len() as u32,
        sha256: crypto::sha256(&[data]),
        width,
        height,
    });
    let e = events::append_own(conn, me, room, Stream::Chat, kind::IMAGE, head, now_ms)?;
    let message = ChatMessage {
        event_id: e.event_id,
        author_id: me.member_id(),
        ts_ms: e.event.ts_ms,
        display_ts: e.event.ts_ms,
        text: String::new(),
        delivery: Some(Delivery::Pending),
        reply: None,
        mentions: Vec::new(),
        image: Some(ImageView {
            width,
            height,
            ready: true,
        }),
    };
    written.push(e);
    Ok((message, written))
}

/// 图片的完整内容；没到齐、被改过或不是图片时为 None。
pub fn image_data(conn: &Connection, room: &RoomId, id: &EventId) -> AppResult<Option<Vec<u8>>> {
    let Some(s) = events::get_in_room(conn, room, id)? else {
        return Ok(None);
    };
    let ev = &s.event.event;
    if ev.kind != kind::IMAGE || ev.stream != Stream::Chat {
        return Ok(None);
    }
    let Ok(b) = body::decode::<ImageBody>(&ev.body) else {
        return Ok(None);
    };
    let Some(parts) = image_parts(conn, &s, &b)? else {
        return Ok(None);
    };
    let mut data = Vec::with_capacity(b.size as usize);
    for p in &parts {
        let Ok(c) = body::decode::<ImageChunkBody>(&p.event.event.body) else {
            return Ok(None);
        };
        data.extend(c.data);
    }
    let ok = data.len() == b.size as usize
        && crypto::sha256(&[data.as_slice()]) == b.sha256
        && image_mime(&data).is_some();
    Ok(ok.then_some(data))
}

/// 这些新到的图片片补齐了哪些已在本机的图片（各片后到时，界面要据此刷新）。
pub fn images_completed_by(
    conn: &Connection,
    room: &RoomId,
    chunks: &[&StoredEvent],
) -> AppResult<Vec<EventId>> {
    let mut done = Vec::new();
    for c in chunks {
        let ev = &c.event.event;
        let heads = events::query_range(
            conn,
            room,
            &ev.author_id,
            Stream::Chat,
            ev.seq + 1,
            ev.seq + IMAGE_MAX_CHUNKS,
        )?;
        for h in heads.iter().filter(|h| h.event.event.kind == kind::IMAGE) {
            let Ok(b) = body::decode::<ImageBody>(&h.event.event.body) else {
                continue;
            };
            let covers =
                chunk_range(h.event.event.seq, &b).is_some_and(|(f, t)| (f..=t).contains(&ev.seq));
            if covers && !done.contains(&h.event.event_id) && image_parts(conn, h, &b)?.is_some() {
                done.push(h.event.event_id);
            }
        }
    }
    Ok(done)
}

/// 新成员同步用：最近 n 条消息里每位作者最早的序号；图片的分界挪到它的第一片（架构 7.4）。
pub fn floors_for_last(
    conn: &Connection,
    room: &RoomId,
    n: u32,
) -> AppResult<Vec<(MemberId, u64)>> {
    let mut floors: Vec<(MemberId, u64)> = Vec::new();
    for s in events::recent_messages(conn, room, n)? {
        let ev = &s.event.event;
        let mut floor = ev.seq;
        if ev.kind == kind::IMAGE
            && let Some((first, _)) = body::decode::<ImageBody>(&ev.body)
                .ok()
                .and_then(|b| chunk_range(ev.seq, &b))
        {
            floor = first;
        }
        match floors.iter_mut().find(|(a, _)| *a == ev.author_id) {
            Some((_, f)) => *f = (*f).min(floor),
            None => floors.push((ev.author_id, floor)),
        }
    }
    Ok(floors)
}

impl TimelineItem {
    /// 用这一条作为翻页游标：下一页取它之前的条目。
    pub fn cursor(&self) -> TimelineCursor {
        match self {
            TimelineItem::Chat(m) => TimelineCursor {
                display_ts: m.display_ts,
                event_id: m.event_id,
            },
            TimelineItem::Notice(n) => TimelineCursor {
                display_ts: n.display_ts,
                event_id: n.event_id,
            },
        }
    }
}

/// 聊天窗口的一页（正序，最多 limit 条）。有些事件不显示（非房主的管理事件、解散后的消息），
/// 过滤后不够一页就继续往前取，保证翻页不重不漏。
pub fn list_messages(
    conn: &Connection,
    room: &RoomId,
    before: Option<TimelineCursor>,
    limit: u32,
) -> AppResult<Vec<TimelineItem>> {
    let row = room_store::get_room(conn, room)?.ok_or(AppError::RoomNotFound)?;
    let mut cursor = before.map(|c| (c.display_ts, c.event_id));
    let mut items: Vec<TimelineItem> = Vec::new();
    loop {
        let raw = events::timeline(conn, room, cursor, visible(&row, false), limit)?;
        let Some(oldest) = raw.first() else {
            break;
        };
        cursor = Some((oldest.display_ts, oldest.event.event_id));
        let mut older = Vec::new();
        for s in &raw {
            if let Some(item) = to_item(conn, s, &row)? {
                older.push(item);
            }
        }
        older.append(&mut items);
        items = older;
        if items.len() >= limit as usize || raw.len() < limit as usize {
            break;
        }
    }
    let extra = items.len().saturating_sub(limit as usize);
    items.drain(..extra);
    Ok(items)
}

/// 搜索本房间看得见的文字消息：不分大小写，从新到旧，最多 limit 条。
pub fn search(
    conn: &Connection,
    room: &RoomId,
    query: &str,
    limit: usize,
) -> AppResult<Vec<TimelineItem>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let row = room_store::get_room(conn, room)?.ok_or(AppError::RoomNotFound)?;
    let mut out = Vec::new();
    for s in events::chats_newest_first(conn, room, visible(&row, true))? {
        let hit = body::decode::<ChatBody>(&s.event.event.body)
            .is_ok_and(|b| b.text.to_lowercase().contains(&q));
        if hit && let Some(item) = to_item(conn, &s, &row)? {
            out.push(item);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

/// 未读：他人发的、本机收到晚于已读游标的消息条数（架构 4.5，迟到的旧消息也算）。
pub fn unread(conn: &Connection, room: &RoomId, me: &MemberId) -> AppResult<u32> {
    let row = room_store::get_room(conn, room)?.ok_or(AppError::RoomNotFound)?;
    let cursor = room_store::read_cursor(conn, room)?;
    events::count_unread(conn, room, me, cursor, visible(&row, true))
}

/// 未读消息里有没有人 @ 我。
pub fn mentioned(conn: &Connection, row: &RoomRow, me: &MemberId) -> AppResult<bool> {
    let cursor = room_store::read_cursor(conn, &row.room_id)?;
    let mine = short_id(me);
    Ok(events::unread_chats(
        conn,
        &row.room_id,
        me,
        cursor,
        visible(row, true),
        MENTION_SCAN,
    )?
    .iter()
    .any(|s| {
        body::decode::<ChatBody>(&s.event.event.body)
            .is_ok_and(|b| b.mentions.contains(&Mention(mine)))
    }))
}

/// 房间列表的预览：最后一条看得见的消息。别人发的前面带名字（「乙：……」），自己发的不带。
pub fn preview(conn: &Connection, row: &RoomRow, me: &MemberId) -> AppResult<Option<String>> {
    let Some(s) = events::last_message(conn, &row.room_id, visible(row, true))? else {
        return Ok(None);
    };
    let Some(text) = message_text(&s) else {
        return Ok(None);
    };
    let text: String = text
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .take(PREVIEW_MAX_CHARS)
        .collect();
    let author = s.event.event.author_id;
    if author == *me {
        return Ok(Some(text));
    }
    let name = room_store::nickname_of(conn, &row.room_id, &author)?
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| crypto::fingerprint(&author).chars().take(6).collect());
    Ok(Some(format!("{name}：{text}")))
}

/// 已读游标移到当前最大的 local_seq。
pub fn mark_read(conn: &Connection, room: &RoomId) -> AppResult<()> {
    room_store::set_read_cursor(conn, room, events::max_local_seq(conn, room)?)
}

/// 清空本机的聊天记录显示（规则见模块说明第 6 条），同时标为已读。
pub fn clear_history(conn: &Connection, room: &RoomId, now_ms: u64) -> AppResult<()> {
    room_store::get_room(conn, room)?.ok_or(AppError::RoomNotFound)?;
    room_store::set_cleared(conn, room, events::max_local_seq(conn, room)?, now_ms)?;
    mark_read(conn, room)
}

/// 这条新消息要不要提醒本机用户：别人发的实时消息，房间没开免打扰，或者 @ 了我（免打扰也提醒）。
pub fn should_alert(item: &TimelineItem, is_live: bool, row: &RoomRow, me: &MemberId) -> bool {
    let TimelineItem::Chat(m) = item else {
        return false;
    };
    is_live && m.author_id != *me && (!row.muted || m.mentions.contains(&short_id(me)))
}
