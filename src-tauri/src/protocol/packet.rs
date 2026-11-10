//! 加密包（架构 6.1）：`"HS"`（2）‖ proto（1）‖ room_id（16）‖ nonce（24）‖ 密文（含 16 字节认证码）。
//!
//! 实现流程：
//! 1. 封包：帧编码成 CBOR，用房间密钥加密（XChaCha20-Poly1305，随机 nonce），包头前 19 字节作关联数据。
//!    整包不超过 1200 字节；帧太长时，Events 按事件、Have 按向量项拆成多个帧分别封包。
//! 2. 拆包：先看包头——不是 "HS" 开头的不是我们的包；proto 不同直接报版本不一致，不再解密；
//!    `peek_room_id` 不用密钥就能读出 room_id，用来找到这个房间的密钥。
//! 3. 解密失败（密钥不对、被改动）或解出的帧看不懂，都返回错误，由调用方丢弃。

use crate::crypto::{self, CryptoError, RoomId, RoomKey};
use crate::protocol::frame::{Frame, MAX_FRAME_LEN, decode_frame, encode_frame};

pub const MAGIC: &[u8; 2] = b"HS";
pub const PROTO: u8 = 2;
pub const HEADER_LEN: usize = 43;
const AAD_LEN: usize = 19;
const TAG_LEN: usize = 16;
pub const MAX_PACKET_LEN: usize = 1200;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PacketError {
    #[error("不是本应用的包")]
    NotOurs,
    #[error("对方协议版本 {0} 与本机不同")]
    ProtocolMismatch(u8),
    #[error("解密失败")]
    Decrypt,
    #[error("帧无法解析")]
    BadFrame,
}

pub fn peek_room_id(raw: &[u8]) -> Option<RoomId> {
    if raw.len() < HEADER_LEN + TAG_LEN || &raw[..2] != MAGIC {
        return None;
    }
    raw[3..19].try_into().ok()
}

fn seal_one(key: &RoomKey, room_id: &RoomId, frame_bytes: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let nonce = crypto::random_bytes::<24>()?;
    let mut out = Vec::with_capacity(HEADER_LEN + frame_bytes.len() + TAG_LEN);
    out.extend_from_slice(MAGIC);
    out.push(PROTO);
    out.extend_from_slice(room_id);
    out.extend_from_slice(&nonce);
    let ct = crypto::seal(key, &nonce, &out[..AAD_LEN], frame_bytes);
    out.extend(ct);
    Ok(out)
}

/// 按顺序把元素装进尽量少的帧，每帧编码后不超过上限。单个元素一定装得下（帧编码测试保证）。
fn split<T: Clone>(items: &[T], make: impl Fn(Vec<T>) -> Frame) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut chunk: Vec<T> = Vec::new();
    for item in items {
        chunk.push(item.clone());
        if encode_frame(&make(chunk.clone())).len() > MAX_FRAME_LEN && chunk.len() > 1 {
            let last = chunk.pop().expect("chunk 至少有两项");
            out.push(encode_frame(&make(std::mem::take(&mut chunk))));
            chunk.push(last);
        }
    }
    if !chunk.is_empty() || out.is_empty() {
        out.push(encode_frame(&make(chunk)));
    }
    out
}

pub fn seal_packets(
    key: &RoomKey,
    room_id: &RoomId,
    frame: &Frame,
) -> Result<Vec<Vec<u8>>, CryptoError> {
    let whole = encode_frame(frame);
    let frames = if whole.len() <= MAX_FRAME_LEN {
        vec![whole]
    } else {
        match frame {
            Frame::Events { live, events } => split(events, |events| Frame::Events {
                live: *live,
                events,
            }),
            Frame::Have {
                from,
                period_s,
                vector,
            } => split(vector, |vector| Frame::Have {
                from: *from,
                period_s: *period_s,
                vector,
            }),
            Frame::Want { to, ranges } => split(ranges, |ranges| Frame::Want { to: *to, ranges }),
            // 实时帧都很小，放得下一个包
            Frame::Live(_) => vec![whole],
        }
    };
    frames.iter().map(|f| seal_one(key, room_id, f)).collect()
}

