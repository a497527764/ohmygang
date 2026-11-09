//! 托盘（需求 4.8）：单击图标打开控制面板；菜单是「信号开启」（勾选）、「打开控制面板」、
//! 「检查更新」（查到新版本时变成「更新到 vX」）、「退出」。
//! 只在 Rust 侧，不给前端开权限。

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::api;

const TRAY_ID: &str = "main";
const TOOLTIP: &str = "lolGang";

/// 「信号开启」这一项，开关变了要跟着勾上或取消。
struct PingsItem(CheckMenuItem<Wry>);
/// 「检查更新」这一项。
struct UpdateItem(MenuItem<Wry>);

/// 把控制面板调到前面（托盘、再次打开程序都用它）。
pub fn show_main(app: &AppHandle) {
    show_page(app, None);
}

/// 把控制面板调到前面，`page` 是要打开的那一页。关掉过的控制面板网页已经销毁（3.0-M6），按配置重新建一个。
pub fn show_page(app: &AppHandle, page: Option<&'static str>) {
    // macOS：控制面板开着时程序坞里才有图标（关掉时藏起来，见 lib.rs）
    #[cfg(target_os = "macos")]
    let _ = app.set_dock_visibility(true);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(p) = page {
            let _ = app.emit_to("main", "nav://page", p);
        }
        return;
    }
    let app = app.clone();
    // Windows 上在事件回调里同步建窗口可能卡住（Tauri 的已知问题），放到别的线程去建。
    std::thread::spawn(move || {
        let Some(mut cfg) = app.config().app.windows.first().cloned() else {
            return;
        };
        if let Some(p) = page {
            cfg.url = WebviewUrl::App(format!("index.html#{p}").into());
        }
        match WebviewWindowBuilder::from_config(&app, &cfg).and_then(|b| b.build()) {
            Ok(w) => {
                let _ = w.set_focus();
                if let Some(state) = app.try_state::<api::AppState>() {
                    let theme = state
                        .settings
                        .lock()
                        .map(|s| s.theme.clone())
                        .unwrap_or_default();
                    api::apply_window_theme(&app, &theme);
                }
            }
            Err(e) => eprintln!("控制面板建不起来：{e}"),
        }
    });
}

pub fn setup(app: &tauri::App, enabled: bool) -> tauri::Result<()> {
    let pings = CheckMenuItem::with_id(app, "pings", "信号开启", true, enabled, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "打开控制面板", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "检查更新", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &pings,
            &open,
            &update,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    app.manage(PingsItem(pings));
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
