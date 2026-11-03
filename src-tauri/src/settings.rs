//! 本机设置（接口文档第 11 节）：只影响本机，不影响层数计算。目前有「允许互联网」、中继列表（M1-4），
//! 音效开关和音量（M1-5，音效在界面里播放，这里只保存），新成员同步聊天的条数上限（M2，局域网 5000、中继 500），
//! 界面主题（V1.1：跟随系统、浅色、深色，界面据此切换配色）。
//!
//! 实现流程：
//! 1. 每项设置存成 `settings` 表的一行，值为 JSON 文本；没存过的项用默认值。
//!    中继列表只在用户改过之后才落盘，所以以后调整默认列表，没改过的用户会自动用上新的；
//!    改回和默认列表一模一样时删掉这一行，恢复「跟随默认」。
//! 2. 修改时先校验再写入：中继地址去掉首尾空格，必须是 `wss://` 加上不含空白的地址，重复的只留第一个。
//!    音量必须在 0–100，聊天条数上限不超过 100000，主题只能是 system、light、dark。任一项不合格就整次修改都不生效。

use rusqlite::Connection;
use serde::de::DeserializeOwned;

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
const SOUND_ENABLED: &str = "sound_enabled";
const VOLUME: &str = "volume";
const HISTORY_LIMIT_LAN: &str = "history_limit_lan";
const HISTORY_LIMIT_RELAY: &str = "history_limit_relay";
const THEME: &str = "theme";
const MAX_HISTORY_LIMIT: u32 = 100_000;
/// 可选的主题：跟随系统、浅色、深色。
pub const THEMES: [&str; 3] = ["system", "light", "dark"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalSettings {
    pub internet_allowed: bool,
    pub relays: Vec<String>,
    pub sound_enabled: bool,
    /// 0–100。
    pub volume: u8,
    /// 新进群时经局域网最多同步最近多少条聊天。
    pub history_limit_lan: u32,
    /// 经中继最多同步多少条（中继每秒只能发 3 包，所以少一些）。
    pub history_limit_relay: u32,
    /// system、light 或 dark。
    pub theme: String,
}

impl Default for LocalSettings {
    fn default() -> Self {
        LocalSettings {
            internet_allowed: true,
            relays: DEFAULT_RELAYS.map(String::from).to_vec(),
            sound_enabled: true,
            volume: 80,
            history_limit_lan: 5000,
            history_limit_relay: 500,
            theme: THEMES[0].to_string(),
        }
    }
}

/// 要修改的项；`None` 表示不改。
#[derive(Clone, Debug, Default)]
pub struct SettingsPatch {
    pub internet_allowed: Option<bool>,
    pub relays: Option<Vec<String>>,
    pub sound_enabled: Option<bool>,
    pub volume: Option<u8>,
    pub history_limit_lan: Option<u32>,
    pub history_limit_relay: Option<u32>,
    pub theme: Option<String>,
}

fn read<T: DeserializeOwned>(conn: &Connection, key: &str) -> AppResult<Option<T>> {
    Ok(store::get(conn, key)?.and_then(|v| serde_json::from_str(&v).ok()))
}

fn write(conn: &Connection, key: &str, value: impl serde::Serialize) -> AppResult<()> {
    store::set(conn, key, &serde_json::json!(value).to_string())
}

pub fn load(conn: &Connection) -> AppResult<LocalSettings> {
    let d = LocalSettings::default();
    Ok(LocalSettings {
        internet_allowed: read(conn, INTERNET_ALLOWED)?.unwrap_or(d.internet_allowed),
        relays: read(conn, RELAYS)?.unwrap_or(d.relays),
        sound_enabled: read(conn, SOUND_ENABLED)?.unwrap_or(d.sound_enabled),
        volume: read(conn, VOLUME)?.unwrap_or(d.volume),
        history_limit_lan: read(conn, HISTORY_LIMIT_LAN)?.unwrap_or(d.history_limit_lan),
        history_limit_relay: read(conn, HISTORY_LIMIT_RELAY)?.unwrap_or(d.history_limit_relay),
        theme: read(conn, THEME)?.unwrap_or(d.theme),
    })
}

pub fn update(conn: &Connection, patch: SettingsPatch) -> AppResult<LocalSettings> {
    let relays = patch.relays.map(normalize_relays).transpose()?;
    if patch.volume.is_some_and(|v| v > 100) {
        return Err(AppError::InvalidVolume);
    }
    if [patch.history_limit_lan, patch.history_limit_relay]
        .iter()
        .any(|v| v.is_some_and(|v| v > MAX_HISTORY_LIMIT))
    {
        return Err(AppError::InvalidHistoryLimit);
    }
    if patch.theme.as_deref().is_some_and(|t| !THEMES.contains(&t)) {
        return Err(AppError::InvalidTheme);
    }
    let tx = conn.unchecked_transaction()?;
    if let Some(v) = patch.internet_allowed {
        write(&tx, INTERNET_ALLOWED, v)?;
    }
    if let Some(v) = relays {
        if v == DEFAULT_RELAYS {
            store::delete(&tx, RELAYS)?;
        } else {
            write(&tx, RELAYS, v)?;
        }
    }
    if let Some(v) = patch.sound_enabled {
        write(&tx, SOUND_ENABLED, v)?;
    }
    if let Some(v) = patch.volume {
        write(&tx, VOLUME, v)?;
    }
    if let Some(v) = patch.history_limit_lan {
        write(&tx, HISTORY_LIMIT_LAN, v)?;
    }
    if let Some(v) = patch.history_limit_relay {
        write(&tx, HISTORY_LIMIT_RELAY, v)?;
    }
    if let Some(v) = patch.theme {
        write(&tx, THEME, v)?;
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
                ..SettingsPatch::default()
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
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        assert!(!s.internet_allowed);
        // 去掉首尾空格、去重、保持顺序
        assert_eq!(s.relays, vec!["wss://a.example", "wss://b.example/path"]);
        assert_eq!(load(&c).unwrap(), s);
    }

    #[test]
    fn theme_follows_system_by_default_and_is_checked() {
        let c = storage::open_in_memory().unwrap();
        assert_eq!(load(&c).unwrap().theme, "system");
        let s = update(
            &c,
            SettingsPatch {
                theme: Some("dark".into()),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        assert_eq!(s.theme, "dark");
        assert!(matches!(
            update(
                &c,
                SettingsPatch {
                    theme: Some("pink".into()),
                    ..SettingsPatch::default()
                }
            ),
            Err(AppError::InvalidTheme)
        ));
        assert_eq!(load(&c).unwrap().theme, "dark");
    }

    #[test]
    fn sound_is_on_at_eighty_percent_by_default() {
        let c = storage::open_in_memory().unwrap();
        let s = load(&c).unwrap();
        assert!(s.sound_enabled);
        assert_eq!(s.volume, 80);

        let s = update(
            &c,
            SettingsPatch {
                sound_enabled: Some(false),
                volume: Some(0),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        assert!(!s.sound_enabled);
        assert_eq!(s.volume, 0);
        assert!(s.internet_allowed, "没给的项不变");
    }

    #[test]
    fn restoring_default_relays_follows_future_defaults() {
        let c = storage::open_in_memory().unwrap();
        update(
            &c,
            SettingsPatch {
                relays: Some(vec!["wss://mine.example".into()]),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        assert!(store::get(&c, RELAYS).unwrap().is_some());
        let s = update(
            &c,
            SettingsPatch {
                relays: Some(DEFAULT_RELAYS.map(String::from).to_vec()),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        assert_eq!(s.relays, DEFAULT_RELAYS.map(String::from).to_vec());
        assert!(
            store::get(&c, RELAYS).unwrap().is_none(),
            "改回默认列表后不再单独保存，以后默认列表调整时自动跟上"
        );
    }

    #[test]
    fn volume_above_hundred_is_rejected() {
        let c = storage::open_in_memory().unwrap();
        let r = update(
            &c,
            SettingsPatch {
                sound_enabled: Some(false),
                volume: Some(101),
                ..SettingsPatch::default()
            },
        );
        assert!(matches!(r, Err(AppError::InvalidVolume)));
        assert_eq!(load(&c).unwrap(), LocalSettings::default());
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
                    ..SettingsPatch::default()
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
                    volume: Some(30),
                    ..SettingsPatch::default()
                },
            )
            .unwrap();
        }
        {
            let c = storage::open(&path).unwrap();
            let s = load(&c).unwrap();
            assert!(!s.internet_allowed);
            assert_eq!(s.relays, vec!["wss://only.example"]);
            assert_eq!(s.volume, 30);
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
