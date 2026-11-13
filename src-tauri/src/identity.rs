//! 本机身份（需求 3.1）。
//!
//! 实现流程：
//! 1. `bootstrap`：库里没有身份就生成一把 Ed25519 私钥，用本机主密钥加密后存进 identity 表；有就解密读出。
//!    解密失败（比如主密钥被换了）直接报错，绝不悄悄生成新身份覆盖旧的，否则会变成另一个人、进不去原来的队伍。
//! 2. 昵称和头像是全局的（各房间相同）：昵称上限 32 个字符，头像必须合格（`Avatar::is_valid`）；
//!    内置头像以 CBOR 字节存进 identity 表，读出时解不开就当作没有。生成各房间 Profile 事件由 `Core` 负责。
//! 3. 200×200 的大头像走分块传输（和队友表情一个通道）：identity 表只存哈希（avatar_hash 列），
//!    图片文件在数据目录 `avatars/`（导入和读取见 `emotes/store.rs`）。

use crate::crypto::{self, Identity, MemberId};
use crate::error::{AppError, AppResult};
use crate::keystore::{PURPOSE_IDENTITY, decrypt_local, encrypt_local};
use crate::protocol::body::{self, Avatar, NICKNAME_MAX_CHARS};
use crate::storage::{Connection, identity as id_store};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityView {
    pub member_id: MemberId,
    pub nickname: String,
    pub fingerprint: String,
    pub avatar: Option<Avatar>,
    /// 200×200 大头像的哈希（文件在数据目录 `avatars/`）；没有大头像时是 None。
    pub avatar_hash: Option<String>,
}

pub fn bootstrap(conn: &Connection, master: &[u8; 32]) -> AppResult<(Identity, IdentityView)> {
    let (me, nickname, avatar, avatar_hash) = match id_store::load(conn)? {
        Some(row) => {
            let secret: [u8; 32] = decrypt_local(master, PURPOSE_IDENTITY, &row.secret_cipher)?
                .try_into()
                .map_err(|_| AppError::Decrypt)?;
            let me = Identity::from_secret(&secret);
            if me.member_id() != row.member_id {
                return Err(AppError::Decrypt);
            }
            let avatar = row.avatar.and_then(|b| body::decode::<Avatar>(&b).ok());
            (me, row.nickname, avatar, row.avatar_hash)
        }
        None => {
            let me = Identity::generate()?;
            let secret_cipher = encrypt_local(master, PURPOSE_IDENTITY, &me.secret())?;
            id_store::insert(
                conn,
                &id_store::IdentityRow {
                    member_id: me.member_id(),
                    nickname: String::new(),
                    secret_cipher,
                    avatar: None,
                    avatar_hash: None,
                },
            )?;
            (me, String::new(), None, None)
        }
    };
    let view = IdentityView {
        member_id: me.member_id(),
        fingerprint: crypto::fingerprint(&me.member_id()),
        nickname,
        avatar,
        avatar_hash,
    };
    Ok((me, view))
}

/// `avatar`：内置头像（Preset）；`avatar_hash`：大头像的哈希（None 表示不用大图）。
pub fn set_profile(
    conn: &Connection,
    nickname: &str,
    avatar: Option<&Avatar>,
    avatar_hash: Option<&str>,
) -> AppResult<()> {
    if nickname.chars().count() > NICKNAME_MAX_CHARS {
        return Err(AppError::NicknameTooLong);
    }
    if avatar.is_some_and(|a| !a.is_valid()) {
        return Err(AppError::InvalidAvatar);
    }
    if avatar_hash.is_some_and(|h| !crate::emotes::is_hash(h)) {
        return Err(AppError::InvalidAvatar);
    }
    id_store::set_profile(
        conn,
        nickname,
        avatar.map(body::encode).as_deref(),
        avatar_hash,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{self, identity as id_store};

    const KEY: [u8; 32] = [9u8; 32];

    #[test]
    fn bootstrap_creates_once_then_reloads() {
        let c = storage::open_in_memory().unwrap();
        let (me1, v1) = bootstrap(&c, &KEY).unwrap();
        let (me2, v2) = bootstrap(&c, &KEY).unwrap();
        assert_eq!(me1.member_id(), me2.member_id());
        assert_eq!(v1, v2);
        assert_eq!(v1.member_id, me1.member_id());
        assert_eq!(v1.fingerprint.len(), 13);
        assert_eq!(v1.nickname, "");
    }

    #[test]
    fn secret_is_stored_encrypted() {
        let c = storage::open_in_memory().unwrap();
        let (me, _) = bootstrap(&c, &KEY).unwrap();
        let row = id_store::load(&c).unwrap().unwrap();
        let secret = me.secret();
        assert!(!row.secret_cipher.windows(32).any(|w| w == secret));
    }

    #[test]
    fn wrong_master_key_fails_instead_of_replacing_identity() {
        let c = storage::open_in_memory().unwrap();
        let (me, _) = bootstrap(&c, &KEY).unwrap();
        assert!(matches!(bootstrap(&c, &[1u8; 32]), Err(AppError::Decrypt)));
        assert_eq!(
            id_store::load(&c).unwrap().unwrap().member_id,
            me.member_id()
        );
    }

    #[test]
    fn nickname_length_is_checked_and_saved() {
        let c = storage::open_in_memory().unwrap();
        bootstrap(&c, &KEY).unwrap();
        assert!(matches!(
            set_profile(&c, &"长".repeat(33), None, None),
            Err(AppError::NicknameTooLong)
        ));
        set_profile(&c, &"长".repeat(32), None, None).unwrap();
        assert_eq!(bootstrap(&c, &KEY).unwrap().1.nickname, "长".repeat(32));
    }

    #[test]
    fn avatar_is_checked_and_saved() {
        let c = storage::open_in_memory().unwrap();
        bootstrap(&c, &KEY).unwrap();
        let bad = Avatar::Image(b"GIF89a".to_vec());
        assert!(matches!(
            set_profile(&c, "阿钢", Some(&bad), None),
            Err(AppError::InvalidAvatar)
        ));
        let preset = Avatar::Preset { icon: 5, color: 1 };
        set_profile(&c, "阿钢", Some(&preset), None).unwrap();
        assert_eq!(bootstrap(&c, &KEY).unwrap().1.avatar, Some(preset));
        set_profile(&c, "阿钢", None, None).unwrap();
        assert_eq!(bootstrap(&c, &KEY).unwrap().1.avatar, None);
    }

    #[test]
    fn avatar_hash_is_checked_and_saved() {
        let c = storage::open_in_memory().unwrap();
        bootstrap(&c, &KEY).unwrap();
        // 形状不对的哈希不要
        assert!(matches!(
            set_profile(&c, "阿钢", None, Some("xyz")),
            Err(AppError::InvalidAvatar)
        ));
        let h = "a".repeat(32);
        set_profile(&c, "阿钢", None, Some(&h)).unwrap();
        assert_eq!(bootstrap(&c, &KEY).unwrap().1.avatar_hash, Some(h));
        set_profile(&c, "阿钢", None, None).unwrap();
        assert_eq!(bootstrap(&c, &KEY).unwrap().1.avatar_hash, None);
    }
}
