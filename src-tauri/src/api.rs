//! 2.0 的程序状态和命令：控制面板和透明窗口通过 `invoke` 调这里。
//!
//! 实现流程：
//! 1. 启动时（`lib.rs`）开 `gang2.db`、读设置，建 `AppState` 交给 Tauri 管理。
//! 2. 改设置：整份设置过 `normalize` → 存库 → 交给输入线程（手势配置）和透明窗口线程（画画的设置）→ 主窗口换主题。
//! 3. 信号开关（托盘、控制面板、暂停快捷键）统一走 `set_enabled`：告诉输入线程、勾上托盘、通知控制面板。
//! 4. 这些命令都很快（读写一行 SQLite、往通道里放消息），而且持锁时不等别的线程，所以放在主线程上执行没问题。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::input::{Command, InputHandle};
use crate::overlay::{Msg, Overlays};
use crate::settings2::{self, OverlaySettings, Settings};
use crate::{storage, tray};

pub struct AppState {
    pub db: Mutex<Connection>,
    pub settings: Mutex<Settings>,
    /// 信号开关。不存进设置：每次启动都是开的。
    pub enabled: AtomicBool,
    /// 输入线程；启动失败时是 None，`input_error` 说明原因。
    pub input: Option<InputHandle>,
    pub input_error: Option<String>,
    pub overlays: Overlays,
}

impl AppState {
    fn settings(&self) -> Settings {
        self.settings.lock().expect("设置锁没有中毒").clone()
    }
}

/// 控制面板顶部的状态。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    /// 输入监听没起来的原因；正常时为 null。
    pub input_error: Option<String>,
}

/// 开关信号，并让托盘、控制面板跟上。
pub fn set_enabled(app: &AppHandle, on: bool) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    state.enabled.store(on, Ordering::Relaxed);
    if let Some(input) = &state.input {
        input.send(Command::Enabled(on));
    }
    after_toggle(app, on);
}

/// 开关已经变了（暂停快捷键在输入线程里直接切换）：托盘勾选、通知控制面板。
pub fn after_toggle(app: &AppHandle, on: bool) {
    if let Some(state) = app.try_state::<AppState>() {
        state.enabled.store(on, Ordering::Relaxed);
    }
    tray::set_checked(app, on);
    let _ = app.emit_to("main", "pings://enabled", on);
}

/// 主窗口标题栏跟着主题走（跟随系统时交给系统）。
pub fn apply_window_theme(app: &AppHandle, theme: &str) {
    let t = match theme {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_theme(t);
    }
}

#[tauri::command]
pub fn settings_read(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// 存一份新设置，返回修正后的设置（控制面板以它为准）。
#[tauri::command]
pub fn settings_write(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Value,
) -> Result<Settings, String> {
    let s = settings2::normalize(&settings);
    storage::settings::save(&state.db.lock().expect("数据库锁没有中毒"), &s)
        .map_err(|e| e.to_string())?;
    *state.settings.lock().expect("设置锁没有中毒") = s.clone();
    if let Some(input) = &state.input {
        input.send(Command::Config(
            s.gesture_config(state.enabled.load(Ordering::Relaxed)),
        ));
    }
    state.overlays.send(Msg::Settings(s.overlay()));
    apply_window_theme(&app, &s.theme);
    Ok(s)
}

#[tauri::command]
pub fn status_get(state: State<'_, AppState>) -> Status {
    Status {
        enabled: state.enabled.load(Ordering::Relaxed),
        input_error: state.input_error.clone(),
    }
}

#[tauri::command]
pub fn pings_set_enabled(app: AppHandle, on: bool) {
    set_enabled(&app, on);
}

/// 设置页录按键期间挂起拦截，键都交给控制面板。
#[tauri::command]
pub fn input_suspend(state: State<'_, AppState>, on: bool) {
    if let Some(input) = &state.input {
        input.send(Command::Suspended(on));
    }
}

/// 在主显示器中间放一个信号看看效果。
#[tauri::command]
pub fn ping_preview(state: State<'_, AppState>, id: String) {
    if settings2::PING_IDS.contains(&id.as_str()) {
        state.overlays.send(Msg::Preview(id));
    }
}

/// 透明窗口页面打开后取一次设置。
#[tauri::command]
pub fn overlay_hello(state: State<'_, AppState>) -> OverlaySettings {
    state.settings().overlay()
}

/// 透明窗口报告自己有没有东西在画。
#[tauri::command]
pub fn overlay_idle(window: WebviewWindow, state: State<'_, AppState>, idle: bool) {
    state.overlays.send(Msg::Idle {
        label: window.label().to_string(),
        idle,
    });
}
