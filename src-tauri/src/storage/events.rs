//! 事件表读写。
//!
//! 实现流程：
//! 1. `put_event`：event_id 已存在就返回 Duplicate；同一作者、同一 stream、同一 seq 已有别的 event_id，
//!    说明出现分叉（架构 4.4），两条都标记 is_fork 并返回 Fork；否则正常插入。
//!    插入时数据库分配自增的 local_seq，表示本机收到的先后顺序，用于未读和已读游标。
//! 2. 查询把行原样还原成 `SignedEvent`，字段一个不改，所以还原后仍能通过验签。
//! 3. `timeline` 按 (display_ts, event_id) 倒序取一页再翻成正序；翻页游标是本页第一条的 (display_ts, event_id)。
//! 4. `append_own`：本人写新事件的唯一入口，取下一个 seq 和严格递增的 ts（架构 4.1），签名后入库，投递状态为 Pending。
//! 5. 「消息」指文字聊天和图片（`MESSAGE_KINDS`），图片的各片不算；未读、预览、最近 n 条都只数消息。
//! 6. 「看得见」（`Visible`）：不在解散之后，也不在本机清空聊天记录之前（入库序号和显示时间都晚于清空时的分界）。
//!    聊天窗口、未读、预览、搜索都只看看得见的；同步和层数不受影响。

use rusqlite::{Connection, OptionalExtension, Params, Row, named_params, params};

use crate::crypto::{EventId, Identity, MemberId, RoomId};
use crate::error::AppResult;
use crate::protocol::event::{SignedEvent, Stream, UnsignedEvent, build_event, kind, next_ts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Pending,
    Sent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PutResult {
    Inserted(i64),
    Duplicate,
    Fork(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredEvent {
    pub event: SignedEvent,
    pub local_seq: i64,
    pub display_ts: u64,
    pub delivery: Option<Delivery>,
    pub is_fork: bool,
}

/// 算作一条消息的 kind：文字和图片（不含图片的各片）。
pub const MESSAGE_KINDS: [u16; 2] = [kind::CHAT, kind::IMAGE];

fn message_kinds() -> String {
    MESSAGE_KINDS.map(|k| k.to_string()).join(",")
}

/// 哪些条目看得见：不在解散之后；本机清空过聊天记录时，入库序号和显示时间都要晚于分界。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Visible {
    pub dissolve_ts: Option<u64>,
    pub cleared_seq: Option<i64>,
    pub cleared_before: Option<u64>,
}

const VISIBLE: &str = "(:dissolve IS NULL OR ts_ms <= :dissolve)
     AND (:cleared_seq IS NULL OR local_seq > :cleared_seq)
     AND (:cleared_before IS NULL OR display_ts > :cleared_before)";

/// 会出现在聊天窗口里的 kind：消息和系统提示。
const TIMELINE_KINDS: [u16; 7] = [
    kind::CHAT,
    kind::IMAGE,
    kind::JOIN,
    kind::LEAVE,
    kind::RENAME,
    kind::CONFIG_NOTICE,
    kind::DISSOLVE,
];

const SELECT: &str = "SELECT local_seq, event_id, room_id, author_id, format, stream, seq, ts_ms, kind, body, sig, \
                      display_ts, delivery, is_fork FROM events";

fn read(row: &Row) -> rusqlite::Result<StoredEvent> {
    let stream = match row.get::<_, u8>(5)? {
        0 => Stream::State,
        1 => Stream::Chat,
        v => return Err(rusqlite::Error::IntegralValueOutOfRange(5, v.into())),
    };
    let delivery = match row.get::<_, Option<u8>>(12)? {
        None => None,
        Some(0) => Some(Delivery::Pending),
        Some(_) => Some(Delivery::Sent),
    };
    Ok(StoredEvent {
        local_seq: row.get(0)?,
        event: SignedEvent {
            event_id: row.get(1)?,
            event: UnsignedEvent {
                room_id: row.get(2)?,
                author_id: row.get(3)?,
                format: row.get(4)?,
                stream,
                seq: row.get(6)?,
                ts_ms: row.get(7)?,
                kind: row.get(8)?,
                body: row.get(9)?,
            },
            sig: row.get(10)?,
        },
        display_ts: row.get(11)?,
        delivery,
        is_fork: row.get(13)?,
    })
}

fn collect(conn: &Connection, sql: &str, p: impl Params) -> AppResult<Vec<StoredEvent>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(p, read)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn put_event(
    conn: &Connection,
    e: &SignedEvent,
    display_ts: u64,
    delivery: Option<Delivery>,
) -> AppResult<PutResult> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT local_seq FROM events WHERE event_id = ?1",
            [e.event_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_some() {
        return Ok(PutResult::Duplicate);
    }
    let ev = &e.event;
    let slot = params![ev.room_id, ev.author_id, ev.stream as u8, ev.seq];
    let others: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE room_id = ?1 AND author_id = ?2 AND stream = ?3 AND seq = ?4",
        slot,
        |r| r.get(0),
    )?;
    let fork = others > 0;
    if fork {
        conn.execute(
            "UPDATE events SET is_fork = 1 WHERE room_id = ?1 AND author_id = ?2 AND stream = ?3 AND seq = ?4",
            slot,
        )?;
    }
    conn.execute(
        "INSERT INTO events (event_id, room_id, author_id, format, stream, seq, ts_ms, kind, body, sig, display_ts, delivery, is_fork)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            e.event_id,
            ev.room_id,
            ev.author_id,
            ev.format,
            ev.stream as u8,
            ev.seq,
            ev.ts_ms,
            ev.kind,
            ev.body,
            e.sig,
            display_ts,
            delivery.map(|d| d as u8),
            fork,
        ],
    )?;
    let local_seq = conn.last_insert_rowid();
    Ok(if fork {
        PutResult::Fork(local_seq)
    } else {
        PutResult::Inserted(local_seq)
    })
}

