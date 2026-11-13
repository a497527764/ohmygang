//! 2.0 的程序状态和命令：控制面板和透明窗口通过 `invoke` 调这里。
//!
//! 实现流程：
//! 1. 启动时（`lib.rs`）开 `gang2.db`、读设置，建 `AppState` 交给 Tauri 管理。
//! 2. 改设置：整份设置过 `normalize` → 存库 → 交给输入线程（手势配置）、透明窗口线程（画画的设置）、语音线程 → 主窗口换主题。
//! 3. 信号开关（托盘、控制面板、暂停快捷键）统一走 `set_enabled`：告诉输入线程、勾上托盘、通知控制面板、更新在线状态。
//!    改了「允许互联网」或中继列表时，让节点换中继。
//! 4. 这些命令都很快（读写一行 SQLite、往通道里放消息），而且持锁时不等别的线程，所以放在主线程上执行没问题。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::Instant;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::input::{Command, InputHandle};
use crate::node::NodeHandle;
use crate::overlay::{Msg, Overlays};
use crate::settings2::{self, OverlaySettings, Settings};
use crate::team::{self, TeamState};
use crate::{emotes, storage, tray};

pub struct AppState {
    pub db: Mutex<Connection>,
    pub settings: Mutex<Settings>,
    /// 信号开关。不存进设置：每次启动都是开的。
    pub enabled: AtomicBool,
    /// 输入线程；启动失败时是 None，`input_error` 说明原因。
    pub input: Option<InputHandle>,
    pub input_error: Option<String>,
    /// 云端连接状态：0=连接中（刚启动，还没判定）、1=连上了、2=连不上。在线状态线程每 5 秒刷新。
    pub cloud_state: AtomicU8,
    /// 连续几次没探到中继（迟滞用：连续两次才算断，免得状态来回跳）。
    pub cloud_miss: AtomicU8,
    /// 中继往返时间（毫秒）；0 表示还没有。
    pub cloud_rtt_ms: AtomicU64,
    /// 启动时刻：头 8 秒里连不上也算「连接中」，不急着报「未连接」。
    pub started_at: Instant,
    pub overlays: Overlays,
    /// 队伍的网络（局域网 + 中继）。
    pub node: NodeHandle,
    pub team: TeamState,
    /// 导入的表情和收到的队友表情。
    pub emotes: std::sync::Arc<crate::emotes::store::Store>,
    pub emote_net: crate::emotes::net::EmoteNet,
    /// 队伍面板、投票、文字。
    pub panel: crate::panel::Panel,
    /// 自动更新的状态。
    pub updates: crate::update::Updates,
    /// 语音（按住说话）。
    pub voice: crate::voice::Voice,
    /// 历史记录（聊天框里的文字、语音、信号、表情）。
    pub history: crate::history::History,
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
    /// macOS 上监听没起来：多半是缺「辅助功能」权限，控制面板顶部显示授权卡片。
    pub needs_permission: bool,
    /// 云端服务器（中继所在的服务器）的连接状态：
    /// connecting（刚启动、还在连）/ ok（连上了）/ down（连不上）。
    /// 在线状态线程每 5 秒刷新，变化时推 "status://changed"。
    pub cloud_state: String,
    /// 中继的往返时间（毫秒）；没连上时是 null。用来一眼看出是不是网络/梯子拖慢了。
    pub relay_ms: Option<u64>,
}

/// 云端连接状态的三种取值（前端也要用）。
pub const CLOUD_CONNECTING: u8 = 0;
pub const CLOUD_OK: u8 = 1;
pub const CLOUD_DOWN: u8 = 2;

/// 云端状态的名字（给前端用）。
pub fn cloud_state_name(v: u8) -> &'static str {
    match v {
        CLOUD_OK => "ok",
        CLOUD_DOWN => "down",
        _ => "connecting",
    }
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
    team::refresh_presence(app);
}

/// 开机启动：只在设置改了时去改系统设置。开发用的第二、第三实例不碰。
fn set_autostart(app: &AppHandle, on: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    if crate::dev_instance().is_some() {
        return Ok(());
    }
    let launcher = app.autolaunch();
    let done = if on {
        launcher.enable()
    } else {
        launcher.disable()
    };
    done.map_err(|e| format!("开机启动没设置上：{e}"))
}

