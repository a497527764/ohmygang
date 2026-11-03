//! 房间相关表：rooms、room_rules、members、read_state。
//!
//! 实现流程：
//! 1. rooms 一行一个房间：房间码密文、owner_tag、状态，以及从状态事件推导出的缓存（群名、房主、解散时间）。
//! 2. room_rules 保存全部规则版本；members 保存房间里出现过的每个人：是否在房、昵称、层数缓存。
//!    退出房间的人 in_room = 0，但层数保留（需求 3.2：层数保留，排行榜只列当前成员）。
//! 3. 缓存都能从 events 重建，权威始终是重放结果（架构 9.2）。

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::{MemberId, RoomId};
use crate::error::AppResult;
use crate::heartsteel::Rules;
use crate::protocol::body::Rule;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomStatus {
    Joining,
    Active,
    Left,
    Dissolved,
}

impl RoomStatus {
    fn to_db(self) -> u8 {
        self as u8
    }

    fn from_db(v: u8) -> Self {
        match v {
            0 => Self::Joining,
            1 => Self::Active,
            2 => Self::Left,
            _ => Self::Dissolved,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomRow {
    pub room_id: RoomId,
    pub code_cipher: Vec<u8>,
    pub owner_tag: [u8; 10],
    pub status: RoomStatus,
    pub title: Option<String>,
    pub owner_id: Option<MemberId>,
    pub dissolve_ts: Option<u64>,
    pub last_active: u64,
    pub muted: bool,
    pub pinned: bool,
    /// 本机清空聊天记录时的最大入库序号和时刻（规则见 chat.rs）。
    pub cleared_seq: Option<i64>,
    pub cleared_ts: Option<u64>,
    /// 在列表里删除了聊天，暂时不显示。
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub member_id: MemberId,
    pub nickname: Option<String>,
    pub in_room: bool,
    pub stacks: u32,
    /// CBOR 编码的头像。
    pub avatar: Option<Vec<u8>>,
}

const SELECT_ROOM: &str = "SELECT room_id, code_cipher, owner_tag, status, title, owner_id, dissolve_ts, last_active, muted, pinned, cleared_seq, cleared_ts, hidden FROM rooms";

fn read_room(r: &rusqlite::Row) -> rusqlite::Result<RoomRow> {
    Ok(RoomRow {
        room_id: r.get(0)?,
        code_cipher: r.get(1)?,
        owner_tag: r.get(2)?,
        status: RoomStatus::from_db(r.get(3)?),
        title: r.get(4)?,
        owner_id: r.get(5)?,
        dissolve_ts: r.get(6)?,
        last_active: r.get(7)?,
        muted: r.get(8)?,
        pinned: r.get(9)?,
        cleared_seq: r.get(10)?,
        cleared_ts: r.get(11)?,
        hidden: r.get(12)?,
    })
}

/// 已存在同一 room_id 时不改动，返回 false。
pub fn insert_room(
    conn: &Connection,
    room: &RoomId,
    code_cipher: &[u8],
    owner_tag: &[u8; 10],
    status: RoomStatus,
    now_ms: u64,
) -> AppResult<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO rooms (room_id, code_cipher, owner_tag, status, last_active) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![room, code_cipher, owner_tag, status.to_db(), now_ms],
    )?;
    Ok(n == 1)
}

pub fn get_room(conn: &Connection, room: &RoomId) -> AppResult<Option<RoomRow>> {
    Ok(conn
        .query_row(
            &format!("{SELECT_ROOM} WHERE room_id = ?1"),
            [room],
            read_room,
        )
        .optional()?)
}

/// 置顶在前，其余按最近活跃排序。
pub fn list_rooms(conn: &Connection) -> AppResult<Vec<RoomRow>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT_ROOM} ORDER BY pinned DESC, last_active DESC"
    ))?;
    let rows = stmt.query_map([], read_room)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 记下清空聊天记录时的入库序号和时刻。
pub fn set_cleared(conn: &Connection, room: &RoomId, seq: i64, ts: u64) -> AppResult<()> {
    conn.execute(
        "UPDATE rooms SET cleared_seq = ?2, cleared_ts = ?3 WHERE room_id = ?1",
        params![room, seq, ts],
    )?;
    Ok(())
}

pub fn set_hidden(conn: &Connection, room: &RoomId, hidden: bool) -> AppResult<()> {
    conn.execute(
        "UPDATE rooms SET hidden = ?2 WHERE room_id = ?1",
        params![room, hidden],
    )?;
    Ok(())
}

/// 某成员在本房间的昵称缓存。
pub fn nickname_of(
    conn: &Connection,
    room: &RoomId,
    member: &MemberId,
) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT nickname FROM members WHERE room_id = ?1 AND member_id = ?2",
            params![room, member],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

pub fn set_status(conn: &Connection, room: &RoomId, status: RoomStatus) -> AppResult<()> {
    conn.execute(
        "UPDATE rooms SET status = ?2 WHERE room_id = ?1",
        params![room, status.to_db()],
    )?;
    Ok(())
}

pub fn update_cache(
    conn: &Connection,
    room: &RoomId,
    title: Option<&str>,
    owner: Option<&MemberId>,
    dissolve_ts: Option<u64>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE rooms SET title = ?2, owner_id = ?3, dissolve_ts = ?4 WHERE room_id = ?1",
        params![room, title, owner, dissolve_ts],
    )?;
    Ok(())
}

