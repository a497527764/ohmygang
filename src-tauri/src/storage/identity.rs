//! 本机身份表（只有一行）。私钥以密文存放，加解密在 `identity` 模块里做；头像分两种：
//! 内置头像（Preset）走 CBOR 存 avatar 列；200×200 的大头像走分块传输（和队友表情一个通道），
//! avatar_hash 列只记它的哈希、图片文件在数据目录 `avatars/` 里（见 `emotes/store.rs`）。

use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::MemberId;
use crate::error::AppResult;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityRow {
    pub member_id: MemberId,
    pub nickname: String,
    pub secret_cipher: Vec<u8>,
    pub avatar: Option<Vec<u8>>,
    /// 200×200 大头像的哈希（32 位十六进制）；没有大头像时是 None。
    pub avatar_hash: Option<String>,
}

pub fn load(conn: &Connection) -> AppResult<Option<IdentityRow>> {
    Ok(conn
        .query_row(
            "SELECT member_id, nickname, secret_cipher, avatar, avatar_hash FROM identity WHERE id = 1",
            [],
            |r| {
                Ok(IdentityRow {
                    member_id: r.get(0)?,
                    nickname: r.get(1)?,
                    secret_cipher: r.get(2)?,
                    avatar: r.get(3)?,
                    avatar_hash: r.get(4)?,
                })
            },
        )
        .optional()?)
}

pub fn insert(conn: &Connection, row: &IdentityRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO identity (id, member_id, nickname, secret_cipher, avatar, avatar_hash) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
        params![row.member_id, row.nickname, row.secret_cipher, row.avatar, row.avatar_hash],
    )?;
    Ok(())
}

pub fn set_profile(
    conn: &Connection,
    nickname: &str,
    avatar: Option<&[u8]>,
    avatar_hash: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE identity SET nickname = ?1, avatar = ?2, avatar_hash = ?3 WHERE id = 1",
        params![nickname, avatar, avatar_hash],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    #[test]
    fn avatar_hash_round_trips() {
        let c = storage::open_in_memory().unwrap();
        insert(
            &c,
            &IdentityRow {
                member_id: [1; 32],
                nickname: "阿钢".into(),
                secret_cipher: vec![7; 40],
                avatar: None,
                avatar_hash: Some("a".repeat(32)),
            },
        )
        .unwrap();
        assert_eq!(load(&c).unwrap().unwrap().avatar_hash, Some("a".repeat(32)));

        set_profile(&c, "钢", None, None).unwrap();
        assert_eq!(load(&c).unwrap().unwrap().avatar_hash, None);
    }
}
