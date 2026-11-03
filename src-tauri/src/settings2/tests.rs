//! 2.0 设置的测试：默认值、坏值换默认、越界夹回范围、和手势状态机配置的对应。

use serde_json::json;

use super::*;
use crate::input::gesture::{MOD_ALT, MOD_CTRL, Trigger as GTrigger};

#[test]
fn empty_or_broken_input_gives_the_defaults() {
    assert_eq!(normalize(&json!({})), Settings::default());
    assert_eq!(normalize(&json!(null)), Settings::default());
    assert_eq!(normalize(&json!("garbage")), Settings::default());
    let d = Settings::default();
    assert_eq!(d.trigger, TriggerKey::Named(NamedTrigger::Alt));
    assert_eq!(d.wheel, DEFAULT_WHEEL.map(String::from).to_vec());
    assert_eq!(d.click_ping_id, "generic");
    assert_eq!(
        d.toggle_hotkey,
        Hotkey {
            mods: MOD_CTRL | MOD_ALT,
            vk: 0x50
        }
    );
    assert!(d.game_check && !d.click_ping && d.tick_sound && !d.muted);
}

#[test]
fn a_saved_file_reads_back_the_same() {
    let s = Settings {
        trigger: TriggerKey::Custom { vk: 0x47 },
        drag_threshold_px: 12,
        click_ping: true,
        click_ping_id: "danger".into(),
        game_check: false,
        wheel: [
            "push",
            "omw",
            "allin",
            "assist",
            "needvision",
            "missing",
            "enemyvision",
            "bait",
        ]
        .map(String::from)
        .to_vec(),
        wheel_size_px: 450,
        toggle_hotkey: Hotkey {
            mods: MOD_ALT,
            vk: 0x4F,
        },
        ping_size_px: 90,
        ping_duration_s: 2.5,
        volume: 30,
        muted: true,
        tick_sound: false,
        theme: "dark".into(),
        allow_internet: false,
        relays: vec!["wss://my.relay".into()],
    };
    assert_eq!(normalize(&serde_json::to_value(&s).unwrap()), s);
}

#[test]
fn numbers_are_clamped_and_snapped_to_their_step() {
    let s = normalize(&json!({
        "dragThresholdPx": 100, "wheelSizePx": 455, "pingSizePx": 10, "pingDurationS": 2.345, "volume": -5
    }));
    assert_eq!(s.drag_threshold_px, 40);
    assert_eq!(s.wheel_size_px, 460);
    assert_eq!(s.ping_size_px, 60);
    assert_eq!(s.ping_duration_s, 2.3);
    assert_eq!(s.volume, 0);
    let s = normalize(&json!({ "volume": "loud", "pingDurationS": f64::NAN }));
    assert_eq!(s.volume, 70);
    assert_eq!(s.ping_duration_s, 3.2);
}

#[test]
fn triggers_accept_named_keys_mouse_buttons_and_safe_custom_keys() {
    let t = |v: serde_json::Value| normalize(&json!({ "trigger": v })).trigger;
    assert_eq!(t(json!("ctrl")), TriggerKey::Named(NamedTrigger::Ctrl));
    assert_eq!(t(json!("mouse5")), TriggerKey::Named(NamedTrigger::Mouse5));
    assert_eq!(t(json!({ "vk": 0x56 })), TriggerKey::Custom { vk: 0x56 });
    // 回车、Tab、空格、Esc、方向键当触发键会弄坏全系统的打字，换回默认。
    for vk in [0x0D, 0x09, 0x08, 0x20, 0x1B, 0x25, 0x28] {
        assert_eq!(
            t(json!({ "vk": vk })),
            TriggerKey::Named(NamedTrigger::Alt),
            "vk {vk:#x}"
        );
    }
    // 修饰键有自己的名字；0 和超出范围的键码不是键。
    for vk in [0xA4, 0x11, 0x5B, 0, 255, 1000] {
        assert_eq!(
            t(json!({ "vk": vk })),
            TriggerKey::Named(NamedTrigger::Alt),
            "vk {vk:#x}"
        );
    }
    assert_eq!(t(json!("meta")), TriggerKey::Named(NamedTrigger::Alt));
}

