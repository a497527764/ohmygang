//! 身份与房间备份（需求 3.1、架构 8.3）：口令经 Argon2id 派生密钥，再用 XChaCha20-Poly1305 加密。
//!
//! 实现流程：
//! 1. 文件 = `HSBK` ‖ 版本 1 ‖ Argon2id 参数（内存 KiB、轮数，各 4 字节小端；并行度 1 字节）‖ 盐 16 ‖ nonce 24 ‖ 密文。
//!    整个文件头作为附加认证数据，改了参数或盐也解不开。
//! 2. 明文是 CBOR：种类、私钥、昵称、头像（V1.1 新增，旧备份没有）、房间列表（房间码，加上可选的全部事件；
//!    事件用线上格式，room_id 由房间码算出）。
//! 3. 口令至少 6 个字符。口令错、文件坏、不是备份文件统一报同一个错，不给猜口令的人多余信息；
//!    只有「版本比本程序新」单独报，提示升级。
//! 4. 参数写在文件里，以后调高强度旧文件照样能读；读文件时限制参数上限，防止恶意文件把内存吃光。
//!    测试里用很小的参数，免得太慢。

use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};

use crate::crypto;
use crate::error::{AppError, AppResult};
use crate::protocol::body::Avatar;
use crate::protocol::event::SignedEvent;
use crate::protocol::frame::WireEvent;
use crate::protocol::room_code::RoomCode;

const MAGIC: &[u8; 4] = b"HSBK";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 4 + 1 + 4 + 4 + 1 + 16 + 24;
const MIN_PASSPHRASE_CHARS: usize = 6;

/// (内存 KiB, 轮数, 并行度)。
#[cfg(not(test))]
const KDF: (u32, u32, u8) = (64 * 1024, 3, 1);
#[cfg(test)]
const KDF: (u32, u32, u8) = (64, 1, 1);
const MAX_KDF: (u32, u32, u8) = (1024 * 1024, 10, 4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupKind {
    /// 只有身份（私钥和昵称）。
    IdentityOnly,
    /// 身份加上全部房间的房间码和事件（聊天记录、层数都在里面）。
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupRoom {
    pub code: String,
    pub events: Vec<SignedEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupData {
    pub kind: BackupKind,
    pub secret: [u8; 32],
    pub nickname: String,
    pub avatar: Option<Avatar>,
    pub rooms: Vec<BackupRoom>,
}

#[derive(Serialize, Deserialize)]
struct Body {
    #[serde(rename = "k")]
    kind: u8,
    #[serde(rename = "s", with = "serde_bytes")]
    secret: [u8; 32],
    #[serde(rename = "n")]
    nickname: String,
    #[serde(rename = "a", default, skip_serializing_if = "Option::is_none")]
    avatar: Option<Avatar>,
    #[serde(rename = "r")]
    rooms: Vec<RoomBody>,
}

#[derive(Serialize, Deserialize)]
struct RoomBody {
    #[serde(rename = "c")]
    code: String,
    #[serde(rename = "e")]
    events: Vec<WireEvent>,
}

fn derive_key(passphrase: &str, salt: &[u8; 16], (m, t, p): (u32, u32, u8)) -> AppResult<[u8; 32]> {
    let params = Params::new(m, t, u32::from(p), Some(32)).map_err(|_| AppError::BackupInvalid)?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|_| AppError::BackupInvalid)?;
    Ok(key)
}

pub fn encode(data: &BackupData, passphrase: &str) -> AppResult<Vec<u8>> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(AppError::BackupPassphraseTooShort);
    }
    let body = Body {
        kind: match data.kind {
            BackupKind::IdentityOnly => 1,
            BackupKind::Full => 2,
        },
        secret: data.secret,
        nickname: data.nickname.clone(),
        avatar: data.avatar.clone(),
        rooms: data
            .rooms
            .iter()
            .map(|r| RoomBody {
                code: r.code.clone(),
                events: r.events.iter().map(WireEvent::from_signed).collect(),
            })
            .collect(),
    };
    let mut plain = Vec::new();
    ciborium::into_writer(&body, &mut plain).expect("写入内存不会失败");

    let salt = crypto::random_bytes::<16>()?;
    let nonce = crypto::random_bytes::<24>()?;
    let (m, t, p) = KDF;
    let mut out = Vec::with_capacity(HEADER_LEN + plain.len() + 16);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&m.to_le_bytes());
    out.extend_from_slice(&t.to_le_bytes());
    out.push(p);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    let key = derive_key(passphrase, &salt, KDF)?;
    let sealed = crypto::seal(&key, &nonce, &out[..HEADER_LEN], &plain);
    out.extend(sealed);
    Ok(out)
}

