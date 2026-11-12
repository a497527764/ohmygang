//! 设置：一行，存整份设置的 JSON。读的时候过 `settings2::normalize`，所以坏数据也能读出合法设置。

use rusqlite::{Connection, OptionalExtension};

use crate::error::AppResult;
use crate::settings2::{self, Settings};

/// 读设置。没存过或存的东西坏了，得到默认值或修正后的值。
pub fn load(conn: &Connection) -> AppResult<Settings> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM settings WHERE id = 1", [], |r| r.get(0))
        .optional()?;
    let raw = json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(settings2::normalize(&raw))
}

/// 库里还没有设置行 = 第一次打开（首启按显示器分辨率定字号 / 表情大小要用）。
pub fn is_fresh(conn: &Connection) -> AppResult<bool> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM settings WHERE id = 1", [], |r| r.get(0))
        .optional()?;
    Ok(json.is_none())
}

pub fn save(conn: &Connection, s: &Settings) -> AppResult<()> {
    let json = serde_json::to_string(s).expect("设置总能写成 JSON");
    conn.execute(
        "INSERT INTO settings (id, json) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET json = excluded.json",
        [json],
    )?;
    Ok(())
}