/// 设置里开着开机启动、系统里却没登记时补上（2.0.1 改了产品名，系统里登记的名字跟着变）。只在正式版里做：
/// 开发版登记的是调试用的程序。
pub fn ensure_autostart(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if cfg!(debug_assertions)
        || crate::dev_instance().is_some()
        || !state.settings().launch_at_startup
    {
        return;
    }
    let launcher = app.autolaunch();
    if !launcher.is_enabled().unwrap_or(true) {
        let _ = launcher.enable();
    }
}

/// 进出队伍后重新给手势配置（文字快捷键和面板键只在队伍里拦）。
pub fn push_gesture_config(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if let Some(input) = &state.input {
        input.send(Command::Config(state.settings().gesture_config(
            state.enabled.load(Ordering::Relaxed),
            state.team.active.load(Ordering::Relaxed),
        )));
    }
}

/// 主窗口标题栏跟着主题走（跟随系统时交给系统）。
pub fn apply_window_theme(app: &AppHandle, theme: &str) {
    // 六套主题按前缀决定原生窗口的明暗（标题栏、系统菜单），具体配色由前端 CSS 负责。
    let t = if theme.starts_with("light-") {
        Some(tauri::Theme::Light)
    } else if theme.starts_with("dark-") {
        Some(tauri::Theme::Dark)
    } else {
        None
    };
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_theme(t);
    }
}

/// 控制面板整体缩放：前端按 500×860 排版再整体 zoom=k，窗口逻辑尺寸同步变成 500k×860k。
/// k = 1.3 × 基准显示器缩放 / 当前显示器缩放 —— 窗口的物理大小不随显示器 DPI 变化；
/// 再按当前显示器的可用区域收口，1080P 这类小屏上等比缩小，窗口不会顶出屏幕。
#[tauri::command]
pub fn ui_sync_window(app: AppHandle, dpr: f64, w: f64, h: f64) -> f64 {
    const UI_SCALE: f64 = 1.3;
    let (w, h) = (w.max(1.0), h.max(1.0));
    let Some(win) = app.get_webview_window("main") else {
        return UI_SCALE;
    };
    let scale = win.scale_factor().unwrap_or(dpr).max(0.5);
    let mut k = UI_SCALE * dpr.clamp(0.5, 4.0) / scale;
    if let Ok(Some(m)) = win.current_monitor() {
        let s = m.size();
        k = k
            .min(s.height as f64 * 0.90 / scale / h)
            .min(s.width as f64 * 0.94 / scale / w);
    }
    k = k.clamp(0.7, 2.6);
    let _ = win.set_size(tauri::LogicalSize::new(w * k, h * k));
    k
}

