//! 2.0 的数据文件 `gang2.db`（架构第 8 节）。1.x 的 `lolgang.db` 不读、不删。
//!
//! 实现流程：
//! 1. `open`：打开数据库，开 WAL（写的时候不挡读，断电不容易坏），按 `PRAGMA user_version` 补齐缺的表结构升级。
//! 2. M1 只有一张 `settings` 表，一行，存整份设置的 JSON。读的时候过 `settings2::normalize`，所以坏数据也能读出合法设置。
//! 3. M2 起在 `MIGRATIONS` 末尾追加身份、队伍、成员、状态事件的表；已经发出去的升级语句不再改。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::error::AppResult;
use crate::settings2::{self, Settings};

const MIGRATIONS: &[&str] =
    &["CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL);"];

pub fn open(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> AppResult<Connection> {
    let conn = Connection::open_in_memory()?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> AppResult<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as u32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// 读设置。没存过或存的东西坏了，得到默认值或修正后的值。
pub fn load_settings(conn: &Connection) -> AppResult<Settings> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM settings WHERE id = 1", [], |r| r.get(0))
        .optional()?;
    let raw = json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(settings2::normalize(&raw))
}

pub fn save_settings(conn: &Connection, s: &Settings) -> AppResult<()> {
    let json = serde_json::to_string(s).expect("设置总能写成 JSON");
    conn.execute(
        "INSERT INTO settings (id, json) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET json = excluded.json",
        [json],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_database_reads_default_settings() {
        let c = open_in_memory().unwrap();
        assert_eq!(load_settings(&c).unwrap(), Settings::default());
    }

    #[test]
    fn saved_settings_read_back() {
        let c = open_in_memory().unwrap();
        let s = Settings {
            volume: 25,
            theme: "dark".into(),
            ..Settings::default()
        };
        save_settings(&c, &s).unwrap();
        assert_eq!(load_settings(&c).unwrap(), s);
        let s2 = Settings { muted: true, ..s };
        save_settings(&c, &s2).unwrap();
        assert_eq!(load_settings(&c).unwrap(), s2);
    }

    #[test]
    fn broken_json_reads_as_defaults() {
        let c = open_in_memory().unwrap();
        c.execute("INSERT INTO settings (id, json) VALUES (1, 'not json')", [])
            .unwrap();
        assert_eq!(load_settings(&c).unwrap(), Settings::default());
    }

    #[test]
    fn reopening_a_file_keeps_the_settings() {
        let dir = std::env::temp_dir().join(format!("gang2-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gang2.db");
        let s = Settings {
            ping_size_px: 150,
            ..Settings::default()
        };
        save_settings(&open(&path).unwrap(), &s).unwrap();
        assert_eq!(load_settings(&open(&path).unwrap()).unwrap(), s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
