//! 本机身份表（只有一行）。私钥以密文存放，加解密在 `identity` 模块里做；头像分两种：
//! 内置头像（Preset）走 CBOR 存 avatar 列；200×200 的大头像走分块传输（和队友表情一个通道），
//! avatar_hash 列只记它的哈希、图片文件在数据目录 `avatars/` 里（见 `emotes/store.rs`）。

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::crypto::MemberId;
use crate::error::AppResult;

/// 身份备份文件（数据目录里）：身份表被清空时用它恢复，避免变成一个全新的人。
const BACKUP_FILE: &str = "identity.bak";

/// 备份文件里的内容。私钥还是密文（要用主密钥才能解开），所以备份不泄密。
#[derive(Serialize, Deserialize)]
struct Backup {
    member_id: String,
    nickname: String,
    secret_cipher: String,
    avatar: Option<String>,
    avatar_hash: Option<String>,
}

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

/// 把身份写进备份文件（数据目录里）。身份表被清空时靠它恢复。
pub fn write_backup(dir: &std::path::Path, row: &IdentityRow) -> AppResult<()> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let backup = Backup {
        member_id: crate::crypto::hex(&row.member_id),
        nickname: row.nickname.clone(),
        secret_cipher: b64.encode(&row.secret_cipher),
        avatar: row.avatar.as_ref().map(|a| b64.encode(a)),
        avatar_hash: row.avatar_hash.clone(),
    };
    let text = serde_json::to_string(&backup)
        .map_err(|e| crate::error::AppError::KeyStore(e.to_string()))?;
    let path = dir.join(BACKUP_FILE);
    let tmp = dir.join("identity.bak.tmp");
    // 先写临时文件再改名：中途断电不会留下半个备份。
    std::fs::write(&tmp, text).map_err(|e| crate::error::AppError::KeyStore(e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| crate::error::AppError::KeyStore(e.to_string()))?;
    Ok(())
}

/// 身份表空着（数据目录被清过）→ 从备份恢复。恢复不了就返回 false，调用方按新身份处理。
pub fn restore_from_backup(conn: &Connection, dir: &std::path::Path) -> bool {
    use base64::Engine;
    if load(conn).ok().flatten().is_some() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(dir.join(BACKUP_FILE)) else {
        return false;
    };
    let Ok(b) = serde_json::from_str::<Backup>(&text) else {
        return false;
    };
    let b64 = base64::engine::general_purpose::STANDARD;
    let Some(member_id) = crate::crypto::parse_hex(&b.member_id) else {
        return false;
    };
    let Ok(member_id) = <[u8; 32]>::try_from(member_id.as_slice()) else {
        return false;
    };
    let Ok(secret_cipher) = b64.decode(b.secret_cipher.as_bytes()) else {
        return false;
    };
    let avatar = b
        .avatar
        .as_ref()
        .and_then(|a| b64.decode(a.as_bytes()).ok());
    let row = IdentityRow {
        member_id,
        nickname: b.nickname,
        secret_cipher,
        avatar,
        avatar_hash: b.avatar_hash,
    };
    insert(conn, &row).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    /// 身份表被清空后能从备份恢复，不会变成另一个人。
    #[test]
    fn backup_restores_identity_after_the_table_is_wiped() {
        let dir = std::env::temp_dir().join(format!("lolgang-id-bak-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = storage::open_in_memory().unwrap();
        let row = IdentityRow {
            member_id: [3; 32],
            nickname: "鹏哥".into(),
            secret_cipher: vec![9; 40],
            avatar: None,
            avatar_hash: Some("b".repeat(32)),
        };
        insert(&c, &row).unwrap();
        write_backup(&dir, &row).unwrap();

        // 模拟数据目录被清空：换个空库
        let fresh = storage::open_in_memory().unwrap();
        assert!(restore_from_backup(&fresh, &dir));
        let back = load(&fresh).unwrap().unwrap();
        assert_eq!(back.member_id, row.member_id);
        assert_eq!(back.nickname, "鹏哥");
        assert_eq!(back.avatar_hash, row.avatar_hash);

        // 已经有身份时不动它
        let again = storage::open_in_memory().unwrap();
        insert(&again, &row).unwrap();
        assert!(!restore_from_backup(&again, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
