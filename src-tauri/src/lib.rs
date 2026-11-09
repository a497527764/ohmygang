pub mod api;
pub mod app_core;
pub mod base32;
pub mod crypto;
pub mod emotes;
pub mod error;
pub mod history;
pub mod identity;
pub mod input;
pub mod keystore;
pub mod node;
pub mod overlay;
pub mod panel;
pub mod protocol;
pub mod rooms;
pub mod settings2;
pub mod storage;
pub mod sync;
pub mod team;
pub mod team_text;
pub mod transport;
pub mod tray;
pub mod update;
pub mod voice;
pub mod votes;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Manager, WindowEvent};

use api::AppState;
use overlay::{Hooks, Msg, Overlays};
use settings2::Hotkey;

/// 开机自动启动时带的参数：这时只进托盘，不打开控制面板。
pub const AUTOSTART_ARG: &str = "--autostart";

/// 开发用的单机多开：环境变量 `LOLGANG_INSTANCE=2` 或 `3` 表示第二、第三实例，数据目录换一套。
/// 第一实例和装好的正式版共用数据目录（同一个身份、同一个队伍），所以两个实例互测要用 2 和 3。
pub fn dev_instance() -> Option<u16> {
    match std::env::var("LOLGANG_INSTANCE").as_deref() {
        Ok("2") => Some(2),
        Ok("3") => Some(3),
        _ => None,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();
    // 再次打开程序时不起新进程，而是把已经打开的控制面板调到前面。
    // 开发用的第二、第三实例不注册，单机多开不受影响。这个插件只在 Rust 侧检查，不给前端任何权限。
    if dev_instance().is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app)
        }));
    }
    builder
        // 开机启动（Windows 注册表 Run、macOS 登录项、Linux 自启动目录）；只在 Rust 侧用，不给前端权限。
        // 自动更新：只在 Rust 侧用（update.rs），不给前端权限；公钥和地址在 tauri.conf.json 的 plugins.updater。
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .setup(|app| {
            start(app)?;
            Ok(())
        })
        // 导入的表情和收到的队友表情：emote://localhost/<哈希>.<webp|anim|sound>（Windows 上是 http://emote.localhost/…）。
        // 只认这种名字，别的一律 404；读文件放到别的线程，不卡主线程。
        .register_asynchronous_uri_scheme_protocol("emote", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let name = request.uri().path().trim_start_matches('/').to_string();
            std::thread::spawn(move || {
                let found = app
                    .try_state::<AppState>()
                    .and_then(|state| state.emotes.read(&name));
                let response = match found {
                    Some((bytes, mime)) => tauri::http::Response::builder()
                        .header("Content-Type", mime)
                        .header("Access-Control-Allow-Origin", "*")
                        .header("Cache-Control", "no-store")
                        .body(bytes),
                    None => tauri::http::Response::builder()
                        .status(404)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(Vec::new()),
                };
                if let Ok(r) = response {
                    responder.respond(r);
                }
            });
        })
        // 点控制面板的关闭按钮只收进托盘，信号照常工作；真正退出用托盘菜单的「退出」。
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                let _ = window.hide();
                api.prevent_close();
                // macOS：控制面板关了，程序坞里不显示图标（需求 4.8）
                #[cfg(target_os = "macos")]
                let _ = window.app_handle().set_dock_visibility(false);
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
            api::emote_import,
            api::emote_remove,
            api::emote_preview,
            api::open_accessibility_settings,
            api::app_relaunch,
            api::voice_view,
            api::voice_test,
            api::history_list,
            api::history_play,
            api::history_stop,
            api::text_preview,
            panel::vote_view,
            panel::vote_start,
            panel::vote_answer,
            panel::text_send,
            panel::text_cancel,
            update::update_view,
            update::update_check,
            update::update_install,
            team::team_view,
            team::team_create,
            team::team_join,
            team::team_cancel,
            team::team_leave,
            team::team_dissolve,
            team::team_rename,
            team::profile_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 启动流程：在应用数据目录打开 `gang2.db`、读设置 → 从凭据管理器取主密钥、读出身份，启动节点（局域网 + 中继）
