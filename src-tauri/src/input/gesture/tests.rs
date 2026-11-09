//! 手势状态机测试：逐条移植 lolPing `native/hook-helper/tests/helper_tests.cpp` 里信号轮盘、表情轮盘、
//! 暂停、修饰键补丁、看门狗、按键同步、自定义键补打的用例（MIT，© 2026 Danrui Meng）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

const ALT: u32 = 0xA4;
const CTRL: u32 = 0xA2;
const KEY_P: u32 = 0x50;
const ESC: u32 = 0x1B;
const CAPS: u32 = 0x14;
const KEY_V: u32 = 0x56;
const KEY_T: u32 = 0x54;
const KEY_G: u32 = 0x47;
const KEY_H: u32 = 0x48;
const TAB: u32 = 0x09;
const LWIN: u32 = 0x5B;
const LSHIFT: u32 = 0xA0;

fn kd(vk: u32) -> InputEvent {
    InputEvent::key(EvKind::KeyDown, vk, 0)
}
fn ku(vk: u32) -> InputEvent {
    InputEvent::key(EvKind::KeyUp, vk, 0)
}
fn md(b: Btn, x: i32, y: i32) -> InputEvent {
    InputEvent::mouse(EvKind::MouseDown, b, x, y, 0)
}
fn md_at(b: Btn, x: i32, y: i32, t: u64) -> InputEvent {
    InputEvent::mouse(EvKind::MouseDown, b, x, y, t)
}
fn mu(b: Btn, x: i32, y: i32) -> InputEvent {
    InputEvent::mouse(EvKind::MouseUp, b, x, y, 0)
}
fn mv(x: i32, y: i32) -> InputEvent {
    InputEvent::mouse(EvKind::MouseMove, Btn::None, x, y, 0)
}
fn mv_at(x: i32, y: i32, t: u64) -> InputEvent {
    InputEvent::mouse(EvKind::MouseMove, Btn::None, x, y, t)
}

fn has_cancel(r: &Outcome) -> bool {
    r.emits.contains(&Emit::Cancel)
}
fn wheel_open(r: &Outcome) -> Option<(i32, i32, WheelKind)> {
    r.emits.iter().find_map(|e| match *e {
        Emit::WheelOpen { x, y, wheel } => Some((x, y, wheel)),
        _ => None,
    })
}
fn wheel_move(r: &Outcome) -> Option<(i32, i32, WheelKind)> {
    r.emits.iter().find_map(|e| match *e {
        Emit::WheelMove { x, y, wheel } => Some((x, y, wheel)),
        _ => None,
    })
}
fn wheel_release(r: &Outcome) -> Option<(i32, i32, WheelKind)> {
    r.emits.iter().find_map(|e| match *e {
        Emit::WheelRelease { x, y, wheel } => Some((x, y, wheel)),
        _ => None,
    })
}
fn click(r: &Outcome) -> Option<(i32, i32, WheelKind)> {
    r.emits.iter().find_map(|e| match *e {
        Emit::Click { x, y, wheel } => Some((x, y, wheel)),
        _ => None,
    })
}
fn toggled(r: &Outcome) -> Option<bool> {
    r.emits.iter().find_map(|e| match *e {
        Emit::Toggled { enabled } => Some(enabled),
        _ => None,
    })
}

fn with_config(c: Config) -> Gesture {
    let mut g = Gesture::new();
    g.set_config(c);
    g
}
fn with_trigger(trigger: Trigger, click_ping: bool) -> Gesture {
    with_config(Config {
        trigger,
        click_ping,
        ..Config::default()
    })
}
/// 信号在 Alt（默认），表情在 `emote`。
fn with_emote(emote: Trigger, emote_click: bool) -> Gesture {
    with_config(Config {
        emote_trigger: emote,
        emote_click,
        ..Config::default()
    })
}
/// 按住 Alt，在 (100,100) 按下左键拖到 (130,100)：轮盘开了。
fn open_wheel(g: &mut Gesture) {
    g.on_event(&kd(ALT));
    g.on_event(&md(Btn::Left, 100, 100));
    g.on_event(&mv(130, 100));
}
/// 同样的拖动换成 Ctrl：表情轮盘开了（配合 with_emote）。
fn open_emote_wheel(g: &mut Gesture) {
    g.on_event(&kd(CTRL));
    g.on_event(&md(Btn::Left, 100, 100));
    g.on_event(&mv(130, 100));
}

const MASK: [Inject; 3] = [
    Inject::KeyDown(MASK_VK),
    Inject::KeyUp(MASK_VK),
    Inject::KeyUp(ALT),
];

#[test]
fn alt_drag_opens_wheel_and_releases() {
    let mut g = Gesture::new();
    assert!(!g.on_event(&kd(ALT)).swallow);
    let r = g.on_event(&md(Btn::Left, 100, 100));
    assert!(r.swallow);
    assert_eq!(g.state(), State::Pending);
    let r = g.on_event(&mv(104, 104)); // 5.7 像素，没过 8 像素的阈值
    assert!(!r.swallow && r.emits.is_empty());
    let r = g.on_event(&mv(120, 100));
    assert!(!r.swallow);
    assert_eq!(g.state(), State::Wheel);
    assert_eq!(
        r.emits,
        vec![
            Emit::WheelOpen {
                x: 100,
                y: 100,
                wheel: WheelKind::Ping
            },
            Emit::WheelMove {
                x: 120,
                y: 100,
                wheel: WheelKind::Ping
            },
        ]
    );
    assert!(wheel_move(&g.on_event(&mv(125, 90))).is_some());
    let r = g.on_event(&mu(Btn::Left, 130, 90));
    assert!(r.swallow);
    assert_eq!(wheel_release(&r), Some((130, 90, WheelKind::Ping)));
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn plain_click_passes_through() {
    let mut g = Gesture::new();
    assert!(!g.on_event(&md(Btn::Left, 1, 1)).swallow);
    assert!(!g.on_event(&mu(Btn::Left, 1, 1)).swallow);
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn alt_click_replays_when_click_ping_off() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    assert!(g.on_event(&md(Btn::Left, 50, 60)).swallow);
    g.on_event(&mv(53, 62)); // 一点抖动，还算单击
    let r = g.on_event(&mu(Btn::Left, 53, 62));
    assert!(r.swallow);
    assert!(r.emits.is_empty());
    assert_eq!(
        r.injects,
        vec![Inject::ButtonDown(Btn::Left), Inject::ButtonUp(Btn::Left)]
    );
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn alt_click_pings_when_click_ping_on() {
    let mut g = with_trigger(Trigger::Alt, true);
    g.on_event(&kd(ALT));
    g.on_event(&md(Btn::Left, 50, 60));
    let r = g.on_event(&mu(Btn::Left, 51, 60));
    assert!(r.swallow);
    assert!(r.injects.is_empty());
    assert_eq!(click(&r), Some((50, 60, WheelKind::Ping)));
}

#[test]
fn self_injected_events_are_ignored() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    let mut e = md(Btn::Left, 5, 5);
    e.self_injected = true;
    assert!(!g.on_event(&e).swallow);
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn right_click_cancels_wheel() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    let r = g.on_event(&md(Btn::Right, 130, 100));
    assert!(r.swallow);
    assert!(has_cancel(&r));
    assert!(g.on_event(&mu(Btn::Right, 130, 100)).swallow);
    let r = g.on_event(&mu(Btn::Left, 130, 100));
    assert!(r.swallow);
    assert!(wheel_release(&r).is_none());
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn right_click_with_alt_but_no_gesture_passes() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    assert!(!g.on_event(&md(Btn::Right, 1, 1)).swallow);
    assert!(!g.on_event(&mu(Btn::Right, 1, 1)).swallow);
}

#[test]
fn escape_cancels_wheel() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    let r = g.on_event(&kd(ESC));
    assert!(r.swallow);
    assert!(has_cancel(&r));
    assert!(g.on_event(&ku(ESC)).swallow);
    let r = g.on_event(&mu(Btn::Left, 130, 100));
    assert!(r.swallow && r.emits.is_empty());
}

