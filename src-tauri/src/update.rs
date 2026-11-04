//! 自动更新（2.0 架构第 9 节）：用 `tauri-plugin-updater` 从本项目 GitHub Release 的 `latest.json` 查新版本。
//!
//! 实现流程：
//! 1. 什么时候查：启动后 30 秒、之后每 6 小时（设置里「自动检查更新」关了就不查）；托盘和「关于」页可以手动查。
//! 2. 查到新版本只提示：托盘那一项变成「更新到 vX」，透明窗口弹一次提示（同一个版本只弹一次），控制面板「关于」页显示版本号和更新说明。
//! 3. 用户点「更新并重启」才下载。下载的包先用 `tauri.conf.json` 里的公钥校验签名，签名里记的版本也要和清单一致
//!    （`requireSignedVersion`，防止拿旧版本的真包冒充新版本），不对就放弃并显示原因；装好后重启。
//! 4. Linux 用 deb 装的没法自己替换：只提示去 Release 页下载（只有 AppImage 能自动更新）。
//! 5. 状态放在 `Updates` 里（一把锁），每次变化推 "update://changed" 给控制面板。

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::api::AppState;
use crate::overlay::Msg;

const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// 发版页（deb 用户手动下载，或者自动更新出错时）。
pub const RELEASES_URL: &str = "https://github.com/lijianqiao/lolGang/releases/latest";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    Failed,
}

#[derive(Default)]
struct Inner {
    phase: Option<Phase>,
    found: Option<Update>,
    error: Option<String>,
    /// 下载进度 0 到 1；不知道总大小时是 None。
    progress: Option<f64>,
    /// 已经弹过提示的版本。
    told: Option<String>,
}

#[derive(Default)]
pub struct Updates {
    inner: Mutex<Inner>,
}

/// 给控制面板的更新状态。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    pub current: String,
    pub phase: Phase,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub progress: Option<f64>,
    pub error: Option<String>,
    /// 能不能在程序里直接更新（Linux 的 deb 不能）。
    pub can_install: bool,
    pub releases_url: &'static str,
}

/// 能不能自己替换：Linux 只有从 AppImage 运行时可以（AppImage 运行时会设 APPIMAGE）。
fn can_install() -> bool {
    !cfg!(target_os = "linux") || std::env::var_os("APPIMAGE").is_some()
}

fn view(app: &AppHandle) -> UpdateView {
    let current = app.package_info().version.to_string();
    let state = app.try_state::<AppState>();
    let inner = state
        .as_ref()
        .map(|s| s.updates.inner.lock().expect("更新锁没有中毒"));
    let (phase, version, notes, progress, error) = match &inner {
        Some(i) => (
            i.phase.unwrap_or(Phase::Idle),
            i.found.as_ref().map(|u| u.version.clone()),
            i.found.as_ref().and_then(|u| u.body.clone()),
            i.progress,
            i.error.clone(),
        ),
        None => (Phase::Idle, None, None, None, None),
    };
    UpdateView {
        current,
        phase,
        version,
        notes,
        progress,
        error,
        can_install: can_install(),
        releases_url: RELEASES_URL,
    }
}

fn set(app: &AppHandle, f: impl FnOnce(&mut Inner)) {
    if let Some(state) = app.try_state::<AppState>() {
        f(&mut state.updates.inner.lock().expect("更新锁没有中毒"));
    }
    let _ = app.emit_to("main", "update://changed", view(app));
}

/// 查一次。`quiet`：自动检查，查到新版本才提示，出错不打扰（错误照样记下，「关于」页能看到）。
pub async fn check(app: &AppHandle, quiet: bool) -> UpdateView {
    let busy = app.try_state::<AppState>().is_some_and(|s| {
        matches!(
            s.updates.inner.lock().expect("更新锁没有中毒").phase,
            Some(Phase::Checking | Phase::Downloading | Phase::Installing)
        )
    });
    if busy {
        return view(app);
    }
    set(app, |i| {
        i.phase = Some(Phase::Checking);
        i.error = None;
    });
    let result = match app.updater() {
        Ok(u) => u.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match result {
        Ok(Some(update)) => {
            let version = update.version.clone();
            let mut first_time = false;
            set(app, |i| {
                first_time = i.told.as_deref() != Some(version.as_str());
                i.told = Some(version.clone());
                i.found = Some(update);
                i.phase = Some(Phase::Available);
            });
            crate::tray::set_update_label(app, Some(&version));
            if first_time && let Some(state) = app.try_state::<AppState>() {
                state.overlays.send(Msg::Toast {
                    title: format!("钢一下有新版本 {version}"),
                    body: "在控制面板的「关于」里更新".into(),
                });
            }
        }
        Ok(None) => {
            set(app, |i| {
                i.found = None;
                i.phase = Some(Phase::UpToDate);
            });
            crate::tray::set_update_label(app, None);
        }
        Err(e) => {
            // 插件对「清单取不到」只给一句英文：说成人话
            let reason = if e.contains("valid release JSON") {
                "GitHub 上还没有发布过新版本，或者连不上 GitHub".to_string()
            } else {
                e
            };
            set(app, |i| {
                i.phase = Some(Phase::Failed);
                i.error = Some(format!("检查更新失败：{reason}"));
            });
            if !quiet {
                tray_show_about(app);
            }
        }
    }
    view(app)
}

/// 启动后 30 秒查一次，之后每 6 小时查一次（设置关了就跳过这一次）。
pub fn start_schedule(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let on = app
                .try_state::<AppState>()
                .is_some_and(|s| s.settings.lock().expect("设置锁没有中毒").auto_update_check);
            if on {
                check(&app, true).await;
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// 打开控制面板的「关于」页。
pub fn tray_show_about(app: &AppHandle) {
    crate::tray::show_main(app);
    let _ = app.emit_to("main", "nav://page", "about");
}

#[tauri::command]
pub fn update_view(app: AppHandle) -> UpdateView {
    view(&app)
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> UpdateView {
    check(&app, false).await
}

/// 下载、校验、安装，然后重启。出错时留在原版本，「关于」页显示原因。
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    if !can_install() {
        return Err("用 deb 装的版本不能自动更新，请到发版页下载新的安装包".into());
    }
    let update = app
        .try_state::<AppState>()
        .and_then(|s| {
            s.updates
                .inner
                .lock()
                .expect("更新锁没有中毒")
                .found
                .clone()
        })
        .ok_or("还没有查到新版本")?;
    set(&app, |i| {
        i.phase = Some(Phase::Downloading);
        i.progress = Some(0.0);
        i.error = None;
    });
    let mut got: u64 = 0;
    let progress_app = app.clone();
    let done_app = app.clone();
    let result = update
        .download_and_install(
            move |chunk, total| {
                got += chunk as u64;
                let p = total
                    .filter(|t| *t > 0)
                    .map(|t| (got as f64 / t as f64).min(1.0));
                set(&progress_app, |i| i.progress = p);
            },
            move || set(&done_app, |i| i.phase = Some(Phase::Installing)),
        )
        .await;
    match result {
        Ok(()) => app.restart(),
        Err(e) => {
            let message = format!("更新没装上：{e}");
            set(&app, |i| {
                i.phase = Some(Phase::Failed);
                i.progress = None;
                i.error = Some(message.clone());
            });
            Err(message)
        }
    }
}
