//! 本机身份表（只有一行）。私钥以密文存放，加解密在 `identity` 模块里做。

use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::MemberId;
use crate::error::AppResult;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityRow {
    pub member_id: MemberId,
    pub nickname: String,
    pub secret_cipher: Vec<u8>,
}

pub fn load(conn: &Connection) -> AppResult<Option<IdentityRow>> {
    Ok(conn
        .query_row(
            "SELECT member_id, nickname, secret_cipher FROM identity WHERE id = 1",
            [],
            |r| {
                Ok(IdentityRow {
                    member_id: r.get(0)?,
                    nickname: r.get(1)?,
                    secret_cipher: r.get(2)?,
                })
            },
        )
        .optional()?)
}

pub fn insert(conn: &Connection, row: &IdentityRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO identity (id, member_id, nickname, secret_cipher) VALUES (1, ?1, ?2, ?3)",
        params![row.member_id, row.nickname, row.secret_cipher],
    )?;
    Ok(())
}

pub fn set_nickname(conn: &Connection, nickname: &str) -> AppResult<()> {
    conn.execute("UPDATE identity SET nickname = ?1 WHERE id = 1", [nickname])?;
    Ok(())
}