/// 和英雄联盟一样，轮盘开了以后只看鼠标：先松开触发键轮盘还在（触控板三指拖动的抬起来得晚，照样能放信号）。
#[test]
fn trigger_released_during_wheel_keeps_wheel() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    let r = g.on_event(&ku(ALT));
    assert!(r.emits.is_empty());
    assert_eq!(g.state(), State::Wheel);
    assert!(r.swallow && r.injects.len() == 3); // Alt 弹起照样打补丁
    assert!(wheel_move(&g.on_event(&mv(140, 90))).is_some());
    let r = g.on_event(&mu(Btn::Left, 140, 90));
    assert!(r.swallow);
    assert_eq!(wheel_release(&r), Some((140, 90, WheelKind::Ping)));
    assert_eq!(g.state(), State::Idle);
    assert!(!g.on_event(&md(Btn::Left, 1, 1)).swallow); // 没按触发键：单击又放行了

    let mut e = Gesture::new();
    open_wheel(&mut e);
    e.on_event(&ku(ALT));
    let r = e.on_event(&kd(ESC)); // Esc 照样取消
    assert!(r.swallow && has_cancel(&r));
    assert!(e.on_event(&mu(Btn::Left, 130, 100)).swallow);
    assert_eq!(e.state(), State::Idle);
}

#[test]
fn trigger_released_during_pending_replays_click_after_mask() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    g.on_event(&md(Btn::Left, 100, 100));
    let r = g.on_event(&ku(ALT));
    assert!(r.swallow); // Alt 弹起被拦下，补丁之后再回放
    assert_eq!(
        r.injects,
        vec![
            Inject::KeyDown(MASK_VK),
            Inject::KeyUp(MASK_VK),
            Inject::KeyUp(ALT),
            Inject::ButtonDown(Btn::Left),
        ]
    );
    assert_eq!(g.state(), State::Idle);
    assert!(!g.on_event(&mu(Btn::Left, 100, 100)).swallow); // 真正的抬起补全这次单击
}

#[test]
fn trigger_released_during_pending_pings_when_click_ping_on() {
    let mut g = with_trigger(Trigger::Alt, true);
    g.on_event(&kd(ALT));
    g.on_event(&md(Btn::Left, 100, 100));
    let r = g.on_event(&ku(ALT));
    assert_eq!(click(&r), Some((100, 100, WheelKind::Ping)));
    assert!(g.on_event(&mu(Btn::Left, 100, 100)).swallow);
}

/// Alt+Tab 属于任务切换：这个组合键期间单击原样放行，松开 Alt 以后 Alt+单击又能开手势。
#[test]
fn alt_tab_clicks_pass_through() {
    let mut g = Gesture::new();
    assert!(!g.on_event(&kd(ALT)).swallow);
    assert!(!g.on_event(&kd(TAB)).swallow); // 切换器的键把这个组合键标成别人的
    assert!(!g.on_event(&ku(TAB)).swallow);
    let r = g.on_event(&md(Btn::Left, 100, 100));
    assert!(!r.swallow); // 这次单击属于切换器
    assert!(click(&r).is_none());
    assert!(!g.on_event(&mu(Btn::Left, 100, 100)).swallow);
    let r = g.on_event(&ku(ALT)); // 什么都没拦，Alt 弹起不用补丁
    assert!(!r.swallow);
    assert!(r.injects.is_empty());
    g.on_event(&kd(ALT)); // 组合键结束：Alt+单击又是手势
    assert!(g.on_event(&md(Btn::Left, 100, 100)).swallow);
    let r = g.on_event(&mu(Btn::Left, 100, 100));
    assert!(r.swallow);
    assert_eq!(r.injects.len(), 2); // 单击信号关着：回放这次单击
}

/// 单击信号打开时也一样：Alt+Tab 加单击不能放信号。
#[test]
fn alt_tab_click_places_no_ping_even_with_click_ping_on() {
    let mut g = with_trigger(Trigger::Alt, true);
    g.on_event(&kd(ALT));
    g.on_event(&kd(TAB));
    let r = g.on_event(&md(Btn::Left, 100, 100));
    assert!(!r.swallow);
    assert!(click(&r).is_none());
    assert!(!g.on_event(&mu(Btn::Left, 100, 100)).swallow);
    let r = g.on_event(&ku(ALT));
    assert!(!r.swallow);
    assert!(r.injects.is_empty());
}

/// Win 当触发键时的 Win+Tab（任务视图）；按键同步会清掉过期的「别人的组合键」标记。
#[test]
fn win_tab_clicks_pass_through_and_sync_clears_flag() {
    let mut g = with_trigger(Trigger::Win, false);
    g.on_event(&kd(LWIN));
    g.on_event(&kd(TAB));
    assert!(!g.on_event(&md(Btn::Left, 100, 100)).swallow);
    assert!(!g.on_event(&mu(Btn::Left, 100, 100)).swallow);
    g.sync_key_states(|_| false); // 系统说所有键都松开了（Win+L、UAC……）
    g.on_event(&kd(LWIN));
    assert!(g.on_event(&md(Btn::Left, 100, 100)).swallow); // 又能开手势了
}

