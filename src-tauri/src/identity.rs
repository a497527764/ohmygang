//! 本机身份（需求 3.1）。
//!
//! 实现流程：
//! 1. `bootstrap`：库里没有身份就生成一把 Ed25519 私钥，用本机主密钥加密后存进 identity 表；有就解密读出。
//!    解密失败（比如主密钥被换了）直接报错，绝不悄悄生成新身份覆盖旧的，否则会变成另一个人、进不去原来的队伍。
//! 2. 昵称和头像是全局的（各房间相同）：昵称上限 32 个字符，头像必须合格（`Avatar::is_valid`）；
//!    头像以 CBOR 字节存进 identity 表，读出时解不开就当作没有。生成各房间 Profile 事件由 `Core` 负责。

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
}

pub fn bootstrap(conn: &Connection, master: &[u8; 32]) -> AppResult<(Identity, IdentityView)> {
    let (me, nickname, avatar) = match id_store::load(conn)? {
        Some(row) => {
            let secret: [u8; 32] = decrypt_local(master, PURPOSE_IDENTITY, &row.secret_cipher)?
                .try_into()
                .map_err(|_| AppError::Decrypt)?;
            let me = Identity::from_secret(&secret);
            if me.member_id() != row.member_id {
                return Err(AppError::Decrypt);
            }
            let avatar = row.avatar.and_then(|b| body::decode::<Avatar>(&b).ok());
            (me, row.nickname, avatar)
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
                },
            )?;
            (me, String::new(), None)
        }
    };
    let view = IdentityView {
        member_id: me.member_id(),
        fingerprint: crypto::fingerprint(&me.member_id()),
        nickname,
        avatar,
    };
    Ok((me, view))
}

pub fn set_profile(conn: &Connection, nickname: &str, avatar: Option<&Avatar>) -> AppResult<()> {
    if nickname.chars().count() > NICKNAME_MAX_CHARS {
        return Err(AppError::NicknameTooLong);
    }
    if avatar.is_some_and(|a| !a.is_valid()) {
        return Err(AppError::InvalidAvatar);
    }
    id_store::set_profile(conn, nickname, avatar.map(body::encode).as_deref())
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
            set_profile(&c, &"长".repeat(33), None),
            Err(AppError::NicknameTooLong)
        ));
        set_profile(&c, &"长".repeat(32), None).unwrap();
        assert_eq!(bootstrap(&c, &KEY).unwrap().1.nickname, "长".repeat(32));
    }

    #[test]
    fn avatar_is_checked_and_saved() {
        let c = storage::open_in_memory().unwrap();
        bootstrap(&c, &KEY).unwrap();
        let bad = Avatar::Image(b"GIF89a".to_vec());
        assert!(matches!(
            set_profile(&c, "阿钢", Some(&bad)),
            Err(AppError::InvalidAvatar)
        ));
        let preset = Avatar::Preset { icon: 5, color: 1 };
        set_profile(&c, "阿钢", Some(&preset)).unwrap();
        assert_eq!(bootstrap(&c, &KEY).unwrap().1.avatar, Some(preset));
        set_profile(&c, "阿钢", None).unwrap();
        assert_eq!(bootstrap(&c, &KEY).unwrap().1.avatar, None);
    }
}
