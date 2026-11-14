pub mod api;
pub mod app_core;
pub mod base32;
pub mod cloud;
pub mod crypto;
pub mod datadir;
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

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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

/// 节点、中继、局域网和 Tauri 的异步命令跑在这里。Tauri 默认一个逻辑核一个工作线程（32 核的电脑就是 32 个），
/// 这些任务几乎都在等网络，2 个足够（3.2.0）。Tauri 只拿句柄，运行时本身放在静态变量里活到程序退出。
fn use_small_async_runtime() {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let rt = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lolgang-async")
            .enable_all()
            .build()
            .expect("能建异步运行时")
    });
    tauri::async_runtime::set(rt.handle().clone());
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use_small_async_runtime();
    let mut builder = tauri::Builder::default();
    // 再次打开程序时不起新进程，而是把已经打开的控制面板调到前面。
    // 开发用的第二、第三实例不注册，单机多开不受影响。这个插件只在 Rust 侧检查，不给前端任何权限。
    if dev_instance().is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app)
        }));
    }
    let mut app = builder
        // 开机启动（Windows 注册表 Run、macOS 登录项）；只在 Rust 侧用，不给前端权限。
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
        // 只认这种名字，别的一律 404。读文件放进 Tauri 的阻塞线程池（3.1.1：以前每个请求新开一个系统线程），不卡主线程。
        // 不给长缓存：3.1.1 试过，WebView2 不缓存自定义协议的回应，缓存头不起作用；同一个页面里同一张图浏览器本来就复用。
        .register_asynchronous_uri_scheme_protocol("emote", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let name = request.uri().path().trim_start_matches('/').to_string();
            tauri::async_runtime::spawn_blocking(move || {
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
        // 点控制面板的关闭按钮：信号照常工作，控制面板的网页销毁（省下一个网页进程的内存和 CPU，3.0-M6），
        // 下次打开时再建（`tray::show_main`）。真正退出用托盘菜单的「退出」。
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { .. } = event
                && window.label() == "main"
            {
                // 记下位置和大小，下次打开时还原（3.1.1）
                tray::remember_main(window);
                // 正在录按键时关掉：页面来不及恢复拦截，这里替它恢复
                if let Some(state) = window.app_handle().try_state::<AppState>()
                    && let Some(input) = &state.input
                {
                    input.send(input::Command::Suspended(false));
                }
                // macOS：控制面板关了，程序坞里不显示图标（需求 4.8）
                #[cfg(target_os = "macos")]
                let _ = window.app_handle().set_dock_visibility(false);
            }
        })
        .invoke_handler(tauri::generate_handler![
            api::ui_sync_window,
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
            panel::vote::vote_view,
            panel::vote::vote_start,
            panel::vote::vote_answer,
            panel::text::text_send,
            panel::text::text_cancel,
            update::update_view,
            update::update_check,
            update::update_install,
            team::commands::team_view,
            team::commands::team_recent,
            team::commands::team_create,
            team::commands::team_join,
            team::commands::team_cancel,
            team::commands::team_leave,
            team::commands::team_forget,
            team::commands::team_moderate,
            team::commands::team_dissolve,
            team::commands::team_rename,
            team::commands::profile_set,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    // 不要窗口库的「设备事件」（原始鼠标、键盘输入，3.2.0）：我们一个都不用（手势走自己的钩子），
    // 订着的话，鼠标在别的程序上每动一下，主线程也要收一条消息（量过 10 秒 2500 次移动占主线程 190–300 毫秒）
    app.set_device_event_filter(tauri::DeviceEventFilter::Always);
    app.run(|_app, event| {
        // 控制面板关掉后还有透明窗口；万一窗口全关了也不退出。托盘的「退出」带退出码，照常退出。
        if let tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
        }
    });
}

/// 启动流程：在应用数据目录打开 `gang2.db`、读设置 → 从凭据管理器取主密钥、读出身份，启动节点（局域网 + 中继）
/// → 启动透明窗口线程 → 启动键鼠监听（把手势事件交给透明窗口线程）→ 交给 Tauri 管理 → 托盘 → 控制面板的主题
/// → 在线状态每 5 秒看一次（离开电脑要靠时间判断）。键鼠监听起不来时程序照常打开，控制面板显示原因。
///
/// 单机多开（开发用）：环境变量 `LOLGANG_INSTANCE=2` 或 `3` 时，数据目录、凭据条目、UDP 端口都换一套；
/// 开发版的广播同时发往这几个端口，几个实例就能在一台电脑上互相发现。
fn start(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // 数据目录用产品名目录（%APPDATA%\OH-MY-GANG!），不用 Tauri 的 app_data_dir：
    // 安装包的「删除应用数据」勾选框删的是 identifier 目录，会把身份、昵称、头像一起删光（见 datadir.rs）。
    let mut dir = datadir::resolve(app.handle());
    if let Some(n) = dev_instance() {
        dir.push(format!("instance-{n}"));
    }
    std::fs::create_dir_all(&dir)?;
    let db = storage::open(&dir.join("gang2.db"))?;
    // 身份表空着（数据目录被清过）→ 先从备份恢复，别变成一个全新的人。
    if storage::identity::restore_from_backup(&db, &dir) {
        eprintln!("身份表是空的，已从备份恢复本机身份");
    }
    let mut settings = storage::settings::load(&db)?;
    // 一次性迁移：旧的自动字号挪到新默认 50（用户自己调过的值不动），迁完存一次。
    if let Some(px) = settings2::migrate_text_size(settings.text_size_px) {
        settings.text_size_px = px;
        if let Err(e) = storage::settings::save(&db, &settings) {
            eprintln!("字号迁移没存上：{e}");
        }
    }
    // 第一次打开（还没存过设置）：按主显示器分辨率定字号和表情大小（字号一律 50，表情 4K 300、2K 250、1080p 220）。
    if storage::settings::is_fresh(&db)?
        && let Some(m) = app.primary_monitor().ok().flatten()
    {
        let (text, emote) = settings2::first_launch_sizes(m.size().height);
        settings.text_size_px = text;
        settings.emote_size_px = emote;
    }
    // 控制面板建好时藏着：开机启动就一直不显示（后面销毁），否则回到上次关掉时的位置再显示
    let autostart = std::env::args().any(|a| a == AUTOSTART_ARG);
    if !autostart && let Some(w) = app.get_webview_window("main") {
        tray::reveal_main(app.handle(), &w, settings.panel_rect);
    }
    let node = start_node(app.handle(), &dir, &settings)?;
    // 身份备份：数据库文件丢了还能恢复
    api::backup_identity(app.handle());
    cloud::spawn_heartbeat(node.clone());
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
    // 3.3：启动是否自动上线由设置决定（默认开）。false 时键鼠监听带着 enabled=false 起来，
    // 托盘勾选和状态也从这里同步，开关/快捷键恢复的路径不变。
    let auto_online = settings.auto_online;
    let (input, input_error) = match input::start(
        settings.gesture_config(auto_online, false),
        Box::new(move |e| sink_to.send(Msg::Input(e))),
    ) {
        Ok(h) => (Some(h), None),
        Err(e) => (None, Some(e)),
    };
    let theme = settings.theme.clone();
    let auto_rejoin = settings.auto_rejoin;
    let emote_store = Arc::new(emotes::store::Store::open(&dir));
    let emote_net = emotes::net::EmoteNet::start(app.handle(), emote_store.clone());
    app.manage(AppState {
        db: Mutex::new(db),
        settings: Mutex::new(settings),
        enabled: AtomicBool::new(auto_online),
        input,
        input_error,
        // 启动先当「连接中」：中继还在连，头 8 秒不报「未连接」；在线状态线程会纠正
        cloud_state: AtomicU8::new(api::CLOUD_CONNECTING),
        cloud_miss: AtomicU8::new(0),
        cloud_last_ok_ms: AtomicU64::new(0),
        cloud_rtt_ms: AtomicU64::new(0),
        started_at: Instant::now(),
        overlays,
        node,
        team: Default::default(),
        emotes: emote_store,
        emote_net,
        panel,
        updates: Default::default(),
        voice,
        history,
        dir: dir.clone(),
    });
    tray::setup(app, auto_online)?;
    api::apply_window_theme(app.handle(), &theme);
    team::refresh_names(app.handle());
    update::start_schedule(app.handle());
    api::ensure_autostart(app.handle());
    if autostart && let Some(w) = app.get_webview_window("main") {
        // 开机启动只进托盘：控制面板的网页不建（销毁），点托盘时再建
        w.destroy()?;
        #[cfg(target_os = "macos")]
        app.handle().set_dock_visibility(false)?;
    }
    // 本人头像的哈希读进缓存（`refresh_presence` 只读缓存，改资料时 `profile_set` 再写）
    if let Some(state) = app.try_state::<AppState>() {
        let node = state.node.clone();
        let hash = tauri::async_runtime::block_on(
            node.call(|core, _| core.identity().avatar_hash.clone()),
        )
        .flatten();
        *state.team.avatar_hash.lock().expect("头像哈希锁没有中毒") = hash;
    }
    let presence = app.handle().clone();
    std::thread::Builder::new()
        .name("presence".into())
        .spawn(move || {
            loop {
                team::refresh_presence(&presence);
                team::watch(&presence);
                std::thread::sleep(Duration::from_secs(5));
            }
        })?;
    // 3.3：启动自动回到上一次的队伍（设置里可关）。等几秒让节点把中继连上；
    // join 只是本地记一笔加等发现，队伍不在了就停在「加入中」，主页历史里那一行还能手动重试。
    if auto_rejoin {
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            team::commands::rejoin_last(&handle).await;
        });
    }
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
    let cfg = node::NodeConfig {
        relay_urls: settings.relay_list(),
        ..node::NodeConfig::default()
    };
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