/// 全屏游戏占着屏幕：检查说全屏时触发键加单击放行，回到桌面后又能开手势（单击信号打开，游戏里放信号的情况）。
#[test]
fn fullscreen_game_keeps_the_click() {
    let mut g = with_config(Config {
        click_ping: true,
        game_check: true,
        ..Config::default()
    });
    let fullscreen = Arc::new(AtomicBool::new(true));
    let f = fullscreen.clone();
    g.set_fullscreen_probe(Box::new(move |_, _| f.load(Ordering::Relaxed)));
    g.on_event(&kd(ALT));
    let r = g.on_event(&md(Btn::Left, 100, 100));
    assert!(!r.swallow); // 点击归游戏
    assert!(click(&r).is_none());
    assert!(!g.on_event(&mu(Btn::Left, 100, 100)).swallow);
    fullscreen.store(false, Ordering::Relaxed); // 回到桌面
    assert!(g.on_event(&md(Btn::Left, 100, 100)).swallow);
    let r = g.on_event(&mu(Btn::Left, 100, 100));
    assert!(r.swallow);
    assert!(click(&r).is_some());
    assert_eq!(g.state(), State::Idle);
}

/// 全屏检查关着时根本不问。
#[test]
fn fullscreen_probe_ignored_when_disabled() {
    let mut g = with_config(Config {
        click_ping: true,
        game_check: false,
        ..Config::default()
    });
    let consulted = Arc::new(AtomicBool::new(false));
    let c = consulted.clone();
    g.set_fullscreen_probe(Box::new(move |_, _| {
        c.store(true, Ordering::Relaxed);
        true
    }));
    g.on_event(&kd(ALT));
    assert!(g.on_event(&md(Btn::Left, 100, 100)).swallow); // 说是全屏也照样开手势
    assert!(!consulted.load(Ordering::Relaxed));
    let r = g.on_event(&mu(Btn::Left, 100, 100));
    assert!(r.swallow);
    assert!(click(&r).is_some());
}

#[test]
fn alt_up_after_gesture_is_masked() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    g.on_event(&mu(Btn::Left, 130, 100));
    let r = g.on_event(&ku(ALT));
    assert!(r.swallow);
    assert_eq!(r.injects, MASK.to_vec());
}

#[test]
fn plain_alt_tap_is_not_masked() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    let r = g.on_event(&ku(ALT));
    assert!(!r.swallow && r.injects.is_empty());
}

#[test]
fn mask_menu_keys_off_leaves_alt_up_alone() {
    let mut g = Gesture::new();
    g.set_mask_menu_keys(false);
    open_wheel(&mut g);
    g.on_event(&mu(Btn::Left, 130, 100));
    let r = g.on_event(&ku(ALT));
    assert!(!r.swallow && r.injects.is_empty());
}

#[test]
fn toggle_hotkey_disables_and_enables() {
    let mut g = Gesture::new();
    g.on_event(&kd(CTRL));
    g.on_event(&kd(ALT));
    let r = g.on_event(&kd(KEY_P));
    assert!(r.swallow);
    assert_eq!(toggled(&r), Some(false));
    assert!(!g.enabled());
    assert!(g.on_event(&ku(KEY_P)).swallow);
    g.on_event(&ku(CTRL));
    assert!(!g.on_event(&md(Btn::Left, 1, 1)).swallow); // 暂停了：Alt+按下放行
    g.on_event(&mu(Btn::Left, 1, 1));
    g.on_event(&kd(CTRL));
    let r = g.on_event(&kd(KEY_P));
    assert_eq!(toggled(&r), Some(true));
    assert!(g.enabled());
}

#[test]
fn hotkey_autorepeat_toggles_once() {
    let mut g = Gesture::new();
    g.on_event(&kd(CTRL));
    g.on_event(&kd(ALT));
    assert!(toggled(&g.on_event(&kd(KEY_P))).is_some());
    let repeat = g.on_event(&kd(KEY_P));
    assert!(repeat.swallow);
    assert!(toggled(&repeat).is_none());
    assert!(!g.enabled());
}

#[test]
fn hotkey_ignored_while_suspended() {
    let mut g = Gesture::new();
    g.set_suspended(true);
    g.on_event(&kd(CTRL));
    g.on_event(&kd(ALT));
    let r = g.on_event(&kd(KEY_P));
    assert!(!r.swallow && r.emits.is_empty());
    assert!(g.enabled());
    assert!(!g.on_event(&md(Btn::Left, 1, 1)).swallow); // 挂起时也不开手势
}

#[test]
fn disabling_mid_drag_cancels_and_swallows_up() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    let r = g.set_enabled(false);
    assert!(has_cancel(&r));
    assert_eq!(g.state(), State::SwallowUp);
    assert!(g.on_event(&mu(Btn::Left, 130, 100)).swallow);
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn config_change_mid_drag_cancels() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    let r = g.set_config(Config::default());
    assert!(has_cancel(&r));
    assert_eq!(g.state(), State::SwallowUp);
}

#[test]
fn lost_up_recovers_on_next_press() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    g.set_enabled(false); // 进 SwallowUp，等一个永远不来的抬起
    g.set_enabled(true);
    let r = g.on_event(&md(Btn::Left, 5, 5)); // Alt 还按着
    assert!(r.swallow);
    assert_eq!(g.state(), State::Pending);
}

#[test]
fn lost_right_up_does_not_swallow_next_click() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    assert!(g.on_event(&md(Btn::Right, 130, 100)).swallow); // 取消轮盘，准备拦右键抬起
    // 右键抬起一直没到（安全桌面、Win+L、UAC）。
    g.on_event(&mu(Btn::Left, 130, 100));
    assert_eq!(g.state(), State::Idle);
    assert!(!g.on_event(&md(Btn::Right, 5, 5)).swallow); // 新的右键按下说明旧的抬起丢了
    assert!(!g.on_event(&mu(Btn::Right, 5, 5)).swallow);
}

#[test]
fn lost_key_up_does_not_swallow_next_press() {
    let mut g = Gesture::new();
    open_wheel(&mut g);
    assert!(g.on_event(&kd(ESC)).swallow); // 取消轮盘，准备拦 Esc 抬起
    // Esc 抬起一直没到。
    g.on_event(&mu(Btn::Left, 130, 100));
    g.on_event(&ku(ALT));
    assert_eq!(g.state(), State::Idle);
    assert!(!g.on_event(&kd(ESC)).swallow); // 之后放行的按下清掉过期的记录
    assert!(!g.on_event(&ku(ESC)).swallow);
}

