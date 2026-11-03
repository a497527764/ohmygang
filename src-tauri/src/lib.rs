pub mod api;
pub mod app_core;
pub mod base32;
pub mod crypto;
pub mod error;
pub mod identity;
pub mod input;
pub mod keystore;
pub mod node;
pub mod overlay;
pub mod protocol;
pub mod rooms;
pub mod settings2;
pub mod storage;
pub mod sync;
pub mod transport;
pub mod tray;

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use tauri::{Manager, WindowEvent};

use api::AppState;
use overlay::{Msg, Overlays};
use settings2::Hotkey;

/// 开发用的单机双开：环境变量 `LOLGANG_INSTANCE=2` 表示第二实例，数据目录换一套。
fn is_second_instance() -> bool {
    std::env::var("LOLGANG_INSTANCE").is_ok_and(|v| v == "2")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();
    // 再次打开程序时不起新进程，而是把已经打开的控制面板调到前面。
    // 第二实例不注册，单机双开不受影响。这个插件只在 Rust 侧检查，不给前端任何权限。
    if !is_second_instance() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app)
        }));
    }
    builder
        .setup(|app| {
            start(app)?;
            Ok(())
        })
        // 点控制面板的关闭按钮只收进托盘，信号照常工作；真正退出用托盘菜单的「退出」。
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            api::settings_read,
            api::settings_write,
            api::status_get,
            api::pings_set_enabled,
            api::input_suspend,
            api::ping_preview,
            api::overlay_hello,
            api::overlay_idle,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 启动流程：在应用数据目录打开 `gang2.db`、读设置 → 启动透明窗口线程 → 启动键鼠监听（把手势事件交给透明窗口线程）
/// → 交给 Tauri 管理 → 托盘 → 控制面板的主题。键鼠监听起不来时程序照常打开，控制面板显示原因。
fn start(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let mut dir = app.path().app_data_dir()?;
    if is_second_instance() {
        dir.push("instance-2");
    }
    std::fs::create_dir_all(&dir)?;
    let db = storage::open(&dir.join("gang2.db"))?;
    let settings = storage::settings::load(&db)?;

    let handle = app.handle().clone();
    let on_toggled = Box::new(move |on: bool| {
        api::after_toggle(&handle, on);
        if let Some(state) = handle.try_state::<AppState>() {
            let hotkey = state.settings.lock().map(|s| s.toggle_hotkey).ok();
            let body = hotkey
                .map(|h| format!("按 {} 切换", hotkey_label(h)))
                .unwrap_or_default();
            let title = if on {
                "信号已恢复"
            } else {
                "信号已暂停"
            };
            state.overlays.send(Msg::Toast {
                title: title.into(),
                body,
            });
        }
    });
    let overlays = Overlays::start(app.handle(), settings.overlay(), on_toggled);
    let sink_to = overlays.clone();
    let (input, input_error) = match input::start(
        settings.gesture_config(true),
        Box::new(move |e| sink_to.send(Msg::Input(e))),
    ) {
        Ok(h) => (Some(h), None),
        Err(e) => (None, Some(e)),
    };
    let theme = settings.theme.clone();
    app.manage(AppState {
        db: Mutex::new(db),
        settings: Mutex::new(settings),
        enabled: AtomicBool::new(true),
        input,
        input_error,
        overlays,
    });
    tray::setup(app, true)?;
    api::apply_window_theme(app.handle(), &theme);
    Ok(())
}

/// 「Ctrl + Alt + P」这样的快捷键文字（提示里用）。
fn hotkey_label(h: Hotkey) -> String {
    let mut parts: Vec<String> = [(1, "Ctrl"), (2, "Alt"), (4, "Shift"), (8, "Win")]
        .iter()
        .filter(|(bit, _)| h.mods & bit != 0)
        .map(|(_, name)| (*name).to_string())
        .collect();
    let key = match h.vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(h.vk).map(String::from).unwrap_or_default(),
        0x70..=0x87 => format!("F{}", h.vk - 0x6F),
        vk => format!("键 0x{vk:02X}"),
    };
    parts.push(key);
    parts.join(" + ")
}
