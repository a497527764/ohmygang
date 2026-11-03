//! 2.0 的设置（架构第 8 节）：信号轮盘部分来自 lolPing 设计文档 8.1 节和 `src/shared/settings.ts`（MIT，© 2026 Danrui Meng），
//! 加上 1.x 的主题。M2 以后的队伍、对线、表情、更新设置到时再加。
//!
//! 实现流程：
//! 1. 存的是一份 JSON（`store` 负责读写）。读出来先过 `normalize`：缺的字段补默认，类型不对或不合法的换默认，
//!    数字夹到范围内并对齐步长。所以旧版本、手改坏了的设置文件都能变成合法设置，新加字段只要在这里给默认值。
//! 2. 控制面板改设置时把整份设置交回来，同样过一遍 `normalize` 再存，存下的永远合法。
//! 3. `gesture_config` 把设置换成手势状态机的配置；`overlay` 换成透明窗口画画要用的那部分。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::input::gesture::{self, MOD_ALT, MOD_CTRL, MOD_WIN};

/// 全部信号的 id，和前端 `src/shared/pings.ts` 一致。
pub const PING_IDS: [&str; 12] = [
    "danger",
    "push",
    "omw",
    "allin",
    "assist",
    "needvision",
    "missing",
    "enemyvision",
    "generic",
    "bait",
    "visioncleared",
    "missingflip",
];
/// 英雄联盟自带的轮盘，从正上方顺时针。
pub const DEFAULT_WHEEL: [&str; 8] = [
    "danger",
    "push",
    "omw",
    "allin",
    "assist",
    "needvision",
    "missing",
    "enemyvision",
];
pub const DEFAULT_CLICK_PING: &str = "generic";
pub const THEMES: [&str; 3] = ["system", "light", "dark"];
/// 默认中继（1.x 架构 6.4，2026-11-02 实测选定）。
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
/// 自己填的中继最多这么多个。
const MAX_RELAYS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NamedTrigger {
    Alt,
    Ctrl,
    Shift,
    Win,
    CapsLock,
    Mouse4,
    Mouse5,
}

/// 触发键：有名字的键或侧键（存成 `"alt"`），或自定义按键（存成 `{ "vk": 71 }`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TriggerKey {
    Named(NamedTrigger),
    Custom { vk: u32 },
}

/// 快捷键：修饰键位（Ctrl 1、Alt 2、Shift 4、Win 8）加一个虚拟键码。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub trigger: TriggerKey,
    pub drag_threshold_px: u32,
    pub click_ping: bool,
    /// 触发键加单击放的信号。
    pub click_ping_id: String,
    /// 全屏程序（游戏）占着屏幕时，触发键加点击原样交给它。
    pub game_check: bool,
    /// 轮盘的 8 格，从正上方顺时针。
    pub wheel: Vec<String>,
    pub wheel_size_px: u32,
    pub toggle_hotkey: Hotkey,
    pub ping_size_px: u32,
    pub ping_duration_s: f64,
    pub volume: u32,
    pub muted: bool,
    pub tick_sound: bool,
    pub theme: String,
    /// 不在同一局域网时经公共中继转发；关掉就只用局域网。
    pub allow_internet: bool,
    /// 自己填的中继；空表示用默认列表（以后默认列表更新了，没改过的人自动用上新的）。
    pub relays: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            trigger: TriggerKey::Named(NamedTrigger::Alt),
            drag_threshold_px: 8,
            click_ping: false,
            click_ping_id: DEFAULT_CLICK_PING.into(),
            game_check: true,
            wheel: DEFAULT_WHEEL.map(String::from).to_vec(),
            wheel_size_px: 600,
            toggle_hotkey: Hotkey {
                mods: MOD_CTRL | MOD_ALT,
                vk: 0x50,
            },
            ping_size_px: 110,
            ping_duration_s: 3.2,
            volume: 70,
            muted: false,
            tick_sound: true,
            theme: THEMES[0].into(),
            allow_internet: true,
            relays: Vec::new(),
        }
    }
}