#[test]
fn button_held_before_trigger_passes_up() {
    let mut g = Gesture::new();
    assert!(!g.on_event(&md(Btn::Left, 1, 1)).swallow);
    g.on_event(&kd(ALT));
    assert!(!g.on_event(&mv(50, 50)).swallow);
    assert!(!g.on_event(&mu(Btn::Left, 50, 50)).swallow);
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn watchdog_cancels_stale_gesture() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    g.on_event(&md_at(Btn::Left, 100, 100, 1000));
    g.on_event(&mv_at(130, 100, 1100));
    assert!(g.tick(5000).emits.is_empty());
    let r = g.tick(1100 + WATCHDOG_MS + 1);
    assert!(has_cancel(&r));
    assert_eq!(g.state(), State::SwallowUp); // 配对的抬起还是要拦
    assert!(g.on_event(&mu(Btn::Left, 130, 100)).swallow);
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn watchdog_from_pending_swallows_later_up() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    g.on_event(&md_at(Btn::Left, 100, 100, 1000)); // 一直没拖过阈值
    assert_eq!(g.state(), State::Pending);
    assert!(g.tick(1000 + WATCHDOG_MS).emits.is_empty()); // 正好到期：还不算卡住
    let r = g.tick(1000 + WATCHDOG_MS + 1);
    assert!(has_cancel(&r));
    assert_eq!(g.state(), State::SwallowUp);
    let r = g.on_event(&mu(Btn::Left, 100, 100));
    assert!(r.swallow && r.emits.is_empty() && r.injects.is_empty());
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn sync_clears_stale_trigger() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT)); // 配对的 Alt 抬起一直没看到（Win+L、UAC、钩子重装……）
    g.sync_key_states(|_| false);
    assert!(!g.on_event(&md(Btn::Left, 1, 1)).swallow);
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn sync_keeps_keys_that_are_still_down() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    g.sync_key_states(|vk| vk == ALT);
    assert!(g.on_event(&md(Btn::Left, 1, 1)).swallow);
    assert_eq!(g.state(), State::Pending);
}