/// 某作者在某 stream 的全部事件，按 (seq, event_id) 升序。
pub fn query_by_author(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
    stream: Stream,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = ?1 AND author_id = ?2 AND stream = ?3 ORDER BY seq, event_id"
        ),
        params![room, author, stream as u8],
    )
}

/// 某作者某 kind 的全部事件，按 (seq, event_id) 升序。
pub fn query_kind_by_author(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
    k: u16,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = ?1 AND author_id = ?2 AND kind = ?3 ORDER BY seq, event_id"
        ),
        params![room, author, k],
    )
}

/// 房间里某 stream 的全部事件，按本机入库顺序。
pub fn query_room_stream(
    conn: &Connection,
    room: &RoomId,
    stream: Stream,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!("{SELECT} WHERE room_id = ?1 AND stream = ?2 ORDER BY local_seq"),
        params![room, stream as u8],
    )
}

/// 房间里的全部事件，按本机入库顺序。
pub fn query_room(conn: &Connection, room: &RoomId) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!("{SELECT} WHERE room_id = ?1 ORDER BY local_seq"),
        [room],
    )
}

/// 房间里发过叠层事件的作者。
pub fn heartsteel_authors(conn: &Connection, room: &RoomId) -> AppResult<Vec<MemberId>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT author_id FROM events WHERE room_id = ?1 AND kind = ?2")?;
    let rows = stmt.query_map(params![room, kind::HEARTSTEEL], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn next_seq(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
    stream: Stream,
) -> AppResult<u64> {
    let max: Option<u64> = conn.query_row(
        "SELECT MAX(seq) FROM events WHERE room_id = ?1 AND author_id = ?2 AND stream = ?3",
        params![room, author, stream as u8],
        |r| r.get(0),
    )?;
    Ok(max.unwrap_or(0) + 1)
}

/// 本人在该房间最后一条事件的 ts_ms（两个 stream 合并），用于保证时间严格递增。
pub fn last_ts(conn: &Connection, room: &RoomId, author: &MemberId) -> AppResult<Option<u64>> {
    Ok(conn.query_row(
        "SELECT MAX(ts_ms) FROM events WHERE room_id = ?1 AND author_id = ?2",
        params![room, author],
        |r| r.get(0),
    )?)
}

/// 聊天窗口的一页，正序。`before` 为 None 时取最新一页；只取看得见的。
pub fn timeline(
    conn: &Connection,
    room: &RoomId,
    before: Option<(u64, EventId)>,
    v: Visible,
    limit: u32,
) -> AppResult<Vec<StoredEvent>> {
    let kinds = TIMELINE_KINDS.map(|k| k.to_string()).join(",");
    let (cursor_ts, cursor_id) = match before {
        Some((ts, id)) => (Some(ts), Some(id)),
        None => (None, None),
    };
    let mut page = collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = :room AND kind IN ({kinds})
             AND (:cursor_ts IS NULL OR display_ts < :cursor_ts OR (display_ts = :cursor_ts AND event_id < :cursor_id))
             AND {VISIBLE}
             ORDER BY display_ts DESC, event_id DESC LIMIT :limit"
        ),
        named_params! {":room": room, ":cursor_ts": cursor_ts, ":cursor_id": cursor_id, ":limit": limit,
        ":dissolve": v.dissolve_ts, ":cleared_seq": v.cleared_seq, ":cleared_before": v.cleared_before},
    )?;
    page.reverse();
    Ok(page)
}

