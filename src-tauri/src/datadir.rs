//! 本机数据目录。
//!
//! 为什么不用 Tauri 的 `app_data_dir()`：Windows 安装包（Tauri 的 NSIS 模板）在卸载确认页上
//! 有一个「删除应用数据」勾选框，勾上就会 `RmDir /r "$APPDATA\<identifier>"`——identifier 就是
//! `com.lijia.lolgang`，正好是 `app_data_dir()`。一勾，身份私钥、昵称、头像、队伍历史全没了，
//! 下次启动生成一把新私钥，变成「另一个人」：旧身份还留在队伍里，成员列表里就多出一个自己。
//!
//! 所以数据放在产品名目录 `%APPDATA%\OH-MY-GANG!\`：卸载器只认 identifier 目录，碰不到这里；
//! 升级、重装、卸载重装都保留身份。
//!
//! 迁移：老目录还在、新的还没有数据库时，把老目录的 `gang2.db` 和 `avatars/` 搬过来（老用户无感）。

use std::path::{Path, PathBuf};

use tauri::Manager;

/// 数据目录名（产品名，和卸载器删的 identifier 目录不是一个）。
pub const DATA_DIR_NAME: &str = "OH-MY-GANG!";

/// 算出本机数据目录，必要时把老目录的数据搬过来。
/// 拿不到路径就退回 Tauri 的 `app_data_dir()`（老行为），不让程序起不来。
pub fn resolve(app: &tauri::AppHandle) -> PathBuf {
    let Ok(old) = app.path().app_data_dir() else {
        return PathBuf::new();
    };
    let Some(parent) = old.parent() else {
        return old;
    };
    let new = parent.join(DATA_DIR_NAME);
    if new == old {
        return old;
    }
    if let Err(e) = std::fs::create_dir_all(&new) {
        eprintln!("建数据目录失败，改用老目录：{e}");
        return old;
    }
    migrate(&old, &new);
    new
}

/// 老目录有数据库、新目录还没有 → 把老数据搬过来。搬不动就算了（宁可没有也不要报错起不来）。
fn migrate(old: &Path, new: &Path) {
    let old_db = old.join("gang2.db");
    let new_db = new.join("gang2.db");
    if !old_db.exists() || new_db.exists() {
        return;
    }
    // SQLite 的 WAL 模式：主库、-wal、-shm 三个文件要一起搬，不然会丢最近的事务。
    for name in ["gang2.db", "gang2.db-wal", "gang2.db-shm"] {
        let from = old.join(name);
        if from.exists() && std::fs::copy(&from, new.join(name)).is_err() {
            return;
        }
    }
    // 头像、表情缓存等子目录
    for entry in std::fs::read_dir(old).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("instance-") || !entry.path().is_dir() {
            continue;
        }
        let _ = copy_dir(&entry.path(), &new.join(name.as_ref()));
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个临时目录（测试用，跑完删掉）。
    fn temp_dir(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("ohg-datadir-{tag}-{n}"));
        std::fs::create_dir_all(&p).expect("建临时目录");
        p
    }

    #[test]
    fn migrate_moves_the_database_and_the_folders() {
        let root = temp_dir("migrate");
        let old = root.join("com.lijia.lolgang");
        let new = root.join(DATA_DIR_NAME);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("gang2.db"), b"db").unwrap();
        std::fs::write(old.join("gang2.db-wal"), b"wal").unwrap();
        std::fs::create_dir_all(old.join("avatars")).unwrap();
        std::fs::write(old.join("avatars").join("a.png"), b"img").unwrap();
        // 多开实例的目录不算本机数据，不搬
        std::fs::create_dir_all(old.join("instance-2")).unwrap();
        std::fs::write(old.join("instance-2").join("x"), b"x").unwrap();

        migrate(&old, &new);

        assert!(new.join("gang2.db").exists(), "数据库要搬过来");
        assert!(new.join("gang2.db-wal").exists(), "WAL 也要一起搬");
        assert!(new.join("avatars").join("a.png").exists(), "头像目录要搬");
        assert!(!new.join("instance-2").exists(), "实例目录不搬");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn migrate_never_overwrites_an_existing_database() {
        let root = temp_dir("keep");
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("gang2.db"), b"old").unwrap();
        std::fs::write(new.join("gang2.db"), b"new").unwrap();

        migrate(&old, &new);

        assert_eq!(std::fs::read(new.join("gang2.db")).unwrap(), b"new");
        std::fs::remove_dir_all(&root).ok();
    }
}