#[test]
fn sync_never_sets_keys() {
    let mut g = Gesture::new();
    g.sync_key_states(|_| true); // 系统说全按着，但我们没见过按下
    assert!(!g.on_event(&md(Btn::Left, 1, 1)).swallow);
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn sync_keeps_swallowed_trigger_key() {
    let mut g = with_trigger(Trigger::CapsLock, false);
    assert!(g.on_event(&kd(CAPS)).swallow);
    g.sync_key_states(|_| false); // 系统从没见过 Caps 按下
    assert!(g.on_event(&md(Btn::Left, 0, 0)).swallow);
    assert_eq!(g.state(), State::Pending);
}

#[test]
fn ctrl_trigger_ignores_alt() {
    let mut g = with_trigger(Trigger::Ctrl, false);
    g.on_event(&kd(ALT));
    assert!(!g.on_event(&md(Btn::Left, 1, 1)).swallow);
    g.on_event(&mu(Btn::Left, 1, 1));
    g.on_event(&ku(ALT));
    g.on_event(&kd(CTRL));
    assert!(g.on_event(&md(Btn::Left, 1, 1)).swallow);
}

#[test]
fn capslock_trigger_swallows_key() {
    let mut g = with_trigger(Trigger::CapsLock, false);
    assert!(g.on_event(&kd(CAPS)).swallow);
    assert!(g.on_event(&md(Btn::Left, 0, 0)).swallow);
    assert!(wheel_open(&g.on_event(&mv(0, 40))).is_some());
    assert!(wheel_release(&g.on_event(&mu(Btn::Left, 0, 40))).is_some());
    let r = g.on_event(&ku(CAPS));
    assert!(r.swallow && r.injects.is_empty());
}

#[test]
fn custom_key_trigger() {
    let mut g = with_trigger(Trigger::Custom(KEY_V), false);
    assert!(g.on_event(&kd(KEY_V)).swallow);
    assert!(g.on_event(&md(Btn::Left, 0, 0)).swallow);
    assert!(g.on_event(&ku(KEY_V)).swallow);
    g.set_enabled(false);
    assert!(!g.on_event(&kd(KEY_V)).swallow); // 暂停了：V 又能打字
}

/// 自定义触发键单独按一下：松开时补打出来。
#[test]
fn custom_key_tap_types_the_key() {
    let mut g = with_trigger(Trigger::Custom(KEY_G), false);
    let r = g.on_event(&kd(KEY_G));
    assert!(r.swallow && r.injects.is_empty()); // 先扣住，看它干什么用
    assert!(g.on_event(&kd(KEY_G)).swallow); // 扣着时的连发：继续等
    let r = g.on_event(&ku(KEY_G));
    assert!(r.swallow);
    assert_eq!(
        r.injects,
        vec![Inject::KeyDown(KEY_G), Inject::KeyUp(KEY_G)]
    );
    assert_eq!(g.state(), State::Idle);
}

/// 快速打 "gh"（G 还没松开就按了 H）保持顺序：先补发 G，H 排在后面回放。
#[test]
fn custom_key_rollover_keeps_typed_order() {
    let mut g = with_trigger(Trigger::Custom(KEY_G), false);
    assert!(g.on_event(&kd(KEY_G)).swallow);
    let r = g.on_event(&kd(KEY_H));
    assert!(r.swallow);
    assert_eq!(
        r.injects,
        vec![Inject::KeyDown(KEY_G), Inject::KeyDown(KEY_H)]
    );
    assert!(!g.on_event(&kd(KEY_G)).swallow); // 补发过了：G 的连发照常打字
    let r = g.on_event(&ku(KEY_G));
    assert!(!r.swallow && r.injects.is_empty()); // 系统见过 G 按下，就得见到真正的抬起
    assert!(!g.on_event(&ku(KEY_H)).swallow);
    // G 在打字时单击不是手势：它属于用户正在打字的地方。
    assert!(g.on_event(&kd(KEY_G)).swallow); // 新的一次按下又先扣住
    g.on_event(&kd(KEY_H));
    assert!(!g.on_event(&md(Btn::Left, 0, 0)).swallow);
    g.on_event(&mu(Btn::Left, 0, 0));
    g.on_event(&ku(KEY_H));
    assert!(!g.on_event(&ku(KEY_G)).swallow);
}

/// 按着 G 时按了 Shift：先补发小写 g，前面的字母不会被变成大写。
#[test]
fn custom_key_flushes_before_modifier() {
    let mut g = with_trigger(Trigger::Custom(KEY_G), false);
    g.on_event(&kd(KEY_G));
    let r = g.on_event(&kd(LSHIFT));
    assert!(r.swallow);
    assert_eq!(
        r.injects,
        vec![Inject::KeyDown(KEY_G), Inject::KeyDown(LSHIFT)]
    );
    assert!(!g.on_event(&ku(KEY_G)).swallow);
}

/// 拖动用掉了扣着的键，松开时什么都不补。
#[test]
fn custom_key_drag_types_nothing() {
    let mut g = with_trigger(Trigger::Custom(KEY_G), false);
    g.on_event(&kd(KEY_G));
    assert!(g.on_event(&md(Btn::Left, 100, 100)).swallow);
    assert!(wheel_open(&g.on_event(&mv(140, 100))).is_some());
    assert!(wheel_release(&g.on_event(&mu(Btn::Left, 140, 100))).is_some());
    let r = g.on_event(&ku(KEY_G));
    assert!(r.swallow && r.injects.is_empty());
}

/// Caps Lock 不补打：补了会切换大小写。
#[test]
fn caps_lock_is_not_tapped_through() {
    let mut g = with_trigger(Trigger::CapsLock, false);
    assert!(g.on_event(&kd(CAPS)).swallow);
    let r = g.on_event(&ku(CAPS));
    assert!(r.swallow && r.injects.is_empty());
}

#[test]
fn mouse4_trigger_drags_with_side_button() {
    let mut g = with_trigger(Trigger::Mouse4, false);
    assert!(!g.on_event(&md(Btn::Left, 0, 0)).swallow);
    g.on_event(&mu(Btn::Left, 0, 0));
    assert!(g.on_event(&md(Btn::X1, 10, 10)).swallow);
    let r = g.on_event(&mu(Btn::X1, 10, 10)); // 没拖动的单击：回放，浏览器的「后退」照常
    assert!(r.swallow);
    assert_eq!(
        r.injects,
        vec![Inject::ButtonDown(Btn::X1), Inject::ButtonUp(Btn::X1)]
    );
    g.on_event(&md(Btn::X1, 10, 10));
    assert!(wheel_open(&g.on_event(&mv(40, 10))).is_some());
    assert!(wheel_release(&g.on_event(&mu(Btn::X1, 40, 10))).is_some());
}

#[test]
fn ctrl_drag_opens_emote_wheel() {
    let mut g = with_emote(Trigger::Ctrl, false);
    assert!(!g.on_event(&kd(CTRL)).swallow);
    assert!(g.on_event(&md(Btn::Left, 100, 100)).swallow);
    assert_eq!(g.state(), State::Pending);
    let r = g.on_event(&mv(130, 100));
    assert_eq!(wheel_open(&r), Some((100, 100, WheelKind::Emote)));
    assert_eq!(wheel_move(&r).map(|m| m.2), Some(WheelKind::Emote));
    assert_eq!(
        wheel_move(&g.on_event(&mv(135, 90))).map(|m| m.2),
        Some(WheelKind::Emote)
    );
    let r = g.on_event(&mu(Btn::Left, 140, 90));
    assert!(r.swallow);
    assert_eq!(wheel_release(&r), Some((140, 90, WheelKind::Emote)));
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn alt_drag_is_still_the_ping_wheel() {
    let mut g = with_emote(Trigger::Ctrl, false);
    assert!(!g.on_event(&kd(ALT)).swallow);
    assert!(g.on_event(&md(Btn::Left, 100, 100)).swallow);
    let r = g.on_event(&mv(130, 100));
    assert_eq!(wheel_open(&r).map(|o| o.2), Some(WheelKind::Ping));
    assert_eq!(wheel_move(&r).map(|m| m.2), Some(WheelKind::Ping));
    let r = g.on_event(&mu(Btn::Left, 130, 100));
    assert!(r.swallow);
    assert_eq!(wheel_release(&r).map(|m| m.2), Some(WheelKind::Ping));
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn both_triggers_held_pass_through() {
    let mut g = with_emote(Trigger::Ctrl, false);
    g.on_event(&kd(ALT));
    g.on_event(&kd(CTRL));
    let r = g.on_event(&md(Btn::Left, 100, 100));
    assert!(!r.swallow && r.emits.is_empty() && r.injects.is_empty());
    assert_eq!(g.state(), State::Idle);
    let r = g.on_event(&mv(150, 100));
    assert!(!r.swallow && r.emits.is_empty());
    let r = g.on_event(&mu(Btn::Left, 150, 100));
    assert!(!r.swallow && r.emits.is_empty() && r.injects.is_empty());
    let r = g.on_event(&ku(ALT)); // 什么都没拦，Alt 弹起不用补丁
    assert!(!r.swallow && r.injects.is_empty());
}

#[test]
fn emote_click_on_and_off() {
    {
        let mut g = with_emote(Trigger::Ctrl, true);
        g.on_event(&kd(CTRL));
        assert!(g.on_event(&md(Btn::Left, 50, 60)).swallow);
        let r = g.on_event(&mu(Btn::Left, 51, 60));
        assert!(r.swallow && r.injects.is_empty());
        assert_eq!(click(&r), Some((50, 60, WheelKind::Emote)));
    }
    {
        let mut g = with_emote(Trigger::Ctrl, false);
        g.on_event(&kd(CTRL));
        assert!(g.on_event(&md(Btn::Left, 50, 60)).swallow);
        let r = g.on_event(&mu(Btn::Left, 50, 60));
        assert!(r.swallow && r.emits.is_empty());
        assert_eq!(
            r.injects,
            vec![Inject::ButtonDown(Btn::Left), Inject::ButtonUp(Btn::Left)]
        );
    }
    {
        // 两个轮盘各看各的单击设置：单击信号开、表情单击关。
        let mut g = with_config(Config {
            click_ping: true,
            emote_trigger: Trigger::Ctrl,
            ..Config::default()
        });
        g.on_event(&kd(CTRL));
        g.on_event(&md(Btn::Left, 50, 60));
        let r = g.on_event(&mu(Btn::Left, 50, 60));
        assert!(r.emits.is_empty() && r.injects.len() == 2);
        g.on_event(&ku(CTRL));
        g.on_event(&kd(ALT));
        g.on_event(&md(Btn::Left, 50, 60));
        let r = g.on_event(&mu(Btn::Left, 50, 60));
        assert_eq!(click(&r).map(|c| c.2), Some(WheelKind::Ping));
        assert!(r.injects.is_empty());
    }
    {
        // 先松开 Ctrl 再松开鼠标：放表情，或者回放单击。
        let mut on = with_emote(Trigger::Ctrl, true);
        on.on_event(&kd(CTRL));
        on.on_event(&md(Btn::Left, 100, 100));
        let up = on.on_event(&ku(CTRL));
        assert_eq!(click(&up), Some((100, 100, WheelKind::Emote)));
        assert!(on.on_event(&mu(Btn::Left, 100, 100)).swallow);
        let mut off = with_emote(Trigger::Ctrl, false);
        off.on_event(&kd(CTRL));
        off.on_event(&md(Btn::Left, 100, 100));
        let r = off.on_event(&ku(CTRL));
        assert_eq!(r.injects, vec![Inject::ButtonDown(Btn::Left)]);
        assert_eq!(off.state(), State::Idle);
        assert!(!off.on_event(&mu(Btn::Left, 100, 100)).swallow);
    }
}

#[test]
fn escape_and_right_click_cancel_emote_gesture() {
    let mut g = with_emote(Trigger::Ctrl, false);
    open_emote_wheel(&mut g);
    let r = g.on_event(&kd(ESC));
    assert!(r.swallow);
    assert!(has_cancel(&r));
    assert!(g.on_event(&ku(ESC)).swallow);
    let r = g.on_event(&mu(Btn::Left, 130, 100));
    assert!(r.swallow && r.emits.is_empty());
    assert_eq!(g.state(), State::Idle);

    g.on_event(&md(Btn::Left, 100, 100)); // Ctrl 还按着
    assert_eq!(g.state(), State::Pending);
    let r = g.on_event(&md(Btn::Right, 100, 100));
    assert!(r.swallow);
    assert!(has_cancel(&r));
    assert!(g.on_event(&mu(Btn::Right, 100, 100)).swallow);
    let r = g.on_event(&mu(Btn::Left, 100, 100));
    assert!(r.swallow && r.emits.is_empty() && r.injects.is_empty());
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn emote_trigger_none_ignores_ctrl_drag() {
    let mut g = with_emote(Trigger::None, false);
    g.on_event(&kd(CTRL));
    assert!(!g.on_event(&md(Btn::Left, 100, 100)).swallow);
    assert!(g.on_event(&mv(150, 100)).emits.is_empty());
    assert!(!g.on_event(&mu(Btn::Left, 150, 100)).swallow);
    assert_eq!(g.state(), State::Idle);
}

#[test]
fn custom_emote_key_is_swallowed() {
    let mut g = with_emote(Trigger::Custom(KEY_T), false);
    assert!(g.on_event(&kd(KEY_T)).swallow);
    assert!(g.on_event(&md(Btn::Left, 0, 0)).swallow);
    assert_eq!(
        wheel_open(&g.on_event(&mv(0, 40))).map(|o| o.2),
        Some(WheelKind::Emote)
    );
    assert_eq!(
        wheel_release(&g.on_event(&mu(Btn::Left, 0, 40))).map(|o| o.2),
        Some(WheelKind::Emote)
    );
    assert!(g.on_event(&ku(KEY_T)).swallow);
    assert!(!g.on_event(&kd(KEY_V)).swallow); // 其他键照常打字
    g.on_event(&ku(KEY_V));
    g.set_enabled(false);
    assert!(!g.on_event(&kd(KEY_T)).swallow); // 暂停了：T 又能打字
    assert!(!g.on_event(&ku(KEY_T)).swallow);
}

#[test]
fn mouse5_emote_beside_alt_ping() {
    let mut g = with_emote(Trigger::Mouse5, false);
    assert!(!g.on_event(&md(Btn::Left, 0, 0)).swallow); // 没按键：普通单击
    g.on_event(&mu(Btn::Left, 0, 0));
    assert!(g.on_event(&md(Btn::X2, 10, 10)).swallow);
    assert_eq!(
        wheel_open(&g.on_event(&mv(40, 10))).map(|o| o.2),
        Some(WheelKind::Emote)
    );
    let r = g.on_event(&mu(Btn::X2, 40, 10));
    assert!(r.swallow);
    assert_eq!(wheel_release(&r).map(|o| o.2), Some(WheelKind::Emote));

    g.on_event(&kd(ALT));
    assert!(g.on_event(&md(Btn::Left, 100, 100)).swallow);
    assert_eq!(
        wheel_open(&g.on_event(&mv(130, 100))).map(|o| o.2),
        Some(WheelKind::Ping)
    );
    assert_eq!(
        wheel_release(&g.on_event(&mu(Btn::Left, 130, 100))).map(|o| o.2),
        Some(WheelKind::Ping)
    );

    // Alt 还按着时，侧键只是表情的触发。
    assert!(g.on_event(&md(Btn::X2, 10, 10)).swallow);
    assert_eq!(
        wheel_open(&g.on_event(&mv(40, 10))).map(|o| o.2),
        Some(WheelKind::Emote)
    );
    g.on_event(&mu(Btn::X2, 40, 10));
    assert_eq!(g.state(), State::Idle);

    // 侧键没拖动的单击回放（表情单击关着），浏览器的「前进」照常。
    g.on_event(&md(Btn::X2, 10, 10));
    let r = g.on_event(&mu(Btn::X2, 10, 10));
    assert!(r.swallow);
    assert_eq!(
        r.injects,
        vec![Inject::ButtonDown(Btn::X2), Inject::ButtonUp(Btn::X2)]
    );
}

#[test]
fn pause_stops_both_wheels() {
    let mut g = with_emote(Trigger::Ctrl, false);
    g.on_event(&kd(CTRL));
    g.on_event(&kd(ALT));
    assert!(toggled(&g.on_event(&kd(KEY_P))).is_some());
    assert!(!g.enabled());
    g.on_event(&ku(KEY_P));
    g.on_event(&ku(ALT)); // 只剩 Ctrl 按着
    assert!(!g.on_event(&md(Btn::Left, 100, 100)).swallow);
    assert!(g.on_event(&mv(150, 100)).emits.is_empty());
    assert!(!g.on_event(&mu(Btn::Left, 150, 100)).swallow);
    g.on_event(&ku(CTRL));
    g.on_event(&kd(ALT));
    assert!(!g.on_event(&md(Btn::Left, 100, 100)).swallow);
    assert!(!g.on_event(&mu(Btn::Left, 100, 100)).swallow);
}

#[test]
fn releasing_emote_trigger_mid_wheel_keeps_wheel() {
    let mut g = with_emote(Trigger::Ctrl, false);
    open_emote_wheel(&mut g);
    g.on_event(&kd(ALT)); // 另一个轮盘的触发键不影响这次手势
    assert!(g.on_event(&ku(ALT)).emits.is_empty());
    assert_eq!(g.state(), State::Wheel);
    assert!(g.on_event(&ku(CTRL)).emits.is_empty());
    assert_eq!(g.state(), State::Wheel);
    let r = g.on_event(&mu(Btn::Left, 130, 100));
    assert!(r.swallow);
    assert_eq!(wheel_release(&r).map(|o| o.2), Some(WheelKind::Emote));
    assert_eq!(g.state(), State::Idle);
}

fn kd_at(vk: u32, t: u64) -> InputEvent {
    InputEvent::key(EvKind::KeyDown, vk, t)
}

fn with_team_keys() -> Gesture {
    with_config(Config {
        text_vk: KEY_T,
        text_mods: MOD_ALT,
        panel_vk: TAB,
        ..Config::default()
    })
}

#[test]
fn text_hotkey_opens_the_box_once_and_cancels_a_gesture() {
    let mut g = Gesture::new();
    g.on_event(&kd(ALT));
    assert!(
        !g.on_event(&kd(KEY_T)).swallow,
        "不在队伍里（没配置）：Alt+T 照常放行"
    );
    g.on_event(&ku(KEY_T));
    g.on_event(&ku(ALT));

    let mut g = with_team_keys();
    g.on_event(&kd(ALT));
    g.on_event(&md(Btn::Left, 100, 100));
    g.on_event(&mv(140, 100));
    let r = g.on_event(&kd(KEY_T));
    assert!(r.swallow);
    assert!(has_cancel(&r), "开着的轮盘取消");
    assert!(r.emits.contains(&Emit::TextRequested));
    assert!(g.on_event(&kd(KEY_T)).emits.is_empty(), "连发不再打开");
    assert!(g.on_event(&ku(KEY_T)).swallow);
    assert!(!g.on_event(&kd(KEY_G)).swallow);
    g.on_event(&ku(KEY_G));
    g.on_event(&ku(ALT));
    assert!(!g.on_event(&kd(KEY_T)).swallow, "不带 Alt 的 T 照常打字");
    g.on_event(&ku(KEY_T));
    g.set_enabled(false);
    g.on_event(&kd(ALT));
    assert!(!g.on_event(&kd(KEY_T)).swallow, "信号暂停时不拦");
}

#[test]
fn panel_key_shows_while_held_and_hides_on_release() {
    let mut g = with_team_keys();
    let r = g.on_event(&kd_at(TAB, 1000));
    assert!(r.swallow && r.emits == vec![Emit::Panel { show: true }]);
    let r = g.on_event(&kd_at(TAB, 1030));
    assert!(r.swallow && r.emits.is_empty(), "连发拦下，不再显示");
    let r = g.on_event(&ku(TAB));
    assert!(r.swallow && r.emits == vec![Emit::Panel { show: false }]);
    // 带修饰键的 Tab（Alt+Tab、Ctrl+Tab）放行
    g.on_event(&kd(ALT));
    assert!(!g.on_event(&kd(TAB)).swallow);
    g.on_event(&ku(TAB));
    g.on_event(&ku(ALT));
    // 没配置时放行
    let mut off = Gesture::new();
    assert!(!off.on_event(&kd(TAB)).swallow);
}

#[test]
fn a_lost_panel_key_up_hides_the_panel_after_the_repeats_stop() {
    let mut g = with_team_keys();
    g.on_event(&kd_at(TAB, 1000));
    assert!(g.tick(1000 + PANEL_REPEAT_GAP_MS).emits.is_empty());
    // 按了别的键：连发停了，但面板键还按着
    g.on_event(&kd_at(KEY_G, 2400));
    g.on_event(&ku(KEY_G));
    assert!(g.tick(2400 + PANEL_REPEAT_GAP_MS).emits.is_empty());
    let r = g.tick(2401 + PANEL_REPEAT_GAP_MS);
    assert_eq!(r.emits, vec![Emit::Panel { show: false }]);
    // 真正的抬起后来到了：照样拦下，不再收起一次
    let r = g.on_event(&ku(TAB));
    assert!(r.swallow && r.emits.is_empty());
    // 下一次按下又能显示
    assert_eq!(
        g.on_event(&kd_at(TAB, 9000)).emits,
        vec![Emit::Panel { show: true }]
    );
}

#[test]
fn panel_key_passes_through_in_a_fullscreen_game_and_when_it_is_a_trigger() {
    let mut g = with_config(Config {
        panel_vk: TAB,
        game_check: true,
        ..Config::default()
    });
    g.set_fullscreen_probe(Box::new(|x, _| x > 1000));
    g.on_event(&mv(1500, 10));
    assert!(
        !g.on_event(&kd(TAB)).swallow,
        "全屏游戏里 Tab 是游戏的记分板"
    );
    g.on_event(&ku(TAB));
    g.on_event(&mv(10, 10));
    assert!(g.on_event(&kd(TAB)).swallow);
    g.on_event(&ku(TAB));
    // 自定义触发键就是这个键：归轮盘
    let mut g = with_config(Config {
        panel_vk: KEY_T,
        trigger: Trigger::Custom(KEY_T),
        ..Config::default()
    });
    assert!(
        !g.on_event(&kd(KEY_T))
            .emits
            .contains(&Emit::Panel { show: true })
    );
}

#[test]
fn panel_watchdog_can_be_turned_off() {
    let mut g = with_team_keys();
    g.set_panel_repeat_gap(0);
    g.on_event(&kd_at(TAB, 1000));
    assert!(g.tick(1_000_000).emits.is_empty(), "关掉后只等真正的抬起");
    assert_eq!(
        g.on_event(&ku(TAB)).emits,
        vec![Emit::Panel { show: false }]
    );
    // 平台层发现漏了事件时可以直接收起
    g.on_event(&kd_at(TAB, 2000));
    assert_eq!(g.release_panel().emits, vec![Emit::Panel { show: false }]);
    assert!(g.release_panel().emits.is_empty());
}

// ---- 说话键（3.0 语音）----

fn ku_at(vk: u32, t: u64) -> InputEvent {
    InputEvent::key(EvKind::KeyUp, vk, t)
}

fn with_talk() -> Gesture {
    with_config(Config {
        talk_vk: KEY_V,
        ..Config::default()
    })
}

fn talk(r: &Outcome) -> Vec<TalkPhase> {
    r.emits
        .iter()
        .filter_map(|e| match *e {
            Emit::Talk(p) => Some(p),
            _ => None,
        })
        .collect()
}

#[test]
fn holding_the_talk_key_talks_and_releasing_sends() {
    let mut g = with_talk();
    let r = g.on_event(&kd_at(KEY_V, 1000));
    assert!(r.swallow && r.injects.is_empty());
    assert_eq!(
        talk(&r),
        vec![TalkPhase::Start],
        "按下就开始录，不吃掉第一个字"
    );
    let r = g.on_event(&kd_at(KEY_V, 1500));
    assert!(r.swallow && talk(&r).is_empty(), "连发拦下，不重复开始");
    let r = g.on_event(&ku_at(KEY_V, 2200));
    assert!(r.swallow && r.injects.is_empty());
    assert_eq!(talk(&r), vec![TalkPhase::End]);
}

#[test]
fn tapping_the_talk_key_types_it() {
    let mut g = with_talk();
    g.on_event(&kd_at(KEY_V, 1000));
    let r = g.on_event(&ku_at(KEY_V, 1000 + TALK_MIN_MS - 1));
    assert!(r.swallow, "真正的抬起拦下（系统没见过按下）");
    assert_eq!(
        r.injects,
        vec![Inject::KeyDown(KEY_V), Inject::KeyUp(KEY_V)]
    );
    assert_eq!(talk(&r), vec![TalkPhase::Cancel]);
}

#[test]
fn typing_right_after_the_talk_key_types_it_first() {
    let mut g = with_talk();
    g.on_event(&kd_at(KEY_V, 1000));
    // 0.3 秒内来了别的键：在打字。先补发 V，再回放这个键，保持顺序
    let r = g.on_event(&kd_at(KEY_G, 1100));
    assert!(r.swallow);
    assert_eq!(
        r.injects,
        vec![Inject::KeyDown(KEY_V), Inject::KeyDown(KEY_G)]
    );
    assert_eq!(talk(&r), vec![TalkPhase::Cancel]);
    // V 的连发和真正的抬起交给系统
    assert!(!g.on_event(&kd_at(KEY_V, 1150)).swallow);
    let up = g.on_event(&ku_at(KEY_V, 1200));
    assert!(!up.swallow && up.injects.is_empty() && talk(&up).is_empty());
    assert!(!g.on_event(&ku_at(KEY_G, 1210)).swallow);
}

#[test]
fn other_keys_while_talking_pass_through() {
    let mut g = with_talk();
    g.on_event(&kd_at(KEY_V, 1000));
    // 说了 0.3 秒以后按技能键：照常交给游戏，接着说
    let r = g.on_event(&kd_at(KEY_G, 1000 + TALK_MIN_MS + 50));
    assert!(!r.swallow && r.injects.is_empty() && talk(&r).is_empty());
    g.on_event(&ku_at(KEY_G, 1400));
    assert_eq!(talk(&g.on_event(&ku_at(KEY_V, 2000))), vec![TalkPhase::End]);
}

#[test]
fn talk_key_with_modifiers_or_outside_a_team_is_left_alone() {
    let mut g = with_talk();
    g.on_event(&kd(CTRL));
    let r = g.on_event(&kd(KEY_V));
    assert!(!r.swallow && talk(&r).is_empty(), "Ctrl+V 照常粘贴");
    g.on_event(&ku(KEY_V));
    g.on_event(&ku(CTRL));
    let mut g = Gesture::new();
    assert!(
        !g.on_event(&kd(KEY_V)).swallow,
        "没配置（不在队伍里、关了语音）：照常打字"
    );
    g.on_event(&ku(KEY_V));
    let mut g = with_talk();
    g.set_enabled(false);
    assert!(!g.on_event(&kd(KEY_V)).swallow, "信号暂停时不拦");
}

#[test]
fn pausing_or_reconfiguring_mid_talk_ends_it_and_still_swallows_the_up() {
    let mut g = with_talk();
    g.on_event(&kd_at(KEY_V, 1000));
    let r = g.set_enabled(false);
    assert_eq!(talk(&r), vec![TalkPhase::End]);
    let up = g.on_event(&ku_at(KEY_V, 3000));
    assert!(up.swallow && talk(&up).is_empty() && up.injects.is_empty());

    let mut g = with_talk();
    g.on_event(&kd_at(KEY_V, 1000));
    let r = g.set_config(Config::default());
    assert_eq!(talk(&r), vec![TalkPhase::End], "退出队伍（说话键关掉）");
    assert!(g.on_event(&ku_at(KEY_V, 3000)).swallow);
}

#[test]
fn a_lost_talk_key_up_ends_talking_after_the_repeats_stop() {
    let mut g = with_talk();
    g.on_event(&kd_at(KEY_V, 1000));
    g.on_event(&kd_at(KEY_V, 1500)); // 连发
    assert!(talk(&g.tick(1500 + PANEL_REPEAT_GAP_MS)).is_empty());
    // 按别的键会停掉连发，但说明还按着
    g.on_event(&kd_at(KEY_G, 2000));
    assert!(talk(&g.tick(2000 + PANEL_REPEAT_GAP_MS)).is_empty());
    assert_eq!(
        talk(&g.tick(2001 + PANEL_REPEAT_GAP_MS)),
        vec![TalkPhase::End]
    );
    // 下一次按下重新开始
    g.on_event(&ku_at(KEY_G, 4000));
    assert_eq!(
        talk(&g.on_event(&kd_at(KEY_V, 5000))),
        vec![TalkPhase::Start]
    );
}

#[test]
fn mouse_is_wanted_only_while_a_trigger_is_held_or_a_gesture_is_open() {
    let mut g = Gesture::new();
    assert!(!g.wants_mouse(), "没按触发键：鼠标事件一个都不用看");
    g.on_event(&kd(ALT));
    assert!(g.wants_mouse(), "按住 Alt：要看");
    g.on_event(&ku(ALT));
    assert!(!g.wants_mouse());
    // 按住 Alt 开了轮盘、先松开 Alt：手势还没完，要等到左键抬起
    open_wheel(&mut g);
    g.on_event(&ku(ALT));
    assert!(g.wants_mouse(), "轮盘开着，等左键抬起");
    g.on_event(&mu(Btn::Left, 130, 100));
    assert!(!g.wants_mouse(), "手势完了");
    // 右键取消轮盘：右键的抬起也要拦下，所以在它来之前还要看
    open_wheel(&mut g);
    g.on_event(&ku(ALT));
    g.on_event(&md(Btn::Right, 130, 100));
    g.on_event(&mu(Btn::Left, 130, 100));
    assert!(g.wants_mouse(), "还有一个右键抬起要拦");
    assert!(g.on_event(&mu(Btn::Right, 130, 100)).swallow);
    assert!(!g.wants_mouse());
}

#[test]
fn mouse_is_wanted_while_paused_only_to_finish_a_gesture() {
    let mut g = Gesture::new();
    g.set_enabled(false);
    g.on_event(&kd(ALT));
    assert!(!g.wants_mouse(), "暂停时按 Alt 也不开手势");
    g.on_event(&ku(ALT));
    g.set_enabled(true);
    g.set_suspended(true);
    g.on_event(&kd(ALT));
    assert!(!g.wants_mouse(), "录按键时也不看");
}

#[test]
fn the_once_a_second_tick_is_needed_only_while_something_is_held() {
    let mut g = Gesture::new();
    assert!(!g.needs_tick(), "什么都没按：不用定时醒");
    g.on_event(&kd(KEY_G));
    assert!(
        g.needs_tick(),
        "有按着的键：每秒和系统对一次，抬起丢了也能发现"
    );
    g.on_event(&ku(KEY_G));
    assert!(!g.needs_tick());
    open_wheel(&mut g);
    g.on_event(&ku(ALT));
    assert!(g.needs_tick(), "轮盘开着：看门狗要每秒查");
    g.on_event(&mu(Btn::Left, 130, 100));
    assert!(!g.needs_tick());
}

#[test]
fn side_button_triggers_always_want_the_mouse() {
    let g = with_trigger(Trigger::Mouse4, false);
    assert!(g.wants_mouse(), "侧键本身就是鼠标事件，只能一直看");
    let mut g = with_emote(Trigger::Mouse5, false);
    assert!(g.wants_mouse());
    g.set_enabled(false);
    assert!(!g.wants_mouse(), "暂停时侧键也不开手势");
    let mut g = with_emote(Trigger::None, false);
    g.on_event(&kd(CTRL));
    assert!(!g.wants_mouse(), "表情轮盘关着，Ctrl 不算触发键");
}
