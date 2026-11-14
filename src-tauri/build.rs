/// 所有 Rust 命令（和 lib.rs 的 `generate_handler!` 一一对应）。列在这里，每个命令就有自己的权限 `allow-命令名`
/// （下划线换成连字符），哪个窗口能调哪个命令由 capabilities/ 里的文件给；没给的窗口调了会被拒（3.2.0）。
const COMMANDS: &[&str] = &[
    "settings_read",
    "settings_write",
    "status_get",
    "pings_set_enabled",
    "input_suspend",
    "ping_preview",
    "overlay_hello",
    "overlay_idle",
    "emote_import",
    "emote_remove",
    "emote_preview",
    "open_accessibility_settings",
    "app_relaunch",
    "voice_view",
    "voice_test",
    "history_list",
    "history_play",
    "history_stop",
    "text_preview",
    "vote_view",
    "vote_start",
    "vote_answer",
    "text_send",
    "text_cancel",
    "update_view",
    "update_check",
    "update_install",
    "team_view",
    "team_create",
    "team_join",
    "team_cancel",
    "team_leave",
    "team_dissolve",
    "team_rename",
    "profile_set",
    "ui_sync_window",
    "team_recent",
    "team_forget",
    "team_moderate",
];

/// lib.rs 里 `generate_handler![...]` 注册的全部命令名。
///
/// 为什么要单独解析一遍：Tauri 2 的权限是「白名单」——命令没列进 COMMANDS，
/// tauri-build 就不会给它生成 `allow-xxx` 权限；capabilities 里也就没法放行它。
/// 前端一调就是一句 `Command xxx not allowed by ACL`，功能在界面上直接罢工
/// （3.2.9 实测踩过：队伍历史、删历史记录、踢人拉黑、窗口缩放四个命令全被拦）。
/// 所以在这里当构建期看门狗：漏一个就直接编译失败，别等发版后用户才发现。
fn handler_commands() -> Vec<String> {
    let src = include_str!("src/lib.rs");
    let marker = "generate_handler![";
    let start = src
        .find(marker)
        .expect("src/lib.rs 里找不到 generate_handler![]");
    let rest = &src[start + marker.len()..];
    let end = rest
        .find(']')
        .expect("generate_handler![] 里的方括号没闭合");
    rest[..end]
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        // 写的是 `team::commands::team_recent`，命令名是最后一段
        .map(|s| s.rsplit("::").next().unwrap_or(s).to_string())
        .collect()
}

fn main() {
    // 看门狗：lib.rs 注册的命令必须都在 COMMANDS 清单里，否则权限发不出来。
    let missing: Vec<String> = handler_commands()
        .into_iter()
        .filter(|c| !COMMANDS.iter().any(|x| *x == c))
        .collect();
    assert!(
        missing.is_empty(),
        "这些命令在 src/lib.rs 里注册了，但 build.rs 的 COMMANDS 里没列，\
         它们的 allow-* 权限不会被生成，前端调用会报 \"not allowed by ACL\"：{missing:?}\n\
         请把缺的名字补进 build.rs 的 COMMANDS。"
    );

    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    // Windows：程序要带 Common Controls v6 的清单（windows/app-manifest.xml，抄自 tauri-build 的默认清单）。
    // tauri-build 默认只把它放进程序本身，测试程序没有；用 tauri::test 的单元测试没有它就启动不了
    // （STATUS_ENTRYPOINT_NOT_FOUND）。所以不让 tauri-build 放，改由 MSVC 链接器给所有目标放同一份。
    let msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !msvc {
        tauri_build::try_build(attributes).expect("tauri-build 失败");
        return;
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo 给了 CARGO_MANIFEST_DIR");
    let manifest = std::path::Path::new(&dir)
        .join("windows")
        .join("app-manifest.xml");
    println!("cargo:rerun-if-changed=windows/app-manifest.xml");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    let attributes =
        attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes).expect("tauri-build 失败");
}