pub fn decode(bytes: &[u8], passphrase: &str) -> AppResult<BackupData> {
    if bytes.len() < HEADER_LEN + 16 || &bytes[..4] != MAGIC {
        return Err(AppError::BackupInvalid);
    }
    if bytes[4] != VERSION {
        return Err(AppError::BackupUnsupported);
    }
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("长度已检查"));
    let kdf = (u32_at(5), u32_at(9), bytes[13]);
    if kdf.0 > MAX_KDF.0 || kdf.1 > MAX_KDF.1 || kdf.2 > MAX_KDF.2 {
        return Err(AppError::BackupInvalid);
    }
    let salt: [u8; 16] = bytes[14..30].try_into().expect("长度已检查");
    let nonce: [u8; 24] = bytes[30..HEADER_LEN].try_into().expect("长度已检查");
    let key = derive_key(passphrase, &salt, kdf)?;
    let plain = crypto::open(&key, &nonce, &bytes[..HEADER_LEN], &bytes[HEADER_LEN..])
        .map_err(|_| AppError::BackupInvalid)?;
    let body: Body =
        ciborium::from_reader(plain.as_slice()).map_err(|_| AppError::BackupInvalid)?;
    let kind = match body.kind {
        1 => BackupKind::IdentityOnly,
        2 => BackupKind::Full,
        _ => return Err(AppError::BackupInvalid),
    };
    let mut rooms = Vec::with_capacity(body.rooms.len());
    for r in body.rooms {
        let room_id = RoomCode::parse(&r.code)
            .map_err(|_| AppError::BackupInvalid)?
            .keys()
            .1;
        let events = r
            .events
            .into_iter()
            .map(|w| w.into_signed(room_id).ok_or(AppError::BackupInvalid))
            .collect::<AppResult<Vec<_>>>()?;
        rooms.push(BackupRoom {
            code: r.code,
            events,
        });
    }
    Ok(BackupData {
        kind,
        secret: body.secret,
        nickname: body.nickname,
        avatar: body.avatar.filter(Avatar::is_valid),
        rooms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BackupData {
        BackupData {
            kind: BackupKind::IdentityOnly,
            secret: [7u8; 32],
            nickname: "阿钢".into(),
            avatar: Some(Avatar::Preset { icon: 2, color: 3 }),
            rooms: Vec::new(),
        }
    }

    #[test]
    fn roundtrip() {
        let file = encode(&sample(), "口令123456").unwrap();
        assert_eq!(&file[..4], b"HSBK");
        assert!(
            !file.windows(6).any(|w| w == "阿钢".as_bytes()),
            "昵称不以明文出现"
        );
        assert_eq!(decode(&file, "口令123456").unwrap(), sample());
    }

    #[test]
    fn wrong_passphrase_or_tampering_fails() {
        let file = encode(&sample(), "口令123456").unwrap();
        assert!(matches!(
            decode(&file, "口令123457"),
            Err(AppError::BackupInvalid)
        ));
        for i in [5, 20, file.len() - 1] {
            let mut bad = file.clone();
            bad[i] ^= 1;
            assert!(
                matches!(decode(&bad, "口令123456"), Err(AppError::BackupInvalid)),
                "改第 {i} 字节"
            );
        }
        assert!(matches!(
            decode(b"not a backup", "口令123456"),
            Err(AppError::BackupInvalid)
        ));
    }

    #[test]
    fn newer_version_is_reported() {
        let mut file = encode(&sample(), "口令123456").unwrap();
        file[4] = 9;
        assert!(matches!(
            decode(&file, "口令123456"),
            Err(AppError::BackupUnsupported)
        ));
    }

    #[test]
    fn short_passphrase_is_rejected() {
        assert!(matches!(
            encode(&sample(), "12345"),
            Err(AppError::BackupPassphraseTooShort)
        ));
    }
}
