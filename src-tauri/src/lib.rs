pub mod base32;
pub mod crypto;
pub mod error;
pub mod heartsteel;
pub mod protocol;
pub mod rooms;
pub mod storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