pub fn max_local_seq(conn: &Connection, room: &RoomId) -> AppResult<i64> {
    let max: Option<i64> = conn.query_row(
        "SELECT MAX(local_seq) FROM events WHERE room_id = ?1",
        [room],
        |r| r.get(0),
    )?;
    Ok(max.unwrap_or(0))
}

/// 未读：他人发的、local_seq 大于已读游标、看得见的消息条数。
pub fn count_unread(
    conn: &Connection,
    room: &RoomId,
    me: &MemberId,
    after: i64,
    v: Visible,
) -> AppResult<u32> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM events WHERE room_id = :room AND kind IN ({}) AND author_id != :me
             AND local_seq > :after AND {VISIBLE}",
            message_kinds()
        ),
        named_params! {":room": room, ":me": me, ":after": after, ":dissolve": v.dissolve_ts, ":cleared_seq": v.cleared_seq, ":cleared_before": v.cleared_before},
        |r| r.get(0),
    )?)
}

/// 未读的他人文字消息，最新的在前，最多 limit 条（用来看有没有人 @ 我）。
pub fn unread_chats(
    conn: &Connection,
    room: &RoomId,
    me: &MemberId,
    after: i64,
    v: Visible,
    limit: u32,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = :room AND kind = :kind AND author_id != :me AND local_seq > :after
             AND {VISIBLE} ORDER BY local_seq DESC LIMIT :limit"
        ),
        named_params! {":room": room, ":kind": kind::CHAT, ":me": me, ":after": after,
        ":dissolve": v.dissolve_ts, ":cleared_seq": v.cleared_seq, ":cleared_before": v.cleared_before, ":limit": limit},
    )
}

/// 显示顺序上最后一条看得见的消息（房间列表的预览）。
pub fn last_message(
    conn: &Connection,
    room: &RoomId,
    v: Visible,
) -> AppResult<Option<StoredEvent>> {
    Ok(collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = :room AND kind IN ({}) AND {VISIBLE}
             ORDER BY display_ts DESC, event_id DESC LIMIT 1",
            message_kinds()
        ),
        named_params! {":room": room, ":dissolve": v.dissolve_ts, ":cleared_seq": v.cleared_seq, ":cleared_before": v.cleared_before},
    )?
    .pop())
}

