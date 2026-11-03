//! 签名事件（架构第 4 节）。
//!
//! 实现流程：
//! 1. 作者组装 `UnsignedEvent`：房间、作者公钥、stream（0 = 状态，1 = 聊天）、作者内连续的 seq、
//!    严格递增的 ts_ms、kind 和已编码好的 body 字节。
//! 2. event_id = SHA-256("hs/event/v1" ‖ 各字段定长大端拼接 ‖ body 长度（4 字节）‖ body)。
//!    ID 由内容决定，同一个 ID 不可能对应两份内容；body 按原始字节参与哈希，收端不重新编码。
//! 3. sig = Ed25519(作者私钥, "hs/sig/v1" ‖ event_id)。memberId 就是公钥，验签不需要查表。
//! 4. 收端只做无状态校验（架构 4.3）：格式版本、body 大小、ID 与内容相符、签名、房间、状态事件序号上限。
//!    事件是否「有效」（比如叠层是否成立）放到重放时算，入库与否不取决于到达顺序。

use crate::crypto::{self, EventId, Identity, MemberId, RoomId};

pub const FORMAT_V1: u8 = 1;
pub const MAX_BODY_LEN: usize = 1024;
pub const MAX_STATE_SEQ: u64 = 100_000;

/// 事件类型编号。未知编号的事件照常入库，但不显示、不参与计算。
pub mod kind {
    pub const ROOM_CREATE: u16 = 1;
    pub const JOIN: u16 = 2;
    pub const LEAVE: u16 = 3;
    pub const PROFILE: u16 = 4;
    pub const RENAME: u16 = 5;
    pub const CONFIG_NOTICE: u16 = 6;
    pub const DISSOLVE: u16 = 7;
    pub const HEARTSTEEL: u16 = 8;
    pub const CHAT: u16 = 100;
    /// 图片的一片（V1.1），不单独显示。
    pub const IMAGE_CHUNK: u16 = 101;
    /// 图片消息（V1.1），前面连续的 n 个聊天序号是它的各片。
    pub const IMAGE: u16 = 102;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stream {
    State = 0,
    Chat = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsignedEvent {
    pub format: u8,
    pub room_id: RoomId,
    pub author_id: MemberId,
    pub stream: Stream,
    pub seq: u64,
    pub ts_ms: u64,
    pub kind: u16,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedEvent {
    pub event: UnsignedEvent,
    pub event_id: EventId,
    pub sig: [u8; 64],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EventError {
    #[error("不支持的事件格式 {0}")]
    UnsupportedFormat(u8),
    #[error("事件内容过大：{0} 字节")]
    BodyTooLarge(usize),
    #[error("事件 ID 与内容不符")]
    BadId,
    #[error("签名无效")]
    BadSignature,
    #[error("事件不属于当前房间")]
    WrongRoom,
    #[error("状态事件序号过大：{0}")]
    StateSeqTooLarge(u64),
}

impl UnsignedEvent {
    pub fn compute_id(&self) -> EventId {
        crypto::sha256(&[
            b"hs/event/v1",
            &[self.format],
            &self.room_id,
            &self.author_id,
            &[self.stream as u8],
            &self.seq.to_be_bytes(),
            &self.ts_ms.to_be_bytes(),
            &self.kind.to_be_bytes(),
            &(self.body.len() as u32).to_be_bytes(),
            &self.body,
        ])
    }
}

/// 以本机身份构建并签名一条事件，作者字段取自身份，不会填错。
pub fn build_event(
    me: &Identity,
    room_id: RoomId,
    stream: Stream,
    seq: u64,
    ts_ms: u64,
    kind: u16,
    body: Vec<u8>,
) -> SignedEvent {
    let event = UnsignedEvent {
        format: FORMAT_V1,
        room_id,
        author_id: me.member_id(),
        stream,
        seq,
        ts_ms,
        kind,
        body,
    };
    let event_id = event.compute_id();
    let sig = me.sign_event_id(&event_id);
    SignedEvent {
        event,
        event_id,
        sig,
    }
}

/// 架构 4.3 的无状态校验。结果不依赖本机已有什么，所以不会造成成员间分歧。
pub fn verify_stateless(e: &SignedEvent, room_id: &RoomId) -> Result<(), EventError> {
    let ev = &e.event;
    if ev.format != FORMAT_V1 {
        return Err(EventError::UnsupportedFormat(ev.format));
    }
    if ev.body.len() > MAX_BODY_LEN {
        return Err(EventError::BodyTooLarge(ev.body.len()));
    }
    if ev.compute_id() != e.event_id {
        return Err(EventError::BadId);
    }
    if !crypto::verify_event_sig(&ev.author_id, &e.event_id, &e.sig) {
        return Err(EventError::BadSignature);
    }
    if &ev.room_id != room_id {
        return Err(EventError::WrongRoom);
    }
    if ev.stream == Stream::State && ev.seq > MAX_STATE_SEQ {
        return Err(EventError::StateSeqTooLarge(ev.seq));
    }
    Ok(())
}

/// ts_ms = max(本机时间, 本人上一事件 ts_ms + 1)，保证同一作者严格递增，时钟回拨也不影响（架构 4.1）。
pub fn next_ts(now_ms: u64, last_ts: Option<u64>) -> u64 {
    match last_ts {
        Some(last) => now_ms.max(last + 1),
        None => now_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn golden_event() -> SignedEvent {
        let me = Identity::from_secret(&[7u8; 32]);
        build_event(
            &me,
            [1u8; 16],
            Stream::Chat,
            1,
            1_700_000_000_000,
            kind::CHAT,
            b"hello".to_vec(),
        )
    }

    #[test]
    fn golden_id_and_sig() {
        let e = golden_event();
        assert_eq!(hex(&e.event_id), GOLDEN_EVENT_ID);
        assert_eq!(hex(&e.sig), GOLDEN_SIG);
        assert_eq!(verify_stateless(&e, &[1u8; 16]), Ok(()));
    }

    #[test]
    fn tampering_any_field_is_detected() {
        let room = [1u8; 16];
        let good = golden_event();
        let mutations: [fn(&mut UnsignedEvent); 6] = [
            |e| e.author_id[0] ^= 1,
            |e| e.stream = Stream::State,
            |e| e.seq += 1,
            |e| e.ts_ms += 1,
            |e| e.kind = kind::JOIN,
            |e| e.body.push(b'!'),
        ];
        for m in mutations {
            let mut bad = good.clone();
            m(&mut bad.event);
            assert_eq!(verify_stateless(&bad, &room), Err(EventError::BadId));
        }
        let mut bad = good.clone();
        bad.sig[0] ^= 1;
        assert_eq!(verify_stateless(&bad, &room), Err(EventError::BadSignature));
        // 篡改 room_id 后 ID 先对不上
        let mut bad = good.clone();
        bad.event.room_id[0] ^= 1;
        assert_eq!(verify_stateless(&bad, &room), Err(EventError::BadId));
    }

    #[test]
    fn rejects_wrong_room_format_size_and_seq() {
        let me = Identity::from_secret(&[7u8; 32]);
        let e = golden_event();
        assert_eq!(verify_stateless(&e, &[2u8; 16]), Err(EventError::WrongRoom));

        let big = build_event(
            &me,
            [1u8; 16],
            Stream::Chat,
            1,
            1,
            kind::CHAT,
            vec![0; MAX_BODY_LEN + 1],
        );
        assert_eq!(
            verify_stateless(&big, &[1u8; 16]),
            Err(EventError::BodyTooLarge(MAX_BODY_LEN + 1))
        );

        let seq = build_event(
            &me,
            [1u8; 16],
            Stream::State,
            MAX_STATE_SEQ + 1,
            1,
            kind::JOIN,
            vec![],
        );
        assert_eq!(
            verify_stateless(&seq, &[1u8; 16]),
            Err(EventError::StateSeqTooLarge(MAX_STATE_SEQ + 1))
        );
        // 聊天序号不受这个上限约束
        let chat = build_event(
            &me,
            [1u8; 16],
            Stream::Chat,
            MAX_STATE_SEQ + 1,
            1,
            kind::CHAT,
            vec![],
        );
        assert_eq!(verify_stateless(&chat, &[1u8; 16]), Ok(()));

        let mut v2 = golden_event();
        v2.event.format = 2;
        assert_eq!(
            verify_stateless(&v2, &[1u8; 16]),
            Err(EventError::UnsupportedFormat(2))
        );
    }

    #[test]
    fn next_ts_is_strictly_increasing() {
        assert_eq!(next_ts(1000, None), 1000);
        assert_eq!(next_ts(1000, Some(500)), 1000);
        assert_eq!(next_ts(1000, Some(1000)), 1001);
        assert_eq!(next_ts(900, Some(1000)), 1001); // 时钟回拨
    }

    const GOLDEN_EVENT_ID: &str =
        "2ab3149a37d2651e1adfa67d95452b0806c8789a4e0212b86c64f14eda640d23";
    const GOLDEN_SIG: &str = "59918b58a3b9ae0074fbf412258816a68c324e08f3ac3045bee09b32e38b331ec6be8b9a21777bcdd514aeb8ed7619d106ecd5539c74c3c58a5e19b63933a20b";
}
