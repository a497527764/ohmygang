pub mod app_core;
pub mod base32;
pub mod chat;
pub mod commands;
pub mod crypto;
pub mod error;
pub mod heartsteel;
pub mod identity;
pub mod keystore;
pub mod node;
pub mod protocol;
pub mod rooms;
pub mod settings;
pub mod storage;
pub mod sync;
pub mod transport;

use std::sync::Arc;

use tauri::{Emitter, Manager};

use transport::lan::LAN_PORT;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let node = start_node(app.handle())?;
            app.manage(node);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::identity_get,
            commands::identity_set_nickname,
            commands::rooms_list,
            commands::rooms_create,
            commands::rooms_join,
            commands::rooms_get_code,
            commands::rooms_members,
            commands::rooms_mark_read,
            commands::rooms_set_prefs,
            commands::heartsteel_leaderboard,
            commands::chat_send_text,
            commands::chat_list,
            commands::settings_get,
            commands::settings_update,
            commands::relay_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 启动流程：在应用数据目录打开数据库 → 从凭据管理器取本机主密钥、读出身份 → 启动节点，
/// 节点的通知转发成 Tauri 事件 `hs://event`。
///
/// 单机双开（开发用）：环境变量 `LOLGANG_INSTANCE=2` 时，数据目录、凭据条目、UDP 端口都换一套；
/// 开发版的广播同时发往两个端口，两个实例就能在一台电脑上互相发现。
fn start_node(app: &tauri::AppHandle) -> Result<node::NodeHandle, Box<dyn std::error::Error>> {
    let second = std::env::var("LOLGANG_INSTANCE").is_ok_and(|v| v == "2");
    let mut dir = app.path().app_data_dir()?;
    if second {
        dir.push("instance-2");
    }
    std::fs::create_dir_all(&dir)?;
    let conn = storage::open(&dir.join("lolgang.db"))?;
    let keystore = if second {
        keystore::SystemKeyStore {
            service: "lolgang-2",
        }
    } else {
        keystore::APP_KEYSTORE
    };
    let core = app_core::Core::open(conn, &keystore)?;

    let mut cfg = node::NodeConfig::default();
    if second {
        cfg.lan.bind.set_port(LAN_PORT + 1);
    }
    if cfg!(debug_assertions) {
        cfg.lan.broadcast_ports = vec![LAN_PORT, LAN_PORT + 1];
    }
    let handle = app.clone();
    let sink: node::Sink = Arc::new(move |e| {
        let _ = handle.emit("hs://event", commands::EventDto::from(e));
    });
    Ok(tauri::async_runtime::block_on(node::Node::spawn(
        core, cfg, sink,
    ))?)
}