pub fn open_packet(key: &RoomKey, raw: &[u8]) -> Result<Frame, PacketError> {
    if raw.len() < HEADER_LEN + TAG_LEN || &raw[..2] != MAGIC {
        return Err(PacketError::NotOurs);
    }
    if raw[2] != PROTO {
        return Err(PacketError::ProtocolMismatch(raw[2]));
    }
    let nonce: [u8; 24] = raw[19..HEADER_LEN]
        .try_into()
        .map_err(|_| PacketError::NotOurs)?;
    let plain = crypto::open(key, &nonce, &raw[..AAD_LEN], &raw[HEADER_LEN..])
        .map_err(|_| PacketError::Decrypt)?;
    decode_frame(&plain).ok_or(PacketError::BadFrame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Identity;
    use crate::protocol::body::{self, ProfileBody};
    use crate::protocol::event::{build_event, kind};
    use crate::protocol::frame::{HaveEntry, WireEvent};

    const ROOM: [u8; 16] = [1u8; 16];
    const KEY: [u8; 32] = [9u8; 32];

    fn wire(seq: u64, len: usize) -> WireEvent {
        let me = Identity::from_secret(&[7u8; 32]);
        let b = body::encode(&ProfileBody {
            nickname: "字".repeat(len / 3),
            avatar: None,
        });
        WireEvent::from_signed(&build_event(&me, ROOM, seq, seq * 1000, kind::PROFILE, b))
    }

    fn one(frame: &Frame) -> Vec<u8> {
        let packets = seal_packets(&KEY, &ROOM, frame).unwrap();
        assert_eq!(packets.len(), 1);
        packets.into_iter().next().unwrap()
    }

    #[test]
    fn seal_open_roundtrip_and_peek() {
        let f = Frame::Events {
            live: true,
            events: vec![wire(1, 30)],
        };
        let p = one(&f);
        assert!(p.len() <= MAX_PACKET_LEN);
        assert_eq!(peek_room_id(&p), Some(ROOM));
        assert_eq!(open_packet(&KEY, &p), Ok(f));
    }

    #[test]
    fn tampering_or_wrong_key_fails() {
        let p = one(&Frame::Events {
            live: true,
            events: vec![wire(1, 30)],
        });
        for i in [0usize, 5, 20, HEADER_LEN + 3, p.len() - 1] {
            let mut bad = p.clone();
            bad[i] ^= 1;
            assert!(open_packet(&KEY, &bad).is_err(), "改动第 {i} 字节应失败");
        }
        assert_eq!(open_packet(&[8u8; 32], &p), Err(PacketError::Decrypt));
    }

    #[test]
    fn other_protocol_and_garbage_are_rejected_before_decrypting() {
        let mut p = one(&Frame::Events {
            live: true,
            events: vec![wire(1, 30)],
        });
        p[2] = 1; // 1.x 的包
        assert_eq!(open_packet(&KEY, &p), Err(PacketError::ProtocolMismatch(1)));
        assert_eq!(open_packet(&KEY, b"hello"), Err(PacketError::NotOurs));
        assert_eq!(peek_room_id(b"hello"), None);
    }

    /// 截断在任何一个字节的包：只是打不开，读包头也不越界。
    #[test]
    fn truncated_packets_are_rejected_at_every_length() {
        let p = one(&Frame::Events {
            live: true,
            events: vec![wire(1, 30)],
        });
        for len in 0..p.len() {
            assert!(open_packet(&KEY, &p[..len]).is_err(), "截到 {len} 字节");
            let peek = peek_room_id(&p[..len]);
            assert!(peek.is_none() || len >= HEADER_LEN + TAG_LEN);
        }
    }

    #[test]
    fn long_events_frame_is_split_into_several_packets() {
        let events: Vec<WireEvent> = (1..=20).map(|i| wire(i, 300)).collect();
        let packets = seal_packets(
            &KEY,
            &ROOM,
            &Frame::Events {
                live: false,
                events: events.clone(),
            },
        )
        .unwrap();
        assert!(packets.len() > 1);
        let mut back = Vec::new();
        for p in &packets {
            assert!(p.len() <= MAX_PACKET_LEN, "包 {} 字节", p.len());
            match open_packet(&KEY, p).unwrap() {
                Frame::Events {
                    live: false,
                    events,
                } => back.extend(events),
                other => panic!("意外的帧 {other:?}"),
            }
        }
        assert_eq!(back, events);
    }

    #[test]
    fn long_have_is_split_into_several_frames() {
        let vector: Vec<HaveEntry> = (0..120u64)
            .map(|i| HaveEntry {
                author: i.to_be_bytes(),
                max: i * 100_000,
            })
            .collect();
        let from = [3u8; 32];
        let packets = seal_packets(
            &KEY,
            &ROOM,
            &Frame::Have {
                from,
                period_s: 5,
                vector: vector.clone(),
            },
        )
        .unwrap();
        assert!(packets.len() > 1);
        let mut back = Vec::new();
        for p in &packets {
            assert!(p.len() <= MAX_PACKET_LEN);
            match open_packet(&KEY, p).unwrap() {
                Frame::Have {
                    from: f,
                    period_s: 5,
                    vector,
                } if f == from => back.extend(vector),
                other => panic!("意外的帧 {other:?}"),
            }
        }
        assert_eq!(back, vector);
    }
}
