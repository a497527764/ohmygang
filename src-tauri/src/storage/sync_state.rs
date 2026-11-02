//! 聊天补齐分界（架构 7.3、7.4）：每个房间、每位作者从第几条聊天开始补。作者用短 ID（公钥前 8 字节）。

use std::collections::HashMap;

use rusqlite::{Connection, params};

use crate::crypto::RoomId;
use crate::error::AppResult;
use crate::protocol::frame::ShortId;

pub fn floors(conn: &Connection, room: &RoomId) -> AppResult<HashMap<ShortId, u64>> {
    let mut stmt = conn.prepare("SELECT author, chat_floor FROM sync_state WHERE room_id = ?1")?;
    let rows = stmt.query_map([room], |r| Ok((r.get::<_, ShortId>(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
}

pub fn set_floor(conn: &Connection, room: &RoomId, author: &ShortId, floor: u64) -> AppResult<()> {
    conn.execute(
        "INSERT INTO sync_state (room_id, author, chat_floor) VALUES (?1, ?2, ?3)
         ON CONFLICT (room_id, author) DO UPDATE SET chat_floor = excluded.chat_floor",
        params![room, author, floor],
    )?;
    Ok(())
}
