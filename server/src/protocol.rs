//! 队伍码与房间密钥派生：和客户端 `crypto.rs` / `protocol/room_code.rs` 逐字节一致，
//! 这样服务端生成的 GT2 长码能被客户端 `RoomCode::parse` 解析，并派生出同一个 room_id。
//!
//! 服务端分配 6 位短码，但短码只是「别名」；真正决定房间密钥的是它背后的 room_secret + owner_tag，
//! 二者被编码进 GT2 长码交给客户端。客户端用 GT2 长码像往常一样派生 room_key / room_id。

use hkdf::Hkdf;
use sha2::{Digest, Sha256};

pub type RoomKey = [u8; 32];
pub type RoomId = [u8; 16];

const PREFIX: &str = "GT2";
const PAYLOAD_LEN: usize = 26;
const NBITS: usize = PAYLOAD_LEN * 8 + 10;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// owner_tag = SHA-256(成员公钥) 前 10 字节，也是房主信任根（和客户端一致）。
pub fn owner_tag_of(member_id: &[u8; 32]) -> [u8; 10] {
    let h = sha256(&[member_id]);
    let mut t = [0u8; 10];
    t.copy_from_slice(&h[..10]);
    t
}

/// 用 HKDF 从 room_secret 和 owner_tag 派生 room_key 和 room_id（和客户端一致）。
pub fn derive_room_keys(secret: &[u8; 16], owner_tag: &[u8; 10]) -> (RoomKey, RoomId) {
    let hk = Hkdf::<Sha256>::new(Some(owner_tag), secret);
    let mut key = [0u8; 32];
    let mut id = [0u8; 16];
    hk.expand(b"hs/room-key/v1", &mut key)
        .expect("长度合法");
    hk.expand(b"hs/room-id/v1", &mut id)
        .expect("长度合法");
    (key, id)
}

fn check_bits(payload: &[u8; PAYLOAD_LEN]) -> u16 {
    let h = sha256(&[b"gang/team/v2", payload]);
    (u16::from(h[0]) << 2) | u16::from(h[1] >> 6)
}

/// Crockford Base32：把前 `nbits` 位编码成字符。
fn base32_encode(data: &[u8], nbits: usize) -> String {
    let chars = nbits.div_ceil(5);
    let mut out = String::with_capacity(chars);
    for c in 0..chars {
        let mut v = 0u8;
        for b in 0..5 {
            let bit = c * 5 + b;
            v <<= 1;
            if bit < nbits && data[bit / 8] & (0x80 >> (bit % 8)) != 0 {
                v |= 1;
            }
        }
        out.push(ALPHABET[v as usize] as char);
    }
    out
}

/// 生成 GT2-XXXX-… 长码（和客户端 `RoomCode::format` 完全一致）。
pub fn format_code(secret: &[u8; 16], owner_tag: &[u8; 10]) -> String {
    let mut payload = [0u8; PAYLOAD_LEN];
    payload[..16].copy_from_slice(secret);
    payload[16..].copy_from_slice(owner_tag);
    let check = check_bits(&payload);
    let mut data = payload.to_vec();
    data.push((check >> 2) as u8);
    data.push(((check & 0b11) << 6) as u8);
    let body = base32_encode(&data, NBITS);
    let groups: Vec<&str> = body
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).expect("Base32 输出是 ASCII"))
        .collect();
    format!("{PREFIX}-{}", groups.join("-"))
}
