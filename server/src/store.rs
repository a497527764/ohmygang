//! SQLite 持久化：队伍（短码 ↔ room_id 映射）、成员在线、回收后的黑名单。
//!
//! 队伍：
//! - `teams`：short_code 主键（6 位短码），room_id 唯一（回收后删除，短码可被新队伍复用）。
//! - `members`：每个房间最近一次心跳的成员及其时间。
//! - `blacklist`：已被回收的 room_id，中继直接拒收，不再转发。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::protocol;

const HOUR: i64 = 3600;
const DAY: i64 = 86400;

pub struct TeamRow {
    pub room_id_hex: String,
    pub last_active: i64,
    pub low_since: Option<i64>,
}

/// 建队额度的四层限制。
#[derive(Clone, Copy, Debug)]
pub struct CreateLimits {
    /// 单台设备每小时最多建几支。
    pub device_hour: u32,
    /// 单台设备每天最多建几支。
    pub device_day: u32,
    /// 整个服务器每小时最多建几支。
    pub global_hour: u32,
    /// 整个服务器每天最多建几支。
    pub global_day: u32,
}

/// 建队配额用完了没有，以及是哪一层用完的。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quota {
    /// 还有额度。
    Ok,
    /// 这台设备（按 member_id 认设备）超了：换 IP 也没用。
    Device,
    /// 整个服务器超了：防脚本狂刷的底线。
    Global,
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
                low_since  INTEGER,
                code_expires_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS members (
                room_id   TEXT NOT NULL,
                member_hex TEXT NOT NULL,
                last_seen INTEGER NOT NULL,
                PRIMARY KEY (room_id, member_hex)
            );
            CREATE TABLE IF NOT EXISTS blacklist (
                room_id TEXT PRIMARY KEY,
                created_at INTEGER
            );
            -- 建队配额流水：按设备（member_id）记，同时留一份 IP 便于排查。
            -- 单独记账而不是数 teams 表：队伍被回收后 teams 行会消失，
            -- 那样「建了再删」就能把额度退回来，等于留了个绕过后门。
            CREATE TABLE IF NOT EXISTS create_events (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at INTEGER NOT NULL,
                peer       TEXT,
                device     TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_create_events_at ON create_events(created_at);
            CREATE INDEX IF NOT EXISTS idx_create_events_device ON create_events(device, created_at);
            -- 回收下来的短码池：建队优先从这里取，短码就不会被白白浪费掉。
            CREATE TABLE IF NOT EXISTS free_codes (
                code        TEXT PRIMARY KEY,
                released_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_free_codes_at ON free_codes(released_at);",
        )?;
        // 老库补列（每次启动都查一下，缺哪列补哪列）。
        let has = |table: &str, col: &str| -> rusqlite::Result<bool> {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let cols = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<Result<Vec<String>, _>>()?;
            Ok(cols.iter().any(|c| c == col))
        };
        // 队名存服务器：中继的事件缓存是内存的，服务器一重启，队名就找不回来了。
        if !has("teams", "title")? {
            let _ = conn.execute("ALTER TABLE teams ADD COLUMN title TEXT", []);
        }
        // 短码到期时间：给短码一个独立的寿命，不用队伍解散来顺带释放。
        if !has("teams", "code_expires_at")? {
            let _ = conn.execute("ALTER TABLE teams ADD COLUMN code_expires_at INTEGER", []);
        }
        // 配额流水里的设备号：设备维度的限额靠它统计。
        if !has("create_events", "device")? {
            let _ = conn.execute("ALTER TABLE create_events ADD COLUMN device TEXT", []);
        }
        // 黑名单的记录时间：黑名单不能只增不减，过老的要清掉。
        if !has("blacklist", "created_at")? {
            let _ = conn.execute("ALTER TABLE blacklist ADD COLUMN created_at INTEGER", []);
        }
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    /// 新建队伍：写入短码、room_secret/owner_tag、room_id，并把创建者记为第一名成员。
    #[allow(clippy::too_many_arguments)]
    pub fn create_team(
        &self,
        short: &str,
        secret: &[u8; 16],
        owner_tag: &[u8; 10],
        room_id_hex: &str,
        now: i64,
        creator_hex: &str,
        title: Option<&str>,
        code_expires_at: i64,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute(
            "INSERT INTO teams (short_code, room_secret, owner_tag, room_id, created_at, last_active, low_since, title, code_expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8)",
            params![short, secret, owner_tag, room_id_hex, now, now, title, code_expires_at],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO members (room_id, member_hex, last_seen) VALUES (?1, ?2, ?3)",
            params![room_id_hex, creator_hex, now],
        )?;
        Ok(())
    }

    /// 预留一次建队配额。两层限制：
    /// - 设备层（member_id）：一小时 `device_hour` 支、24 小时 `device_day` 支。
    ///   认设备而不是认 IP——换个网络、换个梯子出口，额度还是你的。
    /// - 全局层：所有设备合计一小时 `global_hour` 支、24 小时 `global_day` 支。
    ///   设备层挡不住「每次换一个 member_id」的脚本，全局层才是防攻击的底线。
    ///
    /// 检查和记账在同一把 SQLite 锁里完成，并发建队不会超发。
    /// 额度一旦预留就记账，即使后面建队失败也不退——防的是「故意失败来试探额度」。
    pub fn reserve_create(
        &self,
        now: i64,
        peer: &str,
        device: &str,
        limits: CreateLimits,
    ) -> rusqlite::Result<Quota> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let count = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<i64> {
            conn.query_row(sql, p, |r| r.get(0))
        };
        // 设备层
        let dev_h = count(
            "SELECT COUNT(*) FROM create_events WHERE device = ?1 AND created_at >= ?2",
            &[&device, &(now - HOUR)],
        )?;
        if dev_h >= limits.device_hour as i64 {
            return Ok(Quota::Device);
        }
        let dev_d = count(
            "SELECT COUNT(*) FROM create_events WHERE device = ?1 AND created_at >= ?2",
            &[&device, &(now - DAY)],
        )?;
        if dev_d >= limits.device_day as i64 {
            return Ok(Quota::Device);
        }
        // 全局层
        let glob_h = count(
            "SELECT COUNT(*) FROM create_events WHERE created_at >= ?1",
            &[&(now - HOUR)],
        )?;
        if glob_h >= limits.global_hour as i64 {
            return Ok(Quota::Global);
        }
        let glob_d = count(
            "SELECT COUNT(*) FROM create_events WHERE created_at >= ?1",
            &[&(now - DAY)],
        )?;
        if glob_d >= limits.global_day as i64 {
            return Ok(Quota::Global);
        }
        conn.execute(
            "INSERT INTO create_events (created_at, peer, device) VALUES (?1, ?2, ?3)",
            params![now, peer, device],
        )?;
        Ok(Quota::Ok)
    }

    /// 清掉 24 小时前的建队流水（配额只看最近 24 小时，更早的留着只会让表无限长）。
    pub fn prune_create_events(&self, before: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute(
            "DELETE FROM create_events WHERE created_at < ?1",
            params![before],
        )?;
        Ok(())
    }

    // ---------- 短码的回收与复用 ----------

    /// 回收一支队伍在服务器上的登记：删掉队伍与成员记录，短码进池子等着复用。
    ///
    /// 和 `dissolve` 的区别是**不进黑名单**——队伍在中继上还能互相发消息，
    /// 只是服务器不再发这个短码了（别人拿旧码来会得到「已过期」）。
    /// 返回被回收的短码。
    pub fn release_code(&self, room_id_hex: &str, now: i64) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let short: Option<String> = conn
            .query_row(
                "SELECT short_code FROM teams WHERE room_id = ?1",
                [room_id_hex],
                |r| r.get(0),
            )
            .optional()?;
        let Some(short) = short else {
            return Ok(None);
        };
        conn.execute("DELETE FROM teams WHERE room_id = ?1", [room_id_hex])?;
        conn.execute("DELETE FROM members WHERE room_id = ?1", [room_id_hex])?;
        conn.execute(
            "INSERT OR IGNORE INTO free_codes (code, released_at) VALUES (?1, ?2)",
            params![short, now],
        )?;
        Ok(Some(short))
    }

    /// 从池子里取一个可复用的短码：冷却期已过的、按回收先后排队取最早的。
    ///
    /// 冷却期（默认 7 天）是防「刚过期的码马上被别人捡走」——那会让拿着旧码的
    /// 队友稀里糊涂进到一支新队伍里。过了冷却期，码空间大得很，不怕撞。
    pub fn take_free_code(&self, now: i64, cooldown: i64) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let code: Option<String> = conn
            .query_row(
                "SELECT code FROM free_codes WHERE released_at <= ?1 ORDER BY released_at LIMIT 1",
                params![now - cooldown],
                |r| r.get(0),
            )
            .optional()?;
        let Some(code) = code else {
            return Ok(None);
        };
        conn.execute("DELETE FROM free_codes WHERE code = ?1", [&code])?;
        Ok(Some(code))
    }

    /// 短码池留太多没意义：只留最近释放的 `keep` 个，剩下的丢掉。
    pub fn prune_free_codes(&self, keep: usize) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM free_codes", [], |r| r.get(0))?;
        if total as usize <= keep {
            return Ok(());
        }
        let drop = total as usize - keep;
        conn.execute(
            "DELETE FROM free_codes WHERE code IN (
                 SELECT code FROM free_codes ORDER BY released_at LIMIT ?1
             )",
            params![drop as i64],
        )?;
        Ok(())
    }

    /// 短码到期的队伍（房间、短码）。
    pub fn expired_codes(&self, now: i64) -> rusqlite::Result<Vec<(String, String)>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn.prepare(
            "SELECT room_id, short_code FROM teams
             WHERE code_expires_at IS NOT NULL AND code_expires_at <= ?1",
        )?;
        let rows = stmt.query_map(params![now], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()
    }

    /// 给短码续期：队伍还在真用着，短码就继续有效。
    pub fn extend_code_expiry(&self, room_id_hex: &str, expires_at: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute(
            "UPDATE teams SET code_expires_at = ?2 WHERE room_id = ?1",
            params![room_id_hex, expires_at],
        )?;
        Ok(())
    }

    /// 最近 `since` 秒内报过心跳的不同成员数：判断这支队伍是不是「真有人在用」。
    pub fn recent_member_count(&self, room_id_hex: &str, since: i64) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.query_row(
            "SELECT COUNT(DISTINCT member_hex) FROM members WHERE room_id = ?1 AND last_seen >= ?2",
            params![room_id_hex, since],
            |r| r.get::<_, i64>(0).map(|n| n as usize),
        )
    }

    /// 黑名单不能只增不减：清掉 `before` 之前的记录。
    /// room_id 是 32 字节随机值，留几十天足够挡住「拿旧房间复活」这类操作。
    pub fn prune_blacklist(&self, before: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute(
            "DELETE FROM blacklist WHERE created_at IS NOT NULL AND created_at < ?1",
            params![before],
        )?;
        Ok(())
    }

    pub fn team_exists(&self, short: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        Ok(conn
            .query_row("SELECT 1 FROM teams WHERE short_code = ?1", [short], |_| {
                Ok(())
            })
            .optional()?
            .is_some())
    }

    /// 用短码取回 GT2 长码、room_id 和队名（进队时用）。
    pub fn team_by_short(
        &self,
        short: &str,
    ) -> rusqlite::Result<Option<(String, String, Option<String>)>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let row: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT room_id, title FROM teams WHERE short_code = ?1",
                [short],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((room_id_hex, title)) = row else {
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
        Ok(Some((protocol::format_code(&s, &o), room_id_hex, title)))
    }

    /// 改队名（房主改名后同步过来）。
    pub fn set_team_title(&self, room_id_hex: &str, title: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.execute(
            "UPDATE teams SET title = ?2 WHERE room_id = ?1",
            params![room_id_hex, title],
        )?;
        Ok(())
    }

    /// 用 room_id 反查短码（客户端重启后想拿回短码展示时用）。
    pub fn short_by_room(&self, room_id_hex: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.query_row(
            "SELECT short_code FROM teams WHERE room_id = ?1",
            [room_id_hex],
            |r| r.get(0),
        )
        .optional()
    }

    pub fn list_teams(&self) -> rusqlite::Result<Vec<TeamRow>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn.prepare("SELECT room_id, last_active, low_since FROM teams")?;
        let rows = stmt.query_map([], |r| {
            Ok(TeamRow {
                room_id_hex: r.get(0)?,
                last_active: r.get(1)?,
                low_since: r.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
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

    pub fn upsert_member(
        &self,
        room_id_hex: &str,
        member_hex: &str,
        last_seen: i64,
    ) -> rusqlite::Result<()> {
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
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out: HashMap<String, HashMap<String, i64>> = HashMap::new();
        for r in rows {
            let (room, member, seen) = r?;
            out.entry(room).or_default().insert(member, seen);
        }
        Ok(out)
    }

    /// 一段时间内报过心跳的成员数（客户端给主页「加入过的队伍」显示 在线/总数）。
    pub fn room_online(&self, room_id_hex: &str, since: i64) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        conn.query_row(
            "SELECT COUNT(*) FROM members WHERE room_id = ?1 AND last_seen >= ?2",
            params![room_id_hex, since],
            |r| r.get::<_, i64>(0).map(|n| n as usize),
        )
    }

    pub fn load_activity(&self) -> rusqlite::Result<HashMap<String, i64>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn.prepare("SELECT room_id, last_active FROM teams")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        rows.collect::<Result<HashMap<_, _>, _>>()
    }

    pub fn load_blacklist(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        let conn = self.conn.lock().expect("SQLite 锁没有中毒");
        let mut stmt = conn.prepare("SELECT room_id, created_at FROM blacklist")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?.unwrap_or(0),
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    }

    /// 回收一支队伍：删掉队伍与成员，room_id 进黑名单，短码进池子等着复用。
    pub fn dissolve(&self, room_id_hex: &str) -> rusqlite::Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        {
            let conn = self.conn.lock().expect("SQLite 锁没有中毒");
            conn.execute(
                "INSERT OR IGNORE INTO blacklist (room_id, created_at) VALUES (?1, ?2)",
                params![room_id_hex, now],
            )?;
        }
        // 短码别浪费：回收下来进池子，以后建队优先复用。
        // （锁要放掉再调 release_code——SQLite 锁不可重入。）
        self.release_code(room_id_hex, now)?;
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
            "SELECT short_code, room_id, created_at, last_active, room_secret, owner_tag, title
             FROM teams ORDER BY created_at DESC",
        )?;
        let teams: Vec<AdminTeamRow> = stmt
            .query_map([], |r| {
                let secret: Vec<u8> = r.get(4)?;
                let owner: Vec<u8> = r.get(5)?;
                let mut s = [0u8; 16];
                let mut o = [0u8; 10];
                if secret.len() == 16 && owner.len() == 10 {
                    s.copy_from_slice(&secret);
                    o.copy_from_slice(&owner);
                }
                Ok(AdminTeamRow {
                    short_code: r.get(0)?,
                    room_id_hex: r.get(1)?,
                    created_at: r.get(2)?,
                    last_active: r.get(3)?,
                    code: protocol::format_code(&s, &o),
                    title: r.get(6)?,
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
    /// 完整的 GT2 邀请码（管理员复制分享用）。
    pub code: String,
    /// 队名（房主起的名字）。
    pub title: Option<String>,
    pub members: Vec<AdminMember>,
}

/// 管理后台的成员行。
pub struct AdminMember {
    pub member_hex: String,
    pub last_seen: i64,
}