/// 看得见的文字消息，从新到旧（搜索用）。
pub fn chats_newest_first(
    conn: &Connection,
    room: &RoomId,
    v: Visible,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = :room AND kind = :kind AND stream = 1 AND {VISIBLE}
             ORDER BY display_ts DESC, event_id DESC"
        ),
        named_params! {":room": room, ":kind": kind::CHAT, ":dissolve": v.dissolve_ts, ":cleared_seq": v.cleared_seq, ":cleared_before": v.cleared_before},
    )
}

/// 按事件 ID 取本房间的一条事件。
pub fn get_in_room(
    conn: &Connection,
    room: &RoomId,
    id: &EventId,
) -> AppResult<Option<StoredEvent>> {
    Ok(collect(
        conn,
        &format!("{SELECT} WHERE room_id = ?1 AND event_id = ?2"),
        params![room, id],
    )?
    .pop())
}

/// 本人在某房间写一条新事件并入库，返回签好名的事件。聊天流的事件同时把房间的最近活跃时间推到这条事件。
pub fn append_own(
    conn: &Connection,
    me: &Identity,
    room: &RoomId,
    stream: Stream,
    k: u16,
    body: Vec<u8>,
    now_ms: u64,
) -> AppResult<SignedEvent> {
    let author = me.member_id();
    let seq = next_seq(conn, room, &author, stream)?;
    let ts = next_ts(now_ms, last_ts(conn, room, &author)?);
    let e = build_event(me, *room, stream, seq, ts, k, body);
    put_event(conn, &e, ts, Some(Delivery::Pending))?;
    // 只有聊天流（发言、发图）算房间活跃；改资料、叠层这些状态事件不改变房间列表的时间和排序
    if stream == Stream::Chat {
        crate::storage::rooms::touch(conn, room, ts)?;
    }
    Ok(e)
}

/// 本人某个流上 seq 不超过 `upto` 且仍是 Pending 的事件改为 Sent，返回改了的事件 ID。
pub fn mark_sent(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
    stream: Stream,
    upto: u64,
) -> AppResult<Vec<EventId>> {
    let mut stmt = conn.prepare(
        "UPDATE events SET delivery = 1 WHERE room_id = ?1 AND author_id = ?2 AND stream = ?3 AND seq <= ?4 AND delivery = 0 RETURNING event_id",
    )?;
    let rows = stmt.query_map(params![room, author, stream as u8, upto], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 最近 n 条消息（按显示顺序，不含图片的各片），最新的在前。
pub fn recent_messages(conn: &Connection, room: &RoomId, n: u32) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = ?1 AND stream = 1 AND kind IN ({})
             ORDER BY display_ts DESC, event_id DESC LIMIT ?2",
            message_kinds()
        ),
        params![room, n],
    )
}

/// 本人还没被确认的事件，按 local_seq 升序。
pub fn pending_own(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = ?1 AND author_id = ?2 AND delivery = 0 ORDER BY local_seq"
        ),
        params![room, author],
    )
}

/// 房间里每条事件的 (作者, stream, seq)，用来算持有向量。
pub fn holdings_rows(conn: &Connection, room: &RoomId) -> AppResult<Vec<(MemberId, u8, u64)>> {
    let mut stmt = conn.prepare("SELECT author_id, stream, seq FROM events WHERE room_id = ?1")?;
    let rows = stmt.query_map([room], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 房间里出现过的所有作者。
pub fn room_authors(conn: &Connection, room: &RoomId) -> AppResult<Vec<MemberId>> {
    let mut stmt = conn.prepare("SELECT DISTINCT author_id FROM events WHERE room_id = ?1")?;
    let rows = stmt.query_map([room], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 某作者某 stream 序号在 [from, to] 之间的事件（含分叉），按 (seq, event_id) 升序。
pub fn query_range(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
    stream: Stream,
    from: u64,
    to: u64,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = ?1 AND author_id = ?2 AND stream = ?3 AND seq BETWEEN ?4 AND ?5 ORDER BY seq, event_id"
        ),
        params![room, author, stream as u8, from, to],
    )
}