/// 本机偏好（免打扰、置顶），`None` 表示不改。房间不存在时返回 false。
pub fn set_prefs(
    conn: &Connection,
    room: &RoomId,
    muted: Option<bool>,
    pinned: Option<bool>,
) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE rooms SET muted = COALESCE(?2, muted), pinned = COALESCE(?3, pinned) WHERE room_id = ?1",
        params![room, muted, pinned],
    )?;
    Ok(n > 0)
}

/// 删除一个房间在本机的全部记录（加入中取消时用，架构 8.2）。由调用方放进事务。
pub fn delete_room(conn: &Connection, room: &RoomId) -> AppResult<()> {
    for table in [
        "rooms",
        "room_rules",
        "members",
        "events",
        "read_state",
        "sync_state",
    ] {
        conn.execute(&format!("DELETE FROM {table} WHERE room_id = ?1"), [room])?;
    }
    Ok(())
}

/// 最近活跃时间只往后推。
pub fn touch(conn: &Connection, room: &RoomId, ts_ms: u64) -> AppResult<()> {
    conn.execute(
        "UPDATE rooms SET last_active = MAX(last_active, ?2) WHERE room_id = ?1",
        params![room, ts_ms],
    )?;
    Ok(())
}

pub fn replace_rules(conn: &Connection, room: &RoomId, rules: &Rules) -> AppResult<()> {
    conn.execute("DELETE FROM room_rules WHERE room_id = ?1", [room])?;
    for r in rules.values() {
        conn.execute(
            "INSERT INTO room_rules (room_id, rule_epoch, window_ms, threshold, cooldown_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![room, r.rule_epoch, r.window_ms, r.threshold, r.cooldown_ms],
        )?;
    }
    Ok(())
}

pub fn get_rules(conn: &Connection, room: &RoomId) -> AppResult<Rules> {
    let mut stmt = conn.prepare(
        "SELECT rule_epoch, window_ms, threshold, cooldown_ms FROM room_rules WHERE room_id = ?1",
    )?;
    let rows = stmt.query_map([room], |r| {
        Ok(Rule {
            rule_epoch: r.get(0)?,
            window_ms: r.get(1)?,
            threshold: r.get(2)?,
            cooldown_ms: r.get(3)?,
        })
    })?;
    let mut rules = Rules::new();
    for r in rows {
        let r = r?;
        rules.insert(r.rule_epoch, r);
    }
    Ok(rules)
}

/// 按推导结果刷新成员：先全部标为不在房，再把当前成员标回在房；昵称和头像按推导结果覆盖
/// （有 Profile 的人头像以 `avatars` 为准，不在里面的清空）。层数不动。
pub fn set_members(
    conn: &Connection,
    room: &RoomId,
    in_room: &BTreeSet<MemberId>,
    nicknames: &BTreeMap<MemberId, String>,
    avatars: &BTreeMap<MemberId, Vec<u8>>,
) -> AppResult<()> {
    conn.execute("UPDATE members SET in_room = 0 WHERE room_id = ?1", [room])?;
    for m in in_room {
        conn.execute(
            "INSERT INTO members (room_id, member_id, in_room) VALUES (?1, ?2, 1)
             ON CONFLICT (room_id, member_id) DO UPDATE SET in_room = 1",
            params![room, m],
        )?;
    }
    for (m, nick) in nicknames {
        conn.execute(
            "INSERT INTO members (room_id, member_id, nickname, avatar) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (room_id, member_id) DO UPDATE SET nickname = excluded.nickname, avatar = excluded.avatar",
            params![room, m, nick, avatars.get(m)],
        )?;
    }
    Ok(())
}

pub fn set_stacks(
    conn: &Connection,
    room: &RoomId,
    member: &MemberId,
    stacks: u32,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO members (room_id, member_id, stacks) VALUES (?1, ?2, ?3)
         ON CONFLICT (room_id, member_id) DO UPDATE SET stacks = excluded.stacks",
        params![room, member, stacks],
    )?;
    Ok(())
}

pub fn stacks_of(conn: &Connection, room: &RoomId, member: &MemberId) -> AppResult<u32> {
    let s: Option<u32> = conn
        .query_row(
            "SELECT stacks FROM members WHERE room_id = ?1 AND member_id = ?2",
            params![room, member],
            |r| r.get(0),
        )
        .optional()?;
    Ok(s.unwrap_or(0))
}

/// 按层数降序。
pub fn members(conn: &Connection, room: &RoomId) -> AppResult<Vec<MemberRow>> {
    let mut stmt = conn.prepare(
        "SELECT member_id, nickname, in_room, stacks, avatar FROM members WHERE room_id = ?1 ORDER BY stacks DESC, member_id",
    )?;
    let rows = stmt.query_map([room], |r| {
        Ok(MemberRow {
            member_id: r.get(0)?,
            nickname: r.get(1)?,
            in_room: r.get(2)?,
            stacks: r.get(3)?,
            avatar: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn read_cursor(conn: &Connection, room: &RoomId) -> AppResult<i64> {
    let c: Option<i64> = conn
        .query_row(
            "SELECT read_local_seq FROM read_state WHERE room_id = ?1",
            [room],
            |r| r.get(0),
        )
        .optional()?;
    Ok(c.unwrap_or(0))
}

pub fn set_read_cursor(conn: &Connection, room: &RoomId, local_seq: i64) -> AppResult<()> {
    conn.execute(
        "INSERT INTO read_state (room_id, read_local_seq) VALUES (?1, ?2)
         ON CONFLICT (room_id) DO UPDATE SET read_local_seq = excluded.read_local_seq",
        params![room, local_seq],
    )?;
    Ok(())
}