/// 透明窗口画画要的设置。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettings {
    pub wheel_size_px: u32,
    pub ping_size_px: u32,
    pub ping_duration_s: f64,
    /// 0 到 1，静音时是 0。
    pub volume: f64,
    pub tick_sound: bool,
    pub wheel: Vec<String>,
    pub click_ping_id: String,
}

/// 数字设置的范围和步长（同 lolPing 的 LIMITS）。
struct Limit {
    min: f64,
    max: f64,
    step: f64,
}
const DRAG: Limit = Limit {
    min: 2.0,
    max: 40.0,
    step: 1.0,
};
const WHEEL_SIZE: Limit = Limit {
    min: 300.0,
    max: 900.0,
    step: 10.0,
};
const PING_SIZE: Limit = Limit {
    min: 60.0,
    max: 220.0,
    step: 1.0,
};
const DURATION: Limit = Limit {
    min: 1.0,
    max: 8.0,
    step: 0.1,
};
const VOLUME: Limit = Limit {
    min: 0.0,
    max: 100.0,
    step: 1.0,
};

/// 自定义触发键不能用的键：触发键在全系统被拦下，这些键会弄坏打字、提交表单、切换焦点、移动光标。
const UNSAFE_TRIGGER_VKS: [u32; 9] = [0x0D, 0x09, 0x08, 0x20, 0x1B, 0x25, 0x26, 0x27, 0x28];
/// 修饰键有自己的名字，不当自定义键。
const MODIFIER_VKS: [u32; 11] = [
    0x10, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C,
];

fn is_vk(v: u32) -> bool {
    (1..=254).contains(&v)
}

fn num(raw: Option<&Value>, l: &Limit, default: f64) -> f64 {
    let Some(v) = raw.and_then(Value::as_f64).filter(|v| v.is_finite()) else {
        return default;
    };
    let snapped = (v.clamp(l.min, l.max) / l.step).round() * l.step;
    // 去掉 0.1 步长的浮点尾巴（2.3000000000000003）
    (snapped * 10.0).round() / 10.0
}

fn bool_or(raw: Option<&Value>, default: bool) -> bool {
    raw.and_then(Value::as_bool).unwrap_or(default)
}

fn trigger(raw: Option<&Value>) -> TriggerKey {
    let default = Settings::default().trigger;
    let Some(t) = raw.and_then(|v| serde_json::from_value::<TriggerKey>(v.clone()).ok()) else {
        return default;
    };
    match t {
        // macOS 和 X11 拦不住 Caps Lock 的切换，只有 Windows 提供（同 lolPing）。
        TriggerKey::Named(NamedTrigger::CapsLock) if !cfg!(windows) => default,
        TriggerKey::Custom { vk }
            if !is_vk(vk) || UNSAFE_TRIGGER_VKS.contains(&vk) || MODIFIER_VKS.contains(&vk) =>
        {
            default
        }
        t => t,
    }
}

fn hotkey(raw: Option<&Value>) -> Hotkey {
    let default = Settings::default().toggle_hotkey;
    match raw.and_then(|v| serde_json::from_value::<Hotkey>(v.clone()).ok()) {
        // 必须带 Ctrl、Alt 或 Win：只有 Shift 的快捷键会把全系统的大写字母吞掉。
        Some(h)
            if (1..=15).contains(&h.mods)
                && h.mods & (MOD_CTRL | MOD_ALT | MOD_WIN) != 0
                && is_vk(h.vk) =>
        {
            h
        }
        _ => default,
    }
}

fn is_ping_id(s: &str) -> bool {
    PING_IDS.contains(&s)
}

fn wheel(raw: Option<&Value>) -> Vec<String> {
    let list = raw.and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok());
    match list {
        Some(w)
            if w.len() == DEFAULT_WHEEL.len()
                && w.iter().all(|id| is_ping_id(id))
                && w.iter().enumerate().all(|(i, id)| !w[..i].contains(id)) =>
        {
            w
        }
        _ => Settings::default().wheel,
    }
}

/// 只留 wss:// 开头、不含空白的地址，去掉重复的，最多 16 个。
fn relays(raw: Option<&Value>) -> Vec<String> {
    let list = raw
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for url in list {
        let url = url.trim();
        let host = url.strip_prefix("wss://").unwrap_or("");
        if !host.is_empty()
            && !host.chars().any(char::is_whitespace)
            && !out.iter().any(|u| u == url)
        {
            out.push(url.to_string());
        }
    }
    out.truncate(MAX_RELAYS);
    out
}

