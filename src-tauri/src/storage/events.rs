//! 事件表读写。
//!
//! 实现流程：
//! 1. `put_event`：event_id 已存在就返回 Duplicate；同一作者、同一 seq 已有别的 event_id，
//!    说明出现分叉，两条都标记 is_fork 并返回 Fork；否则正常插入。插入时数据库分配自增的 local_seq（本机收到的先后）。
//! 2. 查询把行原样还原成 `SignedEvent`，字段一个不改，所以还原后仍能通过验签。
//! 3. `append_own`：本人写新事件的唯一入口，取下一个 seq 和严格递增的 ts，签名后入库，投递状态为 Pending。
//! 4. 2.0 只有状态事件：1.x 的聊天时间线、未读、预览、搜索、清空记录都去掉了。

use rusqlite::{Connection, OptionalExtension, Params, Row, params};

use crate::crypto::{EventId, Identity, MemberId, RoomId};
use crate::error::AppResult;
use crate::protocol::event::{SignedEvent, UnsignedEvent, build_event, next_ts};

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
    pub delivery: Option<Delivery>,
    pub is_fork: bool,
}

const SELECT: &str = "SELECT local_seq, event_id, room_id, author_id, format, seq, ts_ms, kind, body, sig, \
                      delivery, is_fork FROM events";

fn read(row: &Row) -> rusqlite::Result<StoredEvent> {
    let delivery = match row.get::<_, Option<u8>>(10)? {
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
                seq: row.get(5)?,
                ts_ms: row.get(6)?,
                kind: row.get(7)?,
                body: row.get(8)?,
            },
            sig: row.get(9)?,
        },
        delivery,
        is_fork: row.get(11)?,
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
    let slot = params![ev.room_id, ev.author_id, ev.seq];
    let others: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE room_id = ?1 AND author_id = ?2 AND seq = ?3",
        slot,
        |r| r.get(0),
    )?;
    let fork = others > 0;
    if fork {
        conn.execute(
            "UPDATE events SET is_fork = 1 WHERE room_id = ?1 AND author_id = ?2 AND seq = ?3",
            slot,
        )?;
    }
    conn.execute(
        "INSERT INTO events (event_id, room_id, author_id, format, seq, ts_ms, kind, body, sig, delivery, is_fork)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            e.event_id,
            ev.room_id,
            ev.author_id,
            ev.format,
            ev.seq,
            ev.ts_ms,
            ev.kind,
            ev.body,
            e.sig,
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

/// 某作者的全部事件，按 (seq, event_id) 升序。
pub fn query_by_author(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!("{SELECT} WHERE room_id = ?1 AND author_id = ?2 ORDER BY seq, event_id"),
        params![room, author],
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

/// 队伍里的全部事件，按本机入库顺序。
pub fn query_room(conn: &Connection, room: &RoomId) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!("{SELECT} WHERE room_id = ?1 ORDER BY local_seq"),
        [room],
    )
}

/// 队伍里某 kind 的全部事件，按本机入库顺序。
pub fn query_kind(conn: &Connection, room: &RoomId, k: u16) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!("{SELECT} WHERE room_id = ?1 AND kind = ?2 ORDER BY local_seq"),
        params![room, k],
    )
}

pub fn next_seq(conn: &Connection, room: &RoomId, author: &MemberId) -> AppResult<u64> {
    let max: Option<u64> = conn.query_row(
        "SELECT MAX(seq) FROM events WHERE room_id = ?1 AND author_id = ?2",
        params![room, author],
        |r| r.get(0),
    )?;
    Ok(max.unwrap_or(0) + 1)
}

/// 本人在该队伍最后一条事件的 ts_ms，用于保证时间严格递增。
pub fn last_ts(conn: &Connection, room: &RoomId, author: &MemberId) -> AppResult<Option<u64>> {
    Ok(conn.query_row(
        "SELECT MAX(ts_ms) FROM events WHERE room_id = ?1 AND author_id = ?2",
        params![room, author],
        |r| r.get(0),
    )?)
}

/// 本人在某队伍写一条新事件并入库，返回签好名的事件。
pub fn append_own(
    conn: &Connection,
    me: &Identity,
    room: &RoomId,
    k: u16,
    body: Vec<u8>,
    now_ms: u64,
) -> AppResult<SignedEvent> {
    let author = me.member_id();
    let seq = next_seq(conn, room, &author)?;
    let ts = next_ts(now_ms, last_ts(conn, room, &author)?);
    let e = build_event(me, *room, seq, ts, k, body);
    put_event(conn, &e, Some(Delivery::Pending))?;
    Ok(e)
}

/// 本人 seq 不超过 `upto` 且仍是 Pending 的事件改为 Sent，返回改了的事件 ID。
pub fn mark_sent(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
    upto: u64,
) -> AppResult<Vec<EventId>> {
    let mut stmt = conn.prepare(
        "UPDATE events SET delivery = 1 WHERE room_id = ?1 AND author_id = ?2 AND seq <= ?3 AND delivery = 0 RETURNING event_id",
    )?;
    let rows = stmt.query_map(params![room, author, upto], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
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

/// 队伍里每条事件的 (作者, seq)，用来算持有向量。
pub fn holdings_rows(conn: &Connection, room: &RoomId) -> AppResult<Vec<(MemberId, u64)>> {
    let mut stmt = conn.prepare("SELECT author_id, seq FROM events WHERE room_id = ?1")?;
    let rows = stmt.query_map([room], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 队伍里出现过的所有作者。
pub fn room_authors(conn: &Connection, room: &RoomId) -> AppResult<Vec<MemberId>> {
    let mut stmt = conn.prepare("SELECT DISTINCT author_id FROM events WHERE room_id = ?1")?;
    let rows = stmt.query_map([room], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 某作者序号在 [from, to] 之间的事件（含分叉），按 (seq, event_id) 升序。
pub fn query_range(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
    from: u64,
    to: u64,
) -> AppResult<Vec<StoredEvent>> {
    collect(
        conn,
        &format!(
            "{SELECT} WHERE room_id = ?1 AND author_id = ?2 AND seq BETWEEN ?3 AND ?4 ORDER BY seq, event_id"
        ),
        params![room, author, from, to],
    )
}