/// → 启动透明窗口线程 → 启动键鼠监听（把手势事件交给透明窗口线程）→ 交给 Tauri 管理 → 托盘 → 控制面板的主题
/// → 在线状态每 5 秒看一次（离开电脑要靠时间判断）。键鼠监听起不来时程序照常打开，控制面板显示原因。
///
/// 单机多开（开发用）：环境变量 `LOLGANG_INSTANCE=2` 或 `3` 时，数据目录、凭据条目、UDP 端口都换一套；
/// 开发版的广播同时发往这几个端口，几个实例就能在一台电脑上互相发现。
fn start(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let mut dir = app.path().app_data_dir()?;
    if let Some(n) = dev_instance() {
        dir.push(format!("instance-{n}"));
    }
    std::fs::create_dir_all(&dir)?;
    let db = storage::open(&dir.join("gang2.db"))?;
    let settings = storage::settings::load(&db)?;
    let node = start_node(app.handle(), &dir, &settings)?;
    let me = tauri::async_runtime::block_on(node.call(|core, _| core.identity().member_id))
        .ok_or("网络模块没起来")?;
    let history = history::History::start(app.handle(), storage::open(&dir.join("gang2.db"))?, me);
    let own_live = history.clone();

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
    let panel = panel::Panel::start(app.handle());
    let voice = voice::Voice::start(app.handle(), settings.voice_config());
    let talk_to = voice.clone();
    let share = node.clone();
    let (text_to, panel_to) = (panel.clone(), panel.clone());
    let hooks = Hooks {
        on_toggled,
        on_live: Box::new(move |body| {
            own_live.own_live(&body);
            share.send_live(body);
        }),
        on_text: Box::new(move || text_to.send(panel::PanelMsg::TextRequested)),
        on_talk: Box::new(move |p| talk_to.send(voice::VoiceMsg::Talk(p))),
        on_panel: Box::new(move |show| panel_to.send(panel::PanelMsg::Show(show))),
    };
    let overlays = Overlays::start(app.handle(), settings.overlay(), hooks);
    let sink_to = overlays.clone();
    let (input, input_error) = match input::start(
        settings.gesture_config(true, false),
        Box::new(move |e| sink_to.send(Msg::Input(e))),
    ) {
        Ok(h) => (Some(h), None),
        Err(e) => (None, Some(e)),
    };
    let theme = settings.theme.clone();
    let emote_store = Arc::new(emotes::store::Store::open(&dir));
    let emote_net = emotes::net::EmoteNet::start(app.handle(), emote_store.clone());
    app.manage(AppState {
        db: Mutex::new(db),
        settings: Mutex::new(settings),
        enabled: AtomicBool::new(true),
        input,
        input_error,
        overlays,
        node,
        team: Default::default(),
        emotes: emote_store,
        emote_net,
        panel,
        updates: Default::default(),
        voice,
        history,
    });
    tray::setup(app, true)?;
    api::apply_window_theme(app.handle(), &theme);
    team::refresh_names(app.handle());
    update::start_schedule(app.handle());
    api::ensure_autostart(app.handle());
    if std::env::args().any(|a| a == AUTOSTART_ARG)
        && let Some(w) = app.get_webview_window("main")
    {
        w.hide()?;
        #[cfg(target_os = "macos")]
        app.handle().set_dock_visibility(false)?;
    }
    let presence = app.handle().clone();
    std::thread::Builder::new()
        .name("presence".into())
        .spawn(move || {
            loop {
                team::refresh_presence(&presence);
                std::thread::sleep(Duration::from_secs(5));
            }
        })?;
    Ok(())
}

fn start_node(
    app: &tauri::AppHandle,
    dir: &std::path::Path,
    settings: &settings2::Settings,
) -> Result<node::NodeHandle, Box<dyn std::error::Error>> {
    let instance = dev_instance();
    let keystore = match instance {
        Some(2) => keystore::SystemKeyStore {
            service: "lolgang-2",
        },
        Some(_) => keystore::SystemKeyStore {
            service: "lolgang-3",
        },
        None => keystore::APP_KEYSTORE,
    };
    let core = app_core::Core::open(storage::open(&dir.join("gang2.db"))?, &keystore)?;
    let mut cfg = node::NodeConfig {
        relay_urls: settings.active_relays(),
        ..node::NodeConfig::default()
    };
    if let Some(n) = instance {
        cfg.lan.bind.set_port(transport::lan::LAN_PORT + n - 1);
    }
    if cfg!(debug_assertions) {
        cfg.lan.broadcast_ports = (0..3).map(|i| transport::lan::LAN_PORT + i).collect();
    }
    let handle = app.clone();
    let sink: node::Sink = Arc::new(move |e| team::on_node_event(&handle, e));
    Ok(tauri::async_runtime::block_on(node::Node::spawn(
        core, cfg, sink,
    ))?)
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
