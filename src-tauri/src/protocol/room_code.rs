//! 房间码（架构第 3 节）。
//!
//! 实现流程：
//! 1. 组成：room_secret（16 字节随机数）‖ owner_tag（房主公钥 SHA-256 的前 10 字节）= payload 26 字节；
//!    check = SHA-256("hs/code/v1" ‖ payload) 的前 10 位。
//! 2. 编码：payload ‖ check 共 218 位，用 Crockford Base32 编成 44 个字符，前面加 "HS1"。
//!    展示时每 4 个字符一组，用连字符隔开：HS1-XXXX-XXXX-…（不算连字符共 48 个字符）。
//! 3. 解析：去掉连字符和空白、转大写，检查 "HS1" 前缀和长度，解码后重算校验位。
//!    校验位的作用：抄错时立刻报错，而不是派生出另一把「有效」密钥、进入一个永远没人的空房间。
//!    10 位校验能拦下约 99.9% 的单字符错误（每种错误约有 1/1024 的概率漏过）。
//! 4. 派生：`keys()` 交给 crypto::derive_room_keys 得到 room_key 和 room_id。

use crate::crypto::{self, MemberId, RoomId, RoomKey};

const PREFIX: &str = "HS1";
const PAYLOAD_LEN: usize = 26;
const NBITS: usize = PAYLOAD_LEN * 8 + 10;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RoomCodeError {
    #[error("房间码应以 HS1 开头")]
    BadPrefix,
    #[error("房间码长度或字符不对")]
    BadEncoding,
    #[error("房间码校验位不对，请检查是否抄错")]
    BadCheck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomCode {
    pub room_secret: [u8; 16],
    pub owner_tag: [u8; 10],
}

/// owner_tag = SHA-256(房主公钥) 的前 10 字节，也是房主信任根（架构第 3 节）。
pub fn owner_tag_of(member_id: &MemberId) -> [u8; 10] {
    let h = crypto::sha256(&[member_id]);
    let mut tag = [0u8; 10];
    tag.copy_from_slice(&h[..10]);
    tag
}

fn check_bits(payload: &[u8; PAYLOAD_LEN]) -> u16 {
    let h = crypto::sha256(&[b"hs/code/v1", payload]);
    (u16::from(h[0]) << 2) | u16::from(h[1] >> 6)
}

impl RoomCode {
    pub fn format(&self) -> String {
        let mut payload = [0u8; PAYLOAD_LEN];
        payload[..16].copy_from_slice(&self.room_secret);
        payload[16..].copy_from_slice(&self.owner_tag);
        let check = check_bits(&payload);
        let mut data = payload.to_vec();
        data.push((check >> 2) as u8);
        data.push(((check & 0b11) << 6) as u8);
        let body = crate::base32::encode(&data, NBITS);
        let groups: Vec<&str> = body
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).expect("Base32 输出是 ASCII"))
            .collect();
        format!("{PREFIX}-{}", groups.join("-"))
    }

    pub fn parse(s: &str) -> Result<Self, RoomCodeError> {
        let cleaned: String = s
            .chars()
            .filter(|c| *c != '-' && !c.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase();
        let rest = cleaned
            .strip_prefix(PREFIX)
            .ok_or(RoomCodeError::BadPrefix)?;
        let data = crate::base32::decode(rest, NBITS).ok_or(RoomCodeError::BadEncoding)?;
        let mut payload = [0u8; PAYLOAD_LEN];
        payload.copy_from_slice(&data[..PAYLOAD_LEN]);
        let check = (u16::from(data[PAYLOAD_LEN]) << 2) | u16::from(data[PAYLOAD_LEN + 1] >> 6);
        if check != check_bits(&payload) {
            return Err(RoomCodeError::BadCheck);
        }
        let mut code = RoomCode {
            room_secret: [0; 16],
            owner_tag: [0; 10],
        };
        code.room_secret.copy_from_slice(&payload[..16]);
        code.owner_tag.copy_from_slice(&payload[16..]);
        Ok(code)
    }

    pub fn keys(&self) -> (RoomKey, RoomId) {
        crypto::derive_room_keys(&self.room_secret, &self.owner_tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RoomCode {
        RoomCode {
            room_secret: [0x11; 16],
            owner_tag: [0x22; 10],
        }
    }

    #[test]
    fn format_shape() {
        let s = sample().format();
        assert!(s.starts_with("HS1-"));
        assert_eq!(s.chars().filter(|c| *c != '-').count(), 3 + 44); // "HS1" + 44 个 Base32 字符
        assert_eq!(s, GOLDEN_CODE);
    }

    #[test]
    fn roundtrip_ignores_case_hyphens_spaces() {
        let s = sample().format();
        assert_eq!(RoomCode::parse(&s), Ok(sample()));
        let messy = format!("  {} ", s.to_lowercase().replace('-', " "));
        assert_eq!(RoomCode::parse(&messy), Ok(sample()));
    }

    #[test]
    fn rejects_prefix_length_and_check() {
        let s = sample().format();
        assert_eq!(
            RoomCode::parse(&s.replacen("HS1", "HS2", 1)),
            Err(RoomCodeError::BadPrefix)
        );
        assert_eq!(
            RoomCode::parse(&s[..s.len() - 1]),
            Err(RoomCodeError::BadEncoding)
        );
        // 把第 10 个字符换成别的字符
        let mut chars: Vec<char> = s.chars().collect();
        chars[10] = if chars[10] == 'A' { 'B' } else { 'A' };
        let typo: String = chars.into_iter().collect();
        assert_eq!(RoomCode::parse(&typo), Err(RoomCodeError::BadCheck));
    }

    #[test]
    fn single_char_typos_almost_always_rejected() {
        let s = sample().format();
        let alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
        let (mut tried, mut accepted) = (0, 0);
        for (i, orig) in s.char_indices().skip(4).filter(|(_, c)| *c != '-') {
            for c in alphabet.chars().filter(|c| *c != orig) {
                let mut t = s.clone();
                t.replace_range(i..i + 1, &c.to_string());
                tried += 1;
                if RoomCode::parse(&t).is_ok() {
                    accepted += 1;
                }
            }
        }
        // 1364 种单字符替换，预期漏过约 1.3 个
        assert_eq!(tried, 44 * 31);
        assert!(accepted <= 5, "漏过 {accepted} 个");
    }

    #[test]
    fn owner_tag_changes_room() {
        let a = sample();
        let b = RoomCode {
            owner_tag: [0x23; 10],
            ..a
        };
        assert_ne!(a.keys().1, b.keys().1);
    }

    const GOLDEN_CODE: &str = "HS1-248H-248H-248H-248H-248H-248H-24H2-48H2-48H2-48H2-4A5C";
}
