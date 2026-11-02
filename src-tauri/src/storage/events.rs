//! 事件表读写。
//!
//! 实现流程：
//! 1. `put_event`：event_id 已存在就返回 Duplicate；同一作者、同一 stream、同一 seq 已有别的 event_id，
//!    说明出现分叉（架构 4.4），两条都标记 is_fork 并返回 Fork；否则正常插入。
//!    插入时数据库分配自增的 local_seq，表示本机收到的先后顺序，用于未读和已读游标。
//! 2. 查询把行原样还原成 `SignedEvent`，字段一个不改，所以还原后仍能通过验签。
//! 3. `timeline` 按 (display_ts, event_id) 倒序取一页再翻成正序；翻页游标是本页第一条的 (display_ts, event_id)。

use rusqlite::{Connection, OptionalExtension, Params, Row, params};

use crate::crypto::{EventId, MemberId, RoomId};
use crate::error::AppResult;
use crate::protocol::event::{SignedEvent, Stream, UnsignedEvent, kind};

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

/// 会出现在聊天窗口里的 kind：聊天和系统提示。
const TIMELINE_KINDS: [u16; 6] = [
    kind::CHAT,
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

/// 聊天窗口的一页，正序。`before` 为 None 时取最新一页。
pub fn timeline(
    conn: &Connection,
    room: &RoomId,
    before: Option<(u64, EventId)>,
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
            "{SELECT} WHERE room_id = ?1 AND kind IN ({kinds})
             AND (?2 IS NULL OR display_ts < ?2 OR (display_ts = ?2 AND event_id < ?3))
             ORDER BY display_ts DESC, event_id DESC LIMIT ?4"
        ),
        params![room, cursor_ts, cursor_id, limit],
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

/// 未读：他人发的、local_seq 大于已读游标、且不在解散之后的聊天条数。
pub fn count_unread(
    conn: &Connection,
    room: &RoomId,
    me: &MemberId,
    after: i64,
    dissolve_ts: Option<u64>,
) -> AppResult<u32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM events WHERE room_id = ?1 AND kind = ?2 AND author_id != ?3 AND local_seq > ?4
         AND (?5 IS NULL OR ts_ms <= ?5)",
        params![room, kind::CHAT, me, after, dissolve_ts],
        |r| r.get(0),
    )?)
}
