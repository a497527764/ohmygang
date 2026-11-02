//! 本机设置（接口文档第 11 节）：只影响本机，不影响层数计算。M1-4 只有「允许互联网」和中继列表，
//! 设置页界面在 M1-5。
//!
//! 实现流程：
//! 1. 每项设置存成 `settings` 表的一行，值为 JSON 文本；没存过的项用默认值。
//!    中继列表只在用户改过之后才落盘，所以以后调整默认列表，没改过的用户会自动用上新的。
//! 2. 修改时先校验再写入：中继地址去掉首尾空格，必须是 `wss://` 加上不含空白的地址，重复的只留第一个。
//!    任一地址不合格就整次修改都不生效。

use rusqlite::Connection;

use crate::error::{AppError, AppResult};
use crate::storage::settings as store;

/// 默认中继（架构 6.4，2026-11-02 实测选定）。
pub const DEFAULT_RELAYS: [&str; 8] = [
    "wss://relay.nostrfeed.com",
    "wss://relay.ditto.pub",
    "wss://purplerelay.com",
    "wss://relay.mostro.network",
    "wss://relay.wisp.talk",
    "wss://data-space.world/r/relais-public-2",
    "wss://relay.dreamith.to",
    "wss://relay.gulugulu.moe",
];

const INTERNET_ALLOWED: &str = "internet_allowed";
const RELAYS: &str = "relays";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalSettings {
    pub internet_allowed: bool,
    pub relays: Vec<String>,
}

impl Default for LocalSettings {
    fn default() -> Self {
        LocalSettings {
            internet_allowed: true,
            relays: DEFAULT_RELAYS.map(String::from).to_vec(),
        }
    }
}

/// 要修改的项；`None` 表示不改。
#[derive(Clone, Debug, Default)]
pub struct SettingsPatch {
    pub internet_allowed: Option<bool>,
    pub relays: Option<Vec<String>>,
}

pub fn load(conn: &Connection) -> AppResult<LocalSettings> {
    let mut s = LocalSettings::default();
    if let Some(v) = store::get(conn, INTERNET_ALLOWED)?.and_then(|v| serde_json::from_str(&v).ok())
    {
        s.internet_allowed = v;
    }
    if let Some(v) = store::get(conn, RELAYS)?.and_then(|v| serde_json::from_str(&v).ok()) {
        s.relays = v;
    }
    Ok(s)
}

pub fn update(conn: &Connection, patch: SettingsPatch) -> AppResult<LocalSettings> {
    let relays = patch.relays.map(normalize_relays).transpose()?;
    let tx = conn.unchecked_transaction()?;
    if let Some(v) = patch.internet_allowed {
        store::set(&tx, INTERNET_ALLOWED, &serde_json::json!(v).to_string())?;
    }
    if let Some(v) = relays {
        store::set(&tx, RELAYS, &serde_json::json!(v).to_string())?;
    }
    tx.commit()?;
    load(conn)
}

fn normalize_relays(urls: Vec<String>) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for url in urls {
        let url = url.trim();
        let host = url.strip_prefix("wss://").unwrap_or("");
        if host.is_empty() || host.chars().any(char::is_whitespace) {
            return Err(AppError::InvalidRelay(url.to_string()));
        }
        if !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    #[test]
    fn defaults_allow_internet_with_eight_relays() {
        let c = storage::open_in_memory().unwrap();
        let s = load(&c).unwrap();
        assert!(s.internet_allowed);
        assert_eq!(s.relays.len(), 8);
        assert_eq!(s.relays, DEFAULT_RELAYS.map(String::from).to_vec());
    }

    #[test]
    fn patch_changes_only_given_fields() {
        let c = storage::open_in_memory().unwrap();
        let s = update(
            &c,
            SettingsPatch {
                internet_allowed: Some(false),
                relays: None,
            },
        )
        .unwrap();
        assert!(!s.internet_allowed);
        assert_eq!(s.relays.len(), 8);

        let s = update(
            &c,
            SettingsPatch {
                internet_allowed: None,
                relays: Some(vec![
                    " wss://a.example ".into(),
                    "wss://b.example/path".into(),
                    "wss://a.example".into(),
                ]),
            },
        )
        .unwrap();
        assert!(!s.internet_allowed);
        // 去掉首尾空格、去重、保持顺序
        assert_eq!(s.relays, vec!["wss://a.example", "wss://b.example/path"]);
        assert_eq!(load(&c).unwrap(), s);
    }

    #[test]
    fn invalid_relay_is_rejected_and_nothing_changes() {
        let c = storage::open_in_memory().unwrap();
        for bad in [
            "ws://plain.example",
            "https://x.example",
            "wss://",
            "wss://a b",
            "",
        ] {
            let r = update(
                &c,
                SettingsPatch {
                    internet_allowed: Some(false),
                    relays: Some(vec!["wss://ok.example".into(), bad.into()]),
                },
            );
            assert!(
                matches!(r, Err(AppError::InvalidRelay(_))),
                "{bad:?} 应该被拒绝"
            );
        }
        assert_eq!(load(&c).unwrap(), LocalSettings::default());
    }

    #[test]
    fn settings_survive_reopening() {
        let path = std::env::temp_dir().join(format!(
            "lolgang-settings-{}.db",
            crate::crypto::random_bytes::<8>()
                .map(u64::from_le_bytes)
                .unwrap()
        ));
        {
            let c = storage::open(&path).unwrap();
            update(
                &c,
                SettingsPatch {
                    internet_allowed: Some(false),
                    relays: Some(vec!["wss://only.example".into()]),
                },
            )
            .unwrap();
        }
        {
            let c = storage::open(&path).unwrap();
            let s = load(&c).unwrap();
            assert!(!s.internet_allowed);
            assert_eq!(s.relays, vec!["wss://only.example"]);
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
