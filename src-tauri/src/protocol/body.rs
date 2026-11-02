//! 事件内容（body）与房间规则（架构 4.2、5.1）。
//!
//! 实现流程：
//! 1. 每种事件的内容是一个结构体，用 CBOR 编码成字节放进事件。CBOR 是二进制版的 JSON，体积小、字段可扩展。
//! 2. body 只由作者编码一次；收端按原始字节验 ID，需要时再解码，解码失败就当作这条事件不生效。
//! 3. 事件 ID 用 serde_bytes 编成 CBOR 字节串（34 字节），否则会被编成 32 个整数的数组，体积大一倍。
//! 4. `Rule::is_valid` 检查取值范围；超出范围的规则无效（架构 5.1）。

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::crypto::EventId;

pub const CHAT_TEXT_MAX_BYTES: usize = 900;
pub const NICKNAME_MAX_CHARS: usize = 32;
pub const TITLE_MAX_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub rule_epoch: u32,
    pub window_ms: u32,
    pub threshold: u32,
    pub cooldown_ms: u32,
}

impl Rule {
    pub const DEFAULT: Rule = Rule { rule_epoch: 0, window_ms: 5000, threshold: 3, cooldown_ms: 30_000 };

    pub fn is_valid(&self) -> bool {
        (1000..=60_000).contains(&self.window_ms)
            && (2..=10).contains(&self.threshold)
            && (1000..=3_600_000).contains(&self.cooldown_ms)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatBody {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomCreateBody {
    pub title: String,
    pub rule: Rule,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigNoticeBody {
    pub rule: Rule,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameBody {
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBody {
    pub nickname: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    #[serde(with = "serde_bytes")]
    pub event_id: EventId,
    pub ts_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartsteelBody {
    pub rule_epoch: u32,
    pub triggers: Vec<Trigger>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("事件内容无法解码")]
pub struct BodyError;

pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).expect("写入内存不会失败");
    out
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, BodyError> {
    ciborium::from_reader(bytes).map_err(|_| BodyError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::event::MAX_BODY_LEN;

    #[test]
    fn roundtrip_all_bodies() {
        let chat = ChatBody { text: "你好".into() };
        assert_eq!(decode::<ChatBody>(&encode(&chat)), Ok(chat));
        let create = RoomCreateBody { title: "开黑".into(), rule: Rule::DEFAULT };
        assert_eq!(decode::<RoomCreateBody>(&encode(&create)), Ok(create));
        let hs = HeartsteelBody { rule_epoch: 3, triggers: vec![Trigger { event_id: [9u8; 32], ts_ms: 42 }] };
        assert_eq!(decode::<HeartsteelBody>(&encode(&hs)), Ok(hs));
    }

    #[test]
    fn garbage_does_not_decode() {
        assert_eq!(decode::<ChatBody>(&[0xff, 0x00, 0x13]), Err(BodyError));
        assert_eq!(decode::<ChatBody>(&encode(&ProfileBody { nickname: "x".into() })), Err(BodyError));
    }

    #[test]
    fn largest_bodies_fit() {
        // 阈值 10 的叠层事件、900 字节的聊天都放得下
        let hs = HeartsteelBody {
            rule_epoch: u32::MAX,
            triggers: (0..10).map(|i| Trigger { event_id: [0xab; 32], ts_ms: u64::MAX - i }).collect(),
        };
        let n = encode(&hs).len();
        assert!(n <= MAX_BODY_LEN, "叠层 body {n} 字节");
        let chat = ChatBody { text: "a".repeat(CHAT_TEXT_MAX_BYTES) };
        assert!(encode(&chat).len() <= MAX_BODY_LEN);
    }

    #[test]
    fn rule_ranges() {
        assert!(Rule::DEFAULT.is_valid());
        assert!(!Rule { window_ms: 999, ..Rule::DEFAULT }.is_valid());
        assert!(!Rule { window_ms: 60_001, ..Rule::DEFAULT }.is_valid());
        assert!(!Rule { threshold: 1, ..Rule::DEFAULT }.is_valid());
        assert!(!Rule { threshold: 11, ..Rule::DEFAULT }.is_valid());
        assert!(!Rule { cooldown_ms: 999, ..Rule::DEFAULT }.is_valid());
        assert!(!Rule { cooldown_ms: 3_600_001, ..Rule::DEFAULT }.is_valid());
        assert!(Rule { window_ms: 1000, threshold: 10, cooldown_ms: 3_600_000, rule_epoch: 9 }.is_valid());
    }
}