/// 权限清单的一致性测试。
///
/// Tauri 2 的权限是白名单：前端 `invoke("xxx")` 的命令，必须同时满足三件事才会被放行——
/// ① 在 lib.rs 的 `generate_handler!` 里注册；② 在 build.rs 的 `COMMANDS` 里列了
/// （tauri-build 才会给它生成 `allow-xxx` 权限文件）；③ 被某个 capabilities/*.json 放行。
/// 少任何一环，运行时就是一句 `Command xxx not allowed by ACL`，界面上的功能直接罢工。
///
/// 3.2.9 实测踩过这个坑：队伍历史（team_recent）、删历史记录（team_forget）、
/// 踢人拉黑（team_moderate）、窗口缩放（ui_sync_window）四个命令都只注册了、
/// 忘了补权限，用户点一下就报错。这几个测试就是防止它再发生。
#[cfg(test)]
mod acl_tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// src-tauri 目录。
    fn manifest_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
    }

    /// lib.rs 的 `generate_handler![...]` 里注册的命令名。
    fn registered_commands() -> BTreeSet<String> {
        let src =
            std::fs::read_to_string(manifest_dir().join("src/lib.rs")).expect("读不到 src/lib.rs");
        let marker = "generate_handler![";
        let start = src
            .find(marker)
            .expect("src/lib.rs 里没有 generate_handler![]");
        let rest = &src[start + marker.len()..];
        let end = rest
            .find(']')
            .expect("generate_handler![] 里的方括号没闭合");
        rest[..end]
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            // 注册时写的是 `team::commands::team_recent`，命令名是最后一段
            .map(|s| s.rsplit("::").next().unwrap_or(s).to_string())
            .collect()
    }

    /// build.rs 的 COMMANDS 清单（权限清单的源头）。
    fn manifest_commands() -> BTreeSet<String> {
        let src =
            std::fs::read_to_string(manifest_dir().join("build.rs")).expect("读不到 build.rs");
        let start = src
            .find("const COMMANDS")
            .expect("build.rs 里没有 COMMANDS");
        let rest = &src[start..];
        // 数组是 `= &[` 开头的那种（类型标注里的 `&[&str]` 不算数）
        let open = rest.find("= &[").expect("COMMANDS 的数组没找到") + 3;
        let close = rest[open..].find(']').expect("COMMANDS 的方括号没闭合");
        rest[open + 1..open + close]
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_matches('"').to_string())
            .collect()
    }

    /// capabilities/*.json 里所有放行过的权限标识。
    fn allowed_permissions() -> BTreeSet<String> {
        let dir = manifest_dir().join("capabilities");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("读不到 capabilities 目录")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        files.sort();
        let mut out = BTreeSet::new();
        for f in files {
            let text = std::fs::read_to_string(&f).expect("读不了 capabilities 文件");
            let v: serde_json::Value =
                serde_json::from_str(&text).expect("capabilities 文件不是合法 JSON");
            if let Some(list) = v.get("permissions").and_then(|p| p.as_array()) {
                for p in list {
                    if let Some(s) = p.as_str() {
                        out.insert(s.to_string());
                    }
                }
            }
        }
        out
    }

    /// 前端 invoke 了哪些命令（仓库根的 src/ 下所有 .ts/.tsx）。
    fn invoked_commands() -> BTreeSet<String> {
        fn walk(dir: &Path, out: &mut BTreeSet<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for e in entries.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                    continue;
                }
                let is_ts = p
                    .extension()
                    .map(|x| x == "ts" || x == "tsx")
                    .unwrap_or(false);
                if !is_ts {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&p) else {
                    continue;
                };
                // 扫出所有 invoke("命令名" 的写法：invoke("x")、invoke<number>("x", ...)
                let bytes = text.as_bytes();
                let mut rest = text.as_str();
                while let Some(pos) = rest.find("invoke") {
                    let mut i = pos + "invoke".len();
                    // 跳过空白
                    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                        i += 1;
                    }
                    // 可能有泛型参数：invoke<number>("x", ...)
                    if i < bytes.len() && bytes[i] == b'<' {
                        if let Some(gt) = rest[i..].find('>') {
                            i += gt + 1;
                        }
                        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                            i += 1;
                        }
                    }
                    // 必须是 invoke( 才算调用（注释里出现「invoke」不算）
                    if i >= bytes.len() || bytes[i] != b'(' {
                        rest = &rest[pos + "invoke".len()..];
                        continue;
                    }
                    i += 1;
                    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                        i += 1;
                    }
                    if i >= bytes.len() || bytes[i] != b'"' {
                        rest = &rest[i..];
                        continue;
                    }
                    let start = i + 1;
                    let Some(end) = rest[start..].find('"') else {
                        break;
                    };
                    out.insert(rest[start..start + end].to_string());
                    rest = &rest[start + end..];
                }
            }
        }
        let mut out = BTreeSet::new();
        walk(&manifest_dir().join("../src"), &mut out);
        out
    }

    #[test]
    fn invoked_commands_are_registered() {
        let registered = registered_commands();
        let invoked = invoked_commands();
        let missing: Vec<&String> = invoked
            .iter()
            .filter(|c| !registered.contains(*c))
            .collect();
        assert!(
            missing.is_empty(),
            "前端调用了 lib.rs 里没注册的命令：{missing:?}"
        );
    }

    #[test]
    fn registered_commands_are_in_the_permission_manifest() {
        let manifest = manifest_commands();
        let registered = registered_commands();
        let missing: Vec<&String> = registered
            .iter()
            .filter(|c| !manifest.contains(*c))
            .collect();
        assert!(
            missing.is_empty(),
            "这些命令注册了但没进 build.rs 的 COMMANDS，tauri-build 不会给它们生成 allow-* 权限：{missing:?}"
        );
    }

    #[test]
    fn every_command_is_allowed_by_some_capability() {
        let allowed = allowed_permissions();
        let manifest = manifest_commands();
        let missing: Vec<String> = manifest
            .iter()
            .filter(|c| !allowed.contains(&format!("allow-{}", c.replace('_', "-"))))
            .cloned()
            .collect();
        assert!(
            missing.is_empty(),
            "这些命令没有任何 capabilities 放行，前端一调就报 \"not allowed by ACL\"：{missing:?}"
        );
    }

    // ---- 真跑一遍 IPC 的那部分 ----
    //
    // 上面的几个测试是「看清单」，这里是「真调一次」：用真实的 tauri.conf.json 和
    // capabilities 建一个测试应用，发一条真的 IPC 请求，看 Tauri 的权限层放不放行。
    // 命令体是假的（同名的空壳），因为要验的是「这个命令名放不放行」，跟命令做什么无关。

    /// 同名的空壳命令：登记成 "team_recent"，好让权限层按这个名字去查放行名单。
    #[tauri::command]
    fn team_recent() -> &'static str {
        "ok"
    }

    /// 同名的空壳命令：登记成 "overlay_hello"。它只归透明窗口（overlay-*）用，
    /// 控制面板不该能调——拿来当反例，证明这个测试不是「怎么调都过」。
    #[tauri::command]
    fn overlay_hello() -> &'static str {
        "ok"
    }

    /// 以控制面板（main 窗口）的身份真的调一次命令。
    fn invoke_as_main_window(
        cmd: &str,
    ) -> Result<tauri::ipc::InvokeResponseBody, serde_json::Value> {
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![team_recent, overlay_hello])
            // 真配置（真 capabilities + 真命令权限清单），资源用空壳占位。
            // test = true：macOS 上 generate_context! 会往二进制里塞 Info.plist，
            // 这里第二次调用必须跳过，不然符号重复定义、macOS 上编译不过。
            .build(tauri::generate_context!(
                assets = tauri::test::noop_assets(),
                test = true
            ))
            .expect("建测试应用");
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("建测试窗口");
        tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: cmd.to_string(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                // 本地页面在 Windows/Android 是 http://tauri.localhost，
                // 在 macOS/Linux 是 tauri://localhost——权限是按「来源」判的，写错就调不通。
                url: if cfg!(any(windows, target_os = "android")) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        )
    }

    #[test]
    fn main_window_can_really_call_team_recent() {
        let res = invoke_as_main_window("team_recent");
        assert!(
            res.is_ok(),
            "控制面板调 team_recent 被权限层拦了（用户看到的「读队伍历史失败」）：{res:?}"
        );
    }

    #[test]
    fn main_window_cannot_call_overlay_commands() {
        let res = invoke_as_main_window("overlay_hello");
        let err = res.expect_err("控制面板不该能调透明窗口的命令");
        let msg = err.to_string();
        // 正式版（release）里 Tauri 报「Command xxx not allowed by ACL」，
        // 开发版报得细一点（会列出这个命令只归哪些窗口用）。
        assert!(
            msg.contains("not allowed"),
            "反例不对：控制面板竟然能调 overlay_hello：{msg}"
        );
    }
}
