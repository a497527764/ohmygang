fn main() {
    // Windows：程序要带 Common Controls v6 的清单（windows/app-manifest.xml，抄自 tauri-build 的默认清单）。
    // tauri-build 默认只把它放进程序本身，测试程序没有；用 tauri::test 的单元测试没有它就启动不了
    // （STATUS_ENTRYPOINT_NOT_FOUND）。所以不让 tauri-build 放，改由 MSVC 链接器给所有目标放同一份。
    let msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !msvc {
        tauri_build::build();
        return;
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo 给了 CARGO_MANIFEST_DIR");
    let manifest = std::path::Path::new(&dir)
        .join("windows")
        .join("app-manifest.xml");
    println!("cargo:rerun-if-changed=windows/app-manifest.xml");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes).expect("tauri-build 失败");
}