fn one_of(raw: Option<&Value>, ok: impl Fn(&str) -> bool, default: &str) -> String {
    raw.and_then(Value::as_str)
        .filter(|s| ok(s))
        .unwrap_or(default)
        .to_string()
}

/// 任何 JSON → 合法设置。
pub fn normalize(raw: &Value) -> Settings {
    let d = Settings::default();
    let get = |k: &str| raw.get(k);
    Settings {
        trigger: trigger(get("trigger")),
        drag_threshold_px: num(
            get("dragThresholdPx"),
            &DRAG,
            f64::from(d.drag_threshold_px),
        ) as u32,
        click_ping: bool_or(get("clickPing"), d.click_ping),
        click_ping_id: one_of(get("clickPingId"), is_ping_id, DEFAULT_CLICK_PING),
        game_check: bool_or(get("gameCheck"), d.game_check),
        wheel: wheel(get("wheel")),
        wheel_size_px: num(get("wheelSizePx"), &WHEEL_SIZE, f64::from(d.wheel_size_px)) as u32,
        toggle_hotkey: hotkey(get("toggleHotkey")),
        ping_size_px: num(get("pingSizePx"), &PING_SIZE, f64::from(d.ping_size_px)) as u32,
        ping_duration_s: num(get("pingDurationS"), &DURATION, d.ping_duration_s),
        volume: num(get("volume"), &VOLUME, f64::from(d.volume)) as u32,
        muted: bool_or(get("muted"), d.muted),
        tick_sound: bool_or(get("tickSound"), d.tick_sound),
        theme: one_of(get("theme"), |t| THEMES.contains(&t), THEMES[0]),
        allow_internet: bool_or(get("allowInternet"), d.allow_internet),
        relays: relays(get("relays")),
    }
}

impl Settings {
    /// 实际用的中继：自己填了就用自己的，否则用默认列表。
    pub fn relay_list(&self) -> Vec<String> {
        if self.relays.is_empty() {
            DEFAULT_RELAYS.map(String::from).to_vec()
        } else {
            self.relays.clone()
        }
    }

    /// 手势状态机的配置。`enabled` 是当前的信号开关（托盘、暂停快捷键），不存在设置里。
    pub fn gesture_config(&self, enabled: bool) -> gesture::Config {
        let trigger = match self.trigger {
            TriggerKey::Named(NamedTrigger::Alt) => gesture::Trigger::Alt,
            TriggerKey::Named(NamedTrigger::Ctrl) => gesture::Trigger::Ctrl,
            TriggerKey::Named(NamedTrigger::Shift) => gesture::Trigger::Shift,
            TriggerKey::Named(NamedTrigger::Win) => gesture::Trigger::Win,
            TriggerKey::Named(NamedTrigger::CapsLock) => gesture::Trigger::CapsLock,
            TriggerKey::Named(NamedTrigger::Mouse4) => gesture::Trigger::Mouse4,
            TriggerKey::Named(NamedTrigger::Mouse5) => gesture::Trigger::Mouse5,
            TriggerKey::Custom { vk } => gesture::Trigger::Custom(vk),
        };
        gesture::Config {
            trigger,
            click_ping: self.click_ping,
            game_check: self.game_check,
            emote_trigger: gesture::Trigger::None,
            emote_click: false,
            drag_threshold_px: self.drag_threshold_px as i32,
            toggle_mods: self.toggle_hotkey.mods,
            toggle_vk: self.toggle_hotkey.vk,
            enabled,
        }
    }

    pub fn overlay(&self) -> OverlaySettings {
        OverlaySettings {
            wheel_size_px: self.wheel_size_px,
            ping_size_px: self.ping_size_px,
            ping_duration_s: self.ping_duration_s,
            volume: if self.muted {
                0.0
            } else {
                f64::from(self.volume) / 100.0
            },
            tick_sound: self.tick_sound,
            wheel: self.wheel.clone(),
            click_ping_id: self.click_ping_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
