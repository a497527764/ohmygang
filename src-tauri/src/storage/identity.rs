//! 本机身份表（只有一行）。私钥以密文存放，加解密在 `identity` 模块里做；头像是 CBOR 编码的字节，编解码也在 `identity` 模块。

use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::MemberId;
use crate::error::AppResult;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityRow {
    pub member_id: MemberId,
    pub nickname: String,
    pub secret_cipher: Vec<u8>,
    pub avatar: Option<Vec<u8>>,
}

pub fn load(conn: &Connection) -> AppResult<Option<IdentityRow>> {
    Ok(conn
        .query_row(
            "SELECT member_id, nickname, secret_cipher, avatar FROM identity WHERE id = 1",
            [],
            |r| {
                Ok(IdentityRow {
                    member_id: r.get(0)?,
                    nickname: r.get(1)?,
                    secret_cipher: r.get(2)?,
                    avatar: r.get(3)?,
                })
            },
        )
        .optional()?)
}

pub fn insert(conn: &Connection, row: &IdentityRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO identity (id, member_id, nickname, secret_cipher, avatar) VALUES (1, ?1, ?2, ?3, ?4)",
        params![row.member_id, row.nickname, row.secret_cipher, row.avatar],
    )?;
    Ok(())
}

pub fn set_profile(conn: &Connection, nickname: &str, avatar: Option<&[u8]>) -> AppResult<()> {
    conn.execute(
        "UPDATE identity SET nickname = ?1, avatar = ?2 WHERE id = 1",
        params![nickname, avatar],
    )?;
    Ok(())
}
