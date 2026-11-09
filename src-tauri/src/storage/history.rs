//! 历史记录表（3.0，计划 3.6）：这台电脑在队伍里时发出和收到的文字、语音、信号、表情。
//!
//! 实现流程：
//! 1. 一行一条：队伍、谁、什么时候、种类、内容（文字、信号 id 或表情引用）、语音（Opus 小帧，每个前面两字节长度）和时长。
//!    按自增 id 排先后（就是这台电脑收到的顺序）。
//! 2. 加一条后修剪：每个队伍只留最新的若干条；所有队伍的语音加起来超过上限时，从最旧的语音开始删整条。
//! 3. 只留现在所在的队伍：别的队伍（退出了、解散了）的记录全部删掉；不在任何队伍里就全删。
//!    解散事件晚到（解散时不在线）也一样：同步到以后就不在这个队伍里了，记录跟着删。
//! 4. 不和队友补齐：每台电脑只有自己收到的那些（所有者的决定）。

use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::{MemberId, RoomId};
use crate::error::AppResult;

/// 记录的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text = 1,
    Voice = 2,
    Ping = 3,
    Emote = 4,
}

impl Kind {
    fn from_db(v: u8) -> Self {
        match v {
            2 => Kind::Voice,
            3 => Kind::Ping,
            4 => Kind::Emote,
            _ => Kind::Text,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: i64,
    pub member_id: MemberId,
    pub ts_ms: u64,
    pub kind: Kind,
    /// 文字的内容、信号 id、表情引用；语音为空。
    pub text: String,
    /// 语音的时长（毫秒）；别的是 0。
    pub voice_ms: u32,
}

pub fn add(
    conn: &Connection,
    room: &RoomId,
    member: &MemberId,
    ts_ms: u64,
    kind: Kind,
    text: &str,
    voice: Option<(&[u8], u32)>,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO history (room_id, member_id, ts_ms, kind, text, voice, voice_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            room,
            member,
            ts_ms as i64,
            kind as u8,
            text,
            voice.map(|(b, _)| b),
            voice.map_or(0, |(_, ms)| ms)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 这个队伍只留最新的 `max_rows` 条；所有队伍的语音加起来超过 `max_voice_bytes` 时从最旧的语音删起。
pub fn trim(
    conn: &Connection,
    room: &RoomId,
    max_rows: usize,
    max_voice_bytes: u64,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM history WHERE room_id = ?1 AND id NOT IN
           (SELECT id FROM history WHERE room_id = ?1 ORDER BY id DESC LIMIT ?2)",
        params![room, max_rows as i64],
    )?;
    loop {
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(LENGTH(voice)), 0) FROM history WHERE voice IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        if total as u64 <= max_voice_bytes {
            return Ok(());
        }
        let deleted = conn.execute(
            "DELETE FROM history WHERE id = (SELECT MIN(id) FROM history WHERE voice IS NOT NULL)",
            [],
        )?;
        if deleted == 0 {
            return Ok(());
        }
    }
}

/// 这个队伍的全部记录，从旧到新（不带语音数据，回放时再取）。
pub fn list(conn: &Connection, room: &RoomId) -> AppResult<Vec<Row>> {
    let mut stmt = conn.prepare(
        "SELECT id, member_id, ts_ms, kind, text, voice_ms FROM history WHERE room_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([room], |r| {
        Ok(Row {
            id: r.get(0)?,
            member_id: r.get(1)?,
            ts_ms: r.get::<_, i64>(2)? as u64,
            kind: Kind::from_db(r.get(3)?),
            text: r.get(4)?,
            voice_ms: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 一条语音的数据（回放用）。
pub fn voice(conn: &Connection, id: i64) -> AppResult<Option<Vec<u8>>> {
    Ok(conn
        .query_row("SELECT voice FROM history WHERE id = ?1", [id], |r| {
            r.get::<_, Option<Vec<u8>>>(0)
        })
        .optional()?
        .flatten())
}

/// 只留 `room` 的记录（None：全删）。返回删了几条。
pub fn keep_only(conn: &Connection, room: Option<&RoomId>) -> AppResult<usize> {
    Ok(match room {
        Some(r) => conn.execute("DELETE FROM history WHERE room_id <> ?1", [r])?,
        None => conn.execute("DELETE FROM history", [])?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::open_in_memory;

    const A: RoomId = [1; 16];
    const B: RoomId = [2; 16];
    const ME: MemberId = [7; 32];
    const YOU: MemberId = [8; 32];

    fn texts(c: &Connection, room: &RoomId) -> Vec<String> {
        list(c, room).unwrap().into_iter().map(|r| r.text).collect()
    }

    #[test]
    fn entries_read_back_in_order_per_team() {
        let c = open_in_memory().unwrap();
        add(&c, &A, &ME, 1000, Kind::Text, "开团", None).unwrap();
        add(&c, &B, &YOU, 1500, Kind::Ping, "danger", None).unwrap();
        let v = add(
            &c,
            &A,
            &YOU,
            2000,
            Kind::Voice,
            "",
            Some((&[1, 2, 3], 2400)),
        )
        .unwrap();
        add(&c, &A, &ME, 3000, Kind::Emote, "facepalm", None).unwrap();
        let rows = list(&c, &A).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (
                rows[0].member_id,
                rows[0].ts_ms,
                rows[0].kind,
                rows[0].text.as_str()
            ),
            (ME, 1000, Kind::Text, "开团")
        );
        assert_eq!((rows[1].kind, rows[1].voice_ms), (Kind::Voice, 2400));
        assert_eq!(rows[2].kind, Kind::Emote);
        assert_eq!(voice(&c, v).unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(voice(&c, rows[0].id).unwrap(), None, "文字没有语音数据");
        assert_eq!(voice(&c, 999).unwrap(), None);
    }

    #[test]
    fn each_team_keeps_only_its_newest_rows() {
        let c = open_in_memory().unwrap();
        for i in 0..5 {
            add(&c, &A, &ME, i, Kind::Text, &i.to_string(), None).unwrap();
        }
        add(&c, &B, &ME, 9, Kind::Text, "b", None).unwrap();
        trim(&c, &A, 3, u64::MAX).unwrap();
        assert_eq!(texts(&c, &A), vec!["2", "3", "4"]);
        assert_eq!(texts(&c, &B), vec!["b"], "别的队伍不受影响");
    }

    #[test]
    fn voice_over_the_cap_drops_the_oldest_voice_first() {
        let c = open_in_memory().unwrap();
        add(&c, &A, &ME, 1, Kind::Voice, "v1", Some((&[0; 40], 20))).unwrap();
        add(&c, &A, &ME, 2, Kind::Text, "t", None).unwrap();
        add(&c, &B, &ME, 3, Kind::Voice, "v2", Some((&[0; 40], 20))).unwrap();
        add(&c, &A, &ME, 4, Kind::Voice, "v3", Some((&[0; 40], 20))).unwrap();
        trim(&c, &A, 100, 80).unwrap();
        assert_eq!(texts(&c, &A), vec!["t", "v3"], "最旧的语音删了，文字留着");
        assert_eq!(texts(&c, &B), vec!["v2"]);
    }

    #[test]
    fn keep_only_the_current_team_or_nothing() {
        let c = open_in_memory().unwrap();
        add(&c, &A, &ME, 1, Kind::Text, "a", None).unwrap();
        add(&c, &B, &ME, 2, Kind::Text, "b", None).unwrap();
        assert_eq!(keep_only(&c, Some(&A)).unwrap(), 1);
        assert_eq!((texts(&c, &A).len(), texts(&c, &B).len()), (1, 0));
        assert_eq!(keep_only(&c, None).unwrap(), 1, "不在队伍里：全删");
        assert!(list(&c, &A).unwrap().is_empty());
    }
}