#[test]
fn caps_lock_trigger_is_windows_only() {
    let s = normalize(&json!({ "trigger": "capslock" }));
    if cfg!(windows) {
        assert_eq!(s.trigger, TriggerKey::Named(NamedTrigger::CapsLock));
    } else {
        assert_eq!(s.trigger, TriggerKey::Named(NamedTrigger::Alt));
    }
}

#[test]
fn hotkey_needs_ctrl_alt_or_win() {
    let h = |v: serde_json::Value| normalize(&json!({ "toggleHotkey": v })).toggle_hotkey;
    assert_eq!(
        h(json!({ "mods": 2, "vk": 0x4F })),
        Hotkey { mods: 2, vk: 0x4F }
    );
    assert_eq!(
        h(json!({ "mods": 9, "vk": 0x70 })),
        Hotkey { mods: 9, vk: 0x70 }
    ); // Ctrl+Win+F1
    // 只有 Shift 会把全系统的大写字母吞掉。
    assert_eq!(
        h(json!({ "mods": 4, "vk": 0x4F })),
        Settings::default().toggle_hotkey
    );
    assert_eq!(
        h(json!({ "mods": 0, "vk": 0x4F })),
        Settings::default().toggle_hotkey
    );
    assert_eq!(
        h(json!({ "mods": 2, "vk": 0 })),
        Settings::default().toggle_hotkey
    );
}

#[test]
fn wheel_must_be_eight_known_pings_each_once() {
    let w = |v: serde_json::Value| normalize(&json!({ "wheel": v })).wheel;
    let default = Settings::default().wheel;
    let ok = json!([
        "bait",
        "push",
        "omw",
        "allin",
        "assist",
        "needvision",
        "missing",
        "missingflip"
    ]);
    assert_eq!(
        w(ok.clone()),
        serde_json::from_value::<Vec<String>>(ok).unwrap()
    );
    assert_eq!(w(json!(["bait", "push"])), default);
    assert_eq!(
        w(json!([
            "bait",
            "bait",
            "omw",
            "allin",
            "assist",
            "needvision",
            "missing",
            "danger"
        ])),
        default
    );
    assert_eq!(
        w(json!([
            "nope",
            "push",
            "omw",
            "allin",
            "assist",
            "needvision",
            "missing",
            "danger"
        ])),
        default
    );
}

#[test]
fn unknown_click_ping_and_theme_fall_back() {
    let s = normalize(&json!({ "clickPingId": "nope", "theme": "purple" }));
    assert_eq!(s.click_ping_id, "generic");
    assert_eq!(s.theme, "system");
}

#[test]
fn gesture_config_follows_the_settings() {
    let mut s = Settings {
        trigger: TriggerKey::Named(NamedTrigger::Mouse4),
        click_ping: true,
        drag_threshold_px: 15,
        toggle_hotkey: Hotkey {
            mods: MOD_ALT,
            vk: 0x4F,
        },
        ..Settings::default()
    };
    let c = s.gesture_config(false);
    assert_eq!(c.trigger, GTrigger::Mouse4);
    assert!(c.click_ping && c.game_check && !c.enabled);
    assert_eq!(c.emote_trigger, GTrigger::None);
    assert_eq!(c.drag_threshold_px, 15);
    assert_eq!((c.toggle_mods, c.toggle_vk), (MOD_ALT, 0x4F));
    s.trigger = TriggerKey::Custom { vk: 0x47 };
    assert_eq!(s.gesture_config(true).trigger, GTrigger::Custom(0x47));
}

#[test]
fn overlay_gets_volume_as_a_fraction_and_zero_when_muted() {
    let mut s = Settings::default();
    assert_eq!(s.overlay().volume, 0.7);
    s.muted = true;
    assert_eq!(s.overlay().volume, 0.0);
}

#[test]
fn relays_keep_valid_unique_wss_addresses() {
    let s = normalize(
        &json!({ "relays": [" wss://a.example ", "ws://b.example", "wss://", "wss://a b", "wss://a.example", "wss://c.example"] }),
    );
    assert_eq!(
        s.relays,
        vec!["wss://a.example".to_string(), "wss://c.example".to_string()]
    );
    assert_eq!(s.relay_list(), s.relays);
    let d = Settings::default();
    assert!(d.allow_internet && d.relays.is_empty());
    assert_eq!(d.relay_list(), DEFAULT_RELAYS.map(String::from).to_vec());
    assert!(!normalize(&json!({ "allowInternet": false })).allow_internet);
}
