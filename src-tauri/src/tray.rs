//! 托盘（需求 4.8）：单击图标打开控制面板；菜单是「信号开启」「对线模式」（勾选）、「打开控制面板」、
//! 「检查更新」（查到新版本时变成「更新到 vX」）、「退出」。
//! 只在 Rust 侧，不给前端开权限。

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::api;

const TRAY_ID: &str = "main";
const TOOLTIP: &str = "钢一下";

/// 「信号开启」这一项，开关变了要跟着勾上或取消。
struct PingsItem(CheckMenuItem<Wry>);
/// 「对线模式」这一项。
struct LaningItem(CheckMenuItem<Wry>);
/// 「检查更新」这一项。
struct UpdateItem(MenuItem<Wry>);

/// 把控制面板调到前面（托盘、再次打开程序都用它）。
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn setup(app: &tauri::App, enabled: bool) -> tauri::Result<()> {
    let pings = CheckMenuItem::with_id(app, "pings", "信号开启", true, enabled, None::<&str>)?;
    let laning = CheckMenuItem::with_id(app, "laning", "对线模式", true, false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "打开控制面板", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "检查更新", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &pings,
            &laning,
            &open,
            &update,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    app.manage(PingsItem(pings));
    app.manage(LaningItem(laning));
    app.manage(UpdateItem(update));
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "pings" => {
                let on = app
                    .try_state::<api::AppState>()
                    .is_some_and(|s| !s.enabled.load(std::sync::atomic::Ordering::Relaxed));
                api::set_enabled(app, on);
            }
            "laning" => {
                if let Some(s) = app.try_state::<api::AppState>() {
                    s.lane.send(crate::lane::LaneMsg::Set(None));
                }
            }
            "open" => show_main(app),
            // 打开「关于」页；还没查到新版本就顺便查一次
            "update" => {
                crate::update::tray_show_about(app);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    crate::update::check(&app, false).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn set_checked(app: &AppHandle, on: bool) {
    if let Some(item) = app.try_state::<PingsItem>() {
        let _ = item.0.set_checked(on);
    }
}

/// 查到新版本时「检查更新」变成「更新到 vX」，没有时变回来。
pub fn set_update_label(app: &AppHandle, version: Option<&str>) {
    if let Some(item) = app.try_state::<UpdateItem>() {
        let text = version.map_or_else(|| "检查更新".to_string(), |v| format!("更新到 v{v}"));
        let _ = item.0.set_text(text);
    }
}

pub fn set_laning_checked(app: &AppHandle, on: bool) {
    if let Some(item) = app.try_state::<LaningItem>() {
        let _ = item.0.set_checked(on);
    }
}
