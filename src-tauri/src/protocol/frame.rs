//! 帧：包里加密的内容（1.x 架构 6.2，2.0 架构第 4、5 节）。
//!
//! 实现流程：
//! 1. 事件在线上用 `WireEvent`：一个 CBOR 数组 `[作者公钥, seq, ts_ms, kind, body, sig]`。
//!    room_id 已在包头、event_id 能由内容算出，都不传；收端用包头的 room_id 补上并重算 event_id，
//!    再交给 `verify_stateless` 验签。把别的队伍的事件塞进来，因 room_id 不同 event_id 也不同，验签必然失败。
//! 2. 帧是 CBOR 枚举，变体名和字段名都用一两个字母，节省字节。
//! 3. 2.0 去掉了 1.x 的聊天补齐（WantFloors、Floors），加了实时帧 `Live`（见 `live.rs`）。
//! 4. `decode_frame` 解不出来（损坏、或将来新增的帧类型）就返回 None，由调用方忽略。

use serde::{Deserialize, Serialize};

use crate::crypto::{MemberId, RoomId};
use crate::protocol::event::{FORMAT_V2, SignedEvent, UnsignedEvent};
use crate::protocol::live::WireLive;

/// 包 1200 字节，减去包头 43 字节和认证码 16 字节，留给帧的上限。
pub const MAX_FRAME_LEN: usize = 1200 - 43 - 16;

/// 公钥前 8 字节，只在同步帧里用，节省字节。
pub type ShortId = [u8; 8];

pub fn short_id(member: &MemberId) -> ShortId {
    let mut s = [0u8; 8];
    s.copy_from_slice(&member[..8]);
    s
}

/// 线上的事件：[作者公钥, seq, ts_ms, kind, body, sig]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEvent(
    #[serde(with = "serde_bytes")] pub MemberId,
    pub u64,
    pub u64,
    pub u16,
    #[serde(with = "serde_bytes")] pub Vec<u8>,
    #[serde(with = "serde_bytes")] pub [u8; 64],
);

impl WireEvent {
    pub fn from_signed(e: &SignedEvent) -> Self {
        let ev = &e.event;
        WireEvent(
            ev.author_id,
            ev.seq,
            ev.ts_ms,
            ev.kind,
            ev.body.clone(),
            e.sig,
        )
    }

    /// 用包头的 room_id 还原，并重算 event_id。是否可信由 `verify_stateless` 决定。
    pub fn into_signed(self, room_id: RoomId) -> SignedEvent {
        let WireEvent(author_id, seq, ts_ms, kind, body, sig) = self;
        let event = UnsignedEvent {
            format: FORMAT_V2,
            room_id,
            author_id,
            seq,
            ts_ms,
            kind,
            body,
        };
        let event_id = event.compute_id();
        SignedEvent {
            event,
            event_id,
            sig,
        }
    }
}

/// Have 里的一项：某作者从 1 起连续持有到的最大序号。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HaveEntry {
    #[serde(rename = "a", with = "serde_bytes")]
    pub author: ShortId,
    #[serde(rename = "s")]
    pub max: u64,
}

/// 向某位同伴要某作者的一段序号（含两端）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WantRange {
    #[serde(rename = "a", with = "serde_bytes")]
    pub author: ShortId,
    #[serde(rename = "f")]
    pub from_seq: u64,
    #[serde(rename = "t")]
    pub to_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frame {
    /// 实时广播或补齐响应。
    #[serde(rename = "E")]
    Events {
        #[serde(rename = "l")]
        live: bool,
        #[serde(rename = "e")]
        events: Vec<WireEvent>,
    },
    /// 在线心跳兼确认：发送方在这条路径上的周期和持有向量。
    #[serde(rename = "H")]
    Have {
        #[serde(rename = "f", with = "serde_bytes")]
        from: MemberId,
        #[serde(rename = "p")]
        period_s: u16,
        #[serde(rename = "v")]
        vector: Vec<HaveEntry>,
    },
    /// 向 `to` 要缺的事件。
    #[serde(rename = "W")]
    Want {
        #[serde(rename = "t", with = "serde_bytes")]
        to: ShortId,
        #[serde(rename = "r")]
        ranges: Vec<WantRange>,
    },
    /// 实时帧：信号、在线状态、鼠标等，不入库、不补齐。
    #[serde(rename = "L")]
    Live(WireLive),
}

pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(frame, &mut out).expect("写入内存不会失败");
    out
}

