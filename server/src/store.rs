//! SQLite 持久化：队伍（短码 ↔ room_id 映射）、成员在线、回收后的黑名单。
//!
//! 队伍：
//! - `teams`：short_code 主键（6 位短码），room_id 唯一（回收后删除，短码可被新队伍复用）。
//! - `members`：每个房间最近一次心跳的成员及其时间。
//! - `blacklist`：已被回收的 room_id，中继直接拒收，不再转发。

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::protocol;

pub struct TeamRow {
    pub room_id_hex: String,
    pub last_active: i64,
    pub low_since: Option<i64>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS teams (
                short_code TEXT PRIMARY KEY,
                room_secret BLOB NOT NULL,
                owner_tag  BLOB NOT NULL,
                room_id   TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL,
                last_active INTEGER NOT NULL,
                low_since  INTEGER
            );
            CREATE TABLE IF NOT EXISTS members (
                room_id   TEXT NOT NULL,
                member_hex TEXT NOT NULL,
                last_seen INTEGER NOT NULL,
                PRIMARY KEY (room_id, member_hex)
            );
            CREATE TABLE IF NOT EXISTS blacklist (
                room_id TEXT PRIMARY KEY
            );",
        )?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    /// 新建队伍：写入短码、room_secret/owner_tag、room_id，并把创建者记为第一名成员。
    pub fn create_team(
        &self,
        short: &str,
        secret: &[u8; 16],
        owner_tag: &[u8; 10],
        room_id_hex: &str,
        now: i64,
        creator_hex: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute(
            "INSERT INTO teams (short_code, room_secret, owner_tag, room_id, created_at, last_active, low_since)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![short, secret, owner_tag, room_id_hex, now, now],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO members (room_id, member_hex, last_seen) VALUES (?1, ?2, ?3)",
            params![room_id_hex, creator_hex, now],
        )?;
        Ok(())
    }

    pub fn team_exists(&self, short: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        Ok(conn
            .query_row(
                "SELECT 1 FROM teams WHERE short_code = ?1",
                [short],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// 用短码取回 GT2 长码和 room_id（进队时用）。
    pub fn team_by_short(&self, short: &str) -> rusqlite::Result<Option<(String, String)>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let room_id_hex: Option<String> = conn
            .query_row(
                "SELECT room_id FROM teams WHERE short_code = ?1",
                [short],
                |r| r.get(0),
            )
            .optional()?;
        let Some(room_id_hex) = room_id_hex else {
            return Ok(None);
        };
        let secret: Vec<u8> = conn.query_row(
            "SELECT room_secret FROM teams WHERE room_id = ?1",
            [&room_id_hex],
            |r| r.get(0),
        )?;
        let owner: Vec<u8> = conn.query_row(
            "SELECT owner_tag FROM teams WHERE room_id = ?1",
            [&room_id_hex],
            |r| r.get(0),
        )?;
        let mut s = [0u8; 16];
        let mut o = [0u8; 10];
        s.copy_from_slice(&secret);
        o.copy_from_slice(&owner);
        Ok(Some((protocol::format_code(&s, &o), room_id_hex)))
    }

    /// 用 room_id 反查短码（客户端重启后想拿回短码展示时用）。
    pub fn short_by_room(&self, room_id_hex: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        Ok(conn
            .query_row(
                "SELECT short_code FROM teams WHERE room_id = ?1",
                [room_id_hex],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn list_teams(&self) -> rusqlite::Result<Vec<TeamRow>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn
            .prepare("SELECT room_id, last_active, low_since FROM teams")?;
        let rows = stmt.query_map([], |r| {
            Ok(TeamRow {
                room_id_hex: r.get(0)?,
                last_active: r.get(1)?,
                low_since: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 回收时或周期刷新时，把内存里的活跃时间 / low_since 落盘。
    pub fn update_team_state(
        &self,
        room_id_hex: &str,
        last_active: i64,
        low_since: Option<i64>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute(
            "UPDATE teams SET last_active = ?2, low_since = ?3 WHERE room_id = ?1",
            params![room_id_hex, last_active, low_since],
        )?;
        Ok(())
    }

    pub fn upsert_member(&self, room_id_hex: &str, member_hex: &str, last_seen: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute(
            "INSERT OR REPLACE INTO members (room_id, member_hex, last_seen) VALUES (?1, ?2, ?3)",
            params![room_id_hex, member_hex, last_seen],
        )?;
        Ok(())
    }

    pub fn load_members(&self) -> rusqlite::Result<HashMap<String, HashMap<String, i64>>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn.prepare("SELECT room_id, member_hex, last_seen FROM members")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })?;
        let mut out: HashMap<String, HashMap<String, i64>> = HashMap::new();
        for r in rows {
            let (room, member, seen) = r?;
            out.entry(room).or_default().insert(member, seen);
        }
        Ok(out)
    }

    pub fn load_activity(&self) -> rusqlite::Result<HashMap<String, i64>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn.prepare("SELECT room_id, last_active FROM teams")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
    }

    pub fn load_blacklist(&self) -> rusqlite::Result<Vec<String>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn.prepare("SELECT room_id FROM blacklist")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 回收一支队伍：删掉队伍与成员，room_id 进黑名单，短码随之释放可复用。
    pub fn dissolve(&self, room_id_hex: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute("INSERT OR IGNORE INTO blacklist (room_id) VALUES (?1)", [room_id_hex])?;
        conn.execute("DELETE FROM teams WHERE room_id = ?1", [room_id_hex])?;
        conn.execute("DELETE FROM members WHERE room_id = ?1", [room_id_hex])?;
        Ok(())
    }

    /// 按短码删除队伍（管理后台用）。
    pub fn dissolve_by_short(&self, short: &str) -> rusqlite::Result<bool> {
        let room: Option<String> = {
            let conn = self.conn.lock().expect("SQLite 锁没有中毒");
            conn.query_row(
                "SELECT room_id FROM teams WHERE short_code = ?1",
                [short],
                |r| r.get(0),
            )
            .optional()?
        };
        match room {
            Some(room_id) => {
                self.dissolve(&room_id)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// 全部队伍 + 每支队伍的成员（管理后台用）。
    pub fn list_all_teams(&self) -> rusqlite::Result<Vec<AdminTeamRow>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn.prepare(
            "SELECT short_code, room_id, created_at, last_active FROM teams ORDER BY created_at DESC",
        )?;
        let teams: Vec<AdminTeamRow> = stmt
            .query_map([], |r| {
                Ok(AdminTeamRow {
                    short_code: r.get(0)?,
                    room_id_hex: r.get(1)?,
                    created_at: r.get(2)?,
                    last_active: r.get(3)?,
                    members: Vec::new(),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        let mut stmt = conn.prepare("SELECT room_id, member_hex, last_seen FROM members")?;
        let member_rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut by_room: std::collections::HashMap<String, Vec<(String, i64)>> =
            std::collections::HashMap::new();
        for row in member_rows {
            let (room, member, seen) = row?;
            by_room.entry(room).or_default().push((member, seen));
        }
        let mut teams = teams;
        for t in &mut teams {
            if let Some(members) = by_room.remove(&t.room_id_hex) {
                t.members = members
                    .into_iter()
                    .map(|(member_hex, last_seen)| AdminMember {
                        member_hex,
                        last_seen,
                    })
                    .collect();
            }
        }
        Ok(teams)
    }
}

/// 管理后台的队伍行。
pub struct AdminTeamRow {
    pub short_code: String,
    pub room_id_hex: String,
    pub created_at: i64,
    pub last_active: i64,
    pub members: Vec<AdminMember>,
}

/// 管理后台的成员行。
pub struct AdminMember {
    pub member_hex: String,
    pub last_seen: i64,
}
