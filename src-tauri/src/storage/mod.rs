//! SQLite 存储（架构第 9 节）。
//!
//! 实现流程：
//! 1. `open`：打开数据库文件，开启 WAL 模式（写入时不阻塞读取，意外断电也不容易损坏），再升级表结构。
//! 2. 升级：`PRAGMA user_version` 记录当前表结构版本；低于代码里的最新版本时，在一个事务里依次执行缺少的升级。
//! 3. 各类数据的读写放在子模块里。函数都接收 `&Connection`，事务（`Transaction`）也能当 `&Connection` 用，
//!    所以由调用方决定事务边界：一次业务操作的多次写入要么全部生效，要么全部不生效。

pub mod events;
pub mod identity;
pub mod rooms;
mod schema;
pub mod settings;
pub mod sync_state;

use std::path::Path;

pub use rusqlite::Connection;

use crate::error::AppResult;

pub fn open(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> AppResult<Connection> {
    let conn = Connection::open_in_memory()?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> AppResult<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in schema::MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as u32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