#[tauri::command]
pub fn settings_read(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// 存好的设置，和这次被换掉的按键（快捷键页在那一行下面说换成了什么、和谁撞了）。
#[derive(Serialize)]
pub struct Saved {
    settings: Settings,
    clashes: Vec<settings2::KeyClash>,
}

/// 存一份新设置，返回修正后的设置（控制面板以它为准）。
#[tauri::command]
pub fn settings_write(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Value,
) -> Result<Saved, String> {
    let (s, clashes) = settings2::normalize_keys(&settings);
    let settings = apply_normalized(&app, &state, s)?;
    Ok(Saved { settings, clashes })
}

/// 记下控制面板的位置和大小（关面板时，`tray::remember_main`）。只改这一项，不用通知别的线程。
pub fn save_panel_rect(state: &AppState, rect: settings2::PanelRect) {
    let s = {
        let mut s = state.settings.lock().expect("设置锁没有中毒");
        if s.panel_rect == Some(rect) {
            return;
        }
        s.panel_rect = Some(rect);
        s.clone()
    };
    if let Err(e) = storage::settings::save(&state.db.lock().expect("数据库锁没有中毒"), &s)
    {
        eprintln!("控制面板的位置没存上：{e}");
    }
}

/// 整份设置过 `normalize` → 存库 → 交给各线程（见文件头第 2 条）。
fn apply_settings(app: &AppHandle, state: &AppState, settings: &Value) -> Result<Settings, String> {
    apply_normalized(app, state, settings2::normalize(settings))
}

fn apply_normalized(
    app: &AppHandle,
    state: &AppState,
    mut s: Settings,
) -> Result<Settings, String> {
    let old = state.settings();
    // 控制面板的位置只由关面板时记下（`save_panel_rect`），页面上存设置时带过来的是打开时的旧值
    s.panel_rect = old.panel_rect;
    if s.launch_at_startup != old.launch_at_startup {
        set_autostart(app, s.launch_at_startup)?;
    }
    let old_relays = old.relay_list();
    storage::settings::save(&state.db.lock().expect("数据库锁没有中毒"), &s)
        .map_err(|e| e.to_string())?;
    *state.settings.lock().expect("设置锁没有中毒") = s.clone();
    if let Some(input) = &state.input {
        input.send(Command::Config(s.gesture_config(
            state.enabled.load(Ordering::Relaxed),
            state.team.active.load(Ordering::Relaxed),
        )));
    }
    state.overlays.send(Msg::Settings(s.overlay()));
    state
        .voice
        .send(crate::voice::VoiceMsg::Config(s.voice_config()));
    apply_window_theme(app, &s.theme);
    team::refresh_presence(app); // 轮盘上的导入表情变了，报的哈希跟着变
    let relays = s.relay_list();
    if relays != old_relays {
        let node = state.node.clone();
        tauri::async_runtime::spawn(async move { node.set_relays(relays).await });
    }
    Ok(s)
}

#[tauri::command]
pub fn status_get(state: State<'_, AppState>) -> Status {
    let rtt = state.cloud_rtt_ms.load(Ordering::Relaxed);
    Status {
        enabled: state.enabled.load(Ordering::Relaxed),
        input_error: state.input_error.clone(),
        needs_permission: cfg!(target_os = "macos") && state.input_error.is_some(),
        cloud_state: cloud_state_name(state.cloud_state.load(Ordering::Relaxed)).to_string(),
        relay_ms: (rtt > 0).then_some(rtt),
    }
}

/// macOS：打开「系统设置 → 隐私与安全性 → 辅助功能」。
#[tauri::command]
pub fn open_accessibility_settings() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("只有 macOS 要授权".into());
    }
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打不开系统设置：{e}"))
}

/// 授权后重开程序：键鼠监听只在启动时建。
#[tauri::command]
pub fn app_relaunch(app: AppHandle) {
    app.restart();
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

/// 导入一个表情：静态图（控制面板缩好的 WebP）必有，动画、音效可选，都是 base64。
/// 文件检查过写进 `emotes/`，再加进设置里的导入列表（同一张图再导入一次就换掉原来那条）。
/// `replace_id` 传了就替换那一条（拖到已有表情上），不传就新增（拖到空白处）。
/// 名字后台自动生成，出场效果用全局设置里的 `emote_fx`。
#[tauri::command]
pub fn emote_import(
    app: AppHandle,
    state: State<'_, AppState>,
    still: String,
    anim: Option<String>,
    sound: Option<String>,
    replace_id: Option<String>,
) -> Result<Settings, String> {
    let decode = |s: &str| {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|_| "文件数据格式不对".to_string())
    };
    let still = decode(&still)?;
    let anim = anim.as_deref().map(decode).transpose()?;
    let sound = sound.as_deref().map(decode).transpose()?;
    let mut s = state.settings();
    let hash = emotes::store::hash_of(&still);
    let id = format!("c:{hash}");
    let fx = s.emote_fx;
    // 替换模式：老的那条不占新名额
    let replacing = replace_id
        .as_deref()
        .is_some_and(|rid| s.custom_emotes.iter().any(|c| c.id == rid));
    if s.custom_emotes.len() >= emotes::MAX_CUSTOM
        && !replacing
        && !s.custom_emotes.iter().any(|c| c.id == id)
    {
        return Err(emotes::store::ImportError::Full.to_string());
    }
    state
        .emotes
        .import_custom(&still, anim.as_deref(), sound.as_deref())
        .map_err(|e| e.to_string())?;
    let name = emotes::clean_name("", &hash);
    let entry = emotes::CustomEmote {
        id: id.clone(),
        name,
        animated: anim.is_some(),
        sound: sound.is_some(),
        fx,
    };
    // 替换掉的旧条目的文件先记下来，等设置写盘成功后再删——
    // 顺序反了的话，设置写入失败会让列表还挂着已删文件的条目。
    let mut stale_hash = None;
    if let Some(rid) = replace_id.as_deref() {
        // 拖到已有表情上：原地替换，轮盘引用跟着换
        if let Some(pos) = s.custom_emotes.iter().position(|c| c.id == rid) {
            if rid != id {
                for slot in &mut s.emote_wheel {
                    if slot == rid {
                        *slot = id.clone();
                    }
                }
                if s.click_emote == rid {
                    s.click_emote = id.clone();
                }
                stale_hash = emotes::custom_hash(rid);
            }
            s.custom_emotes[pos] = entry;
        } else {
            s.custom_emotes.push(entry);
        }
    } else {
        match s.custom_emotes.iter_mut().find(|c| c.id == id) {
            Some(c) => *c = entry,
            None => s.custom_emotes.push(entry),
        }
    }
    let raw = serde_json::to_value(&s).map_err(|e| e.to_string())?;
    let saved = apply_settings(&app, &state, &raw)?;
    if let Some(hash) = stale_hash {
        state.emotes.remove(hash);
    }
    Ok(saved)
}

