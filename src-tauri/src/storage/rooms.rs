//! 队伍相关表：rooms、members（内部沿用 1.x 的「房间」叫法，界面叫队伍）。
//!
//! 实现流程：
//! 1. rooms 一行一个队伍：队伍码密文、owner_tag、状态，以及从状态事件推导出的缓存（队名、队长、解散时间）。
//! 2. members 保存队伍里出现过的每个人：是否在队、昵称、头像。退出的人 in_room = 0。
//!    2.0 的心之钢层数（stacks、taken 两列）3.0 不再读写，列留着，不改旧库。
//! 3. 缓存都能从 events 重建，权威始终是重放结果。

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::{MemberId, RoomId};
use crate::error::AppResult;

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
    /// 云端分配的 6 位短码；老队伍或离线建的队没有。
    pub short_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub member_id: MemberId,
    pub nickname: Option<String>,
    pub in_room: bool,
    /// CBOR 编码的头像（内置 Preset）。
    pub avatar: Option<Vec<u8>>,
    /// 200×200 大头像的哈希；图片在数据目录 `avatars/`。
    pub avatar_hash: Option<String>,
}

const SELECT_ROOM: &str = "SELECT room_id, code_cipher, owner_tag, status, title, owner_id, dissolve_ts, last_active, short_code FROM rooms";

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
        short_code: r.get(8)?,
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

/// 按最近活跃排序。
pub fn list_rooms(conn: &Connection) -> AppResult<Vec<RoomRow>> {
    let mut stmt = conn.prepare(&format!("{SELECT_ROOM} ORDER BY last_active DESC"))?;
    let rows = stmt.query_map([], read_room)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn set_status(conn: &Connection, room: &RoomId, status: RoomStatus) -> AppResult<()> {
    conn.execute(
        "UPDATE rooms SET status = ?2 WHERE room_id = ?1",
        params![room, status.to_db()],
    )?;
    Ok(())
}

/// 记下云端分配的 6 位短码（建队/进队时写）。
pub fn set_short_code(conn: &Connection, room: &RoomId, code: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE rooms SET short_code = ?2 WHERE room_id = ?1",
        params![room, code],
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

/// 删除一个队伍在本机的全部记录。由调用方放进事务。
pub fn delete_room(conn: &Connection, room: &RoomId) -> AppResult<()> {
    for table in ["rooms", "members", "events"] {
        conn.execute(&format!("DELETE FROM {table} WHERE room_id = ?1"), [room])?;
    }
    Ok(())
}

/// 按推导结果刷新成员：先全部标为不在队，再把当前成员标回在队；昵称和头像按推导结果覆盖
/// （有 Profile 的人头像以 `avatars` 为准，不在里面的清空）。
pub fn set_members(
    conn: &Connection,
    room: &RoomId,
    in_room: &BTreeSet<MemberId>,
    nicknames: &BTreeMap<MemberId, String>,
    avatars: &BTreeMap<MemberId, Vec<u8>>,
    avatar_hashes: &BTreeMap<MemberId, String>,
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
            "INSERT INTO members (room_id, member_id, nickname, avatar, avatar_hash) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (room_id, member_id) DO UPDATE SET nickname = excluded.nickname, avatar = excluded.avatar, avatar_hash = excluded.avatar_hash",
            params![room, m, nick, avatars.get(m), avatar_hashes.get(m)],
        )?;
    }
    Ok(())
}

pub fn members(conn: &Connection, room: &RoomId) -> AppResult<Vec<MemberRow>> {
    let mut stmt = conn.prepare(
        "SELECT member_id, nickname, in_room, avatar, avatar_hash FROM members WHERE room_id = ?1 ORDER BY member_id",
    )?;
    let rows = stmt.query_map([room], |r| {
        Ok(MemberRow {
            member_id: r.get(0)?,
            nickname: r.get(1)?,
            in_room: r.get(2)?,
            avatar: r.get(3)?,
            avatar_hash: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