pub fn decode_frame(bytes: &[u8]) -> Option<Frame> {
    ciborium::from_reader(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Identity;
    use crate::protocol::body::{self, Avatar, ProfileBody};
    use crate::protocol::event::{EventError, build_event, kind, verify_stateless};
    use crate::protocol::live::{LiveBody, sign_live};

    const ROOM: [u8; 16] = [1u8; 16];

    fn me() -> Identity {
        Identity::from_secret(&[7u8; 32])
    }

    fn profile(name: &str) -> SignedEvent {
        let b = body::encode(&ProfileBody {
            nickname: name.into(),
            avatar: None,
        });
        build_event(&me(), ROOM, 1, 1_700_000_000_000, kind::PROFILE, b)
    }

    #[test]
    fn wire_event_restores_a_verifiable_event() {
        let e = profile("你好");
        let back = WireEvent::from_signed(&e).into_signed(ROOM);
        assert_eq!(back, e);
        assert_eq!(verify_stateless(&back, &ROOM), Ok(()));
    }

    #[test]
    fn restoring_into_another_room_fails_verification() {
        let e = profile("你好");
        let other = [2u8; 16];
        let moved = WireEvent::from_signed(&e).into_signed(other);
        assert_eq!(
            verify_stateless(&moved, &other),
            Err(EventError::BadSignature)
        );
    }

    #[test]
    fn frames_roundtrip() {
        let frames = vec![
            Frame::Events {
                live: true,
                events: vec![WireEvent::from_signed(&profile("一"))],
            },
            Frame::Have {
                from: me().member_id(),
                period_s: 5,
                vector: vec![HaveEntry {
                    author: short_id(&me().member_id()),
                    max: 3,
                }],
            },
            Frame::Want {
                to: [9u8; 8],
                ranges: vec![WantRange {
                    author: [8u8; 8],
                    from_seq: 2,
                    to_seq: 5,
                }],
            },
            Frame::Live(sign_live(
                &me(),
                &ROOM,
                42,
                1_700_000_000_000,
                &LiveBody::Ping {
                    display: 1,
                    x: 100,
                    y: 65535,
                    id: "danger".into(),
                },
            )),
        ];
        for f in frames {
            assert_eq!(decode_frame(&encode_frame(&f)), Some(f));
        }
    }

    #[test]
    fn garbage_and_unknown_frames_are_ignored() {
        assert_eq!(decode_frame(&[0xff, 0x00, 0x13]), None);
        let mut unknown = Vec::new();
        ciborium::into_writer(
            &std::collections::BTreeMap::from([("Z", 1u8)]),
            &mut unknown,
        )
        .unwrap();
        assert_eq!(decode_frame(&unknown), None);
    }

    /// 解密后的帧来自队友，内容可能是坏的：嵌套到包能装下的最深、谎报 2^64 那么长，都只是解不出来，
    /// 不爆栈、不按谎报的长度分配内存。不认识的字段会被整个跳过，跳的时候要一层层走进去，所以坏内容放在那里。
    #[test]
    fn deep_nesting_and_lying_lengths_are_rejected() {
        // {"L": {"zz": <value>}}
        let in_unknown_field = |value: &[u8]| {
            let mut b = vec![0xa1, 0x61, b'L', 0xa1, 0x62, b'z', b'z'];
            b.extend_from_slice(value);
            b
        };
        let mut deep = vec![0x81; MAX_FRAME_LEN]; // [[[[…0]]]]
        deep.push(0x00);
        assert_eq!(decode_frame(&in_unknown_field(&deep)), None);
        let mut deep_maps = [0xa1, 0x00].repeat(MAX_FRAME_LEN / 2); // {0: {0: …}}
        deep_maps.push(0x00);
        assert_eq!(decode_frame(&in_unknown_field(&deep_maps)), None);
        // 字节串、文字、数组、表的长度都写成 2^64-1，后面只跟 4 个字节
        for head in [0x5b, 0x7b, 0x9b, 0xbb] {
            let mut lie = vec![head];
            lie.extend([0xff; 8]);
            lie.extend([0; 4]);
            assert_eq!(decode_frame(&in_unknown_field(&lie)), None);
        }
        // 事件列表谎报 2^63 项：{"E": {"l": true, "e": [<2^63 项>…]}}
        let mut events = vec![
            0xa1, 0x61, b'E', 0xa2, 0x61, b'l', 0xf5, 0x61, b'e', 0x9b, 0x7f,
        ];
        events.extend([0xff; 7]);
        events.extend([0x80; 4]);
        assert_eq!(decode_frame(&events), None);
    }

    #[test]
    fn largest_events_fit_in_one_packet() {
        let fits = |e: SignedEvent| {
            encode_frame(&Frame::Events {
                live: true,
                events: vec![WireEvent::from_signed(&e)],
            })
            .len()
        };
        let small = fits(profile("你好"));
        assert!(small <= 200, "普通资料帧 {small} 字节");
        let big = body::encode(&ProfileBody {
            nickname: "字".repeat(body::NICKNAME_MAX_CHARS),
            avatar: Some(Avatar::Image(vec![0xab; body::AVATAR_IMAGE_MAX_BYTES])),
        });
        let len = fits(build_event(
            &me(),
            ROOM,
            u64::MAX,
            u64::MAX,
            kind::PROFILE,
            big,
        ));
        assert!(len <= MAX_FRAME_LEN, "最大的资料帧 {len} 字节");
    }
}