/// 删掉一个导入的表情：先从设置里拿掉（轮盘上的那格换回默认），再删文件。
#[tauri::command]
pub fn emote_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<Settings, String> {
    let mut s = state.settings();
    s.custom_emotes.retain(|c| c.id != id);
    let raw = serde_json::to_value(&s).map_err(|e| e.to_string())?;
    let saved = apply_settings(&app, &state, &raw)?;
    if let Some(hash) = emotes::custom_hash(&id) {
        state.emotes.remove(hash);
    }
    Ok(saved)
}

/// 在主显示器中间放一个表情看看效果。
/// 语音页：麦克风列表和语音的状态（打不开的原因、试音进行到哪）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    pub mics: Vec<crate::voice::audio::MicInfo>,
    pub status: crate::voice::VoiceStatus,
}

#[tauri::command]
pub fn voice_view(state: State<'_, AppState>) -> VoiceView {
    VoiceView {
        mics: crate::voice::audio::mics(),
        status: state.voice.status(),
    }
}

/// 试音：录 3 秒放给自己听。进度经 "voice://changed" 推给控制面板。
#[tauri::command]
pub fn voice_test(state: State<'_, AppState>) {
    state.voice.send(crate::voice::VoiceMsg::Test);
}

/// 聊天框：现在所在队伍的历史记录，从旧到新。
/// 这个和下面的回放是异步命令（3.1.1）：等历史记录线程读库时不占主线程。
#[tauri::command]
pub async fn history_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::history::EntryDto>, String> {
    Ok(state.history.list().await)
}

/// 回放一条语音（只在本机放，像微信语音）。播放和停止经 "voice:playing" 推给聊天框。
#[tauri::command]
pub async fn history_play(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    if let Some(frames) = state.history.voice(id).await {
        state
            .voice
            .send(crate::voice::VoiceMsg::Play { id, frames });
    }
    Ok(())
}

#[tauri::command]
pub fn history_stop(state: State<'_, AppState>) {
    state.voice.send(crate::voice::VoiceMsg::StopPlay);
}

/// 功能页的预览：按页面上选的样子（还没存也行）在主显示器中间放一句字。
#[tauri::command]
pub fn text_preview(state: State<'_, AppState>, style: String, size: u32, effect: String) {
    let ok = settings2::TEXT_STYLES.contains(&style.as_str())
        && settings2::TEXT_EFFECTS.contains(&effect.as_str());
    if ok {
        state.overlays.send(Msg::TextPreview {
            style,
            size: size.clamp(16, 100),
            effect,
        });
    }
}

#[tauri::command]
pub fn emote_preview(state: State<'_, AppState>, emote: String) {
    if emotes::is_ref_shape(&emote) {
        state.overlays.send(Msg::EmotePreview(emote));
    }
}
