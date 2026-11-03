//! 事件内容（body）与房间规则（架构 4.2、5.1）。
//!
//! 实现流程：
//! 1. 每种事件的内容是一个结构体，用 CBOR 编码成字节放进事件。CBOR 是二进制版的 JSON，体积小、字段可扩展。
//! 2. body 只由作者编码一次；收端按原始字节验 ID，需要时再解码，解码失败就当作这条事件不生效。
//! 3. 事件 ID 用 serde_bytes 编成 CBOR 字节串（34 字节），否则会被编成 32 个整数的数组，体积大一倍。
//! 4. `Rule::is_valid` 检查取值范围；超出范围的规则无效（架构 5.1）。
//! 5. 新字段只能是可选的、追加在末尾（V1.1 的头像）：旧版本解码时忽略不认识的字段，照样能用原有的字段；
//!    新版本解码旧事件时，缺的字段取默认值。所以加字段不需要升协议版本。
//! 6. 头像（`Avatar`）放在 Profile 里随昵称一起同步，不分片：内置头像只是两个小数字；
//!    上传的图片由界面压到 800 字节以内，只认 WebP、PNG、JPEG 的文件头。

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::crypto::EventId;

pub const CHAT_TEXT_MAX_BYTES: usize = 900;
pub const NICKNAME_MAX_CHARS: usize = 32;
pub const TITLE_MAX_CHARS: usize = 64;
/// 上传头像图片的上限。加上 32 个字的昵称，整个 Profile 仍在 1024 字节以内。
pub const AVATAR_IMAGE_MAX_BYTES: usize = 800;
/// 内置头像的图标数和底色数。界面里的列表只能在末尾追加，已有的下标不能改。
pub const AVATAR_PRESET_ICONS: u16 = 48;
pub const AVATAR_PRESET_COLORS: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub rule_epoch: u32,
    pub window_ms: u32,
    pub threshold: u32,
    pub cooldown_ms: u32,
}

impl Rule {
    pub const DEFAULT: Rule = Rule {
        rule_epoch: 0,
        window_ms: 5000,
        threshold: 3,
        cooldown_ms: 30_000,
    };

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
    /// V1.1 新增；没有时用按指纹自动生成的头像。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Avatar>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Avatar {
    /// 内置头像：第几个图标、第几种底色。
    Preset { icon: u16, color: u8 },
    /// 上传的图片。
    Image(#[serde(with = "serde_bytes")] Vec<u8>),
}

impl ProfileBody {
    /// 昵称和头像都没设置时不用写 Profile。
    pub fn has_content(&self) -> bool {
        !self.nickname.is_empty() || self.avatar.is_some()
    }
}

impl Avatar {
    /// 内置头像的下标在范围内；图片不超过上限、文件头是认识的格式。
    pub fn is_valid(&self) -> bool {
        match self {
            Avatar::Preset { icon, color } => {
                *icon < AVATAR_PRESET_ICONS && *color < AVATAR_PRESET_COLORS
            }
            Avatar::Image(b) => b.len() <= AVATAR_IMAGE_MAX_BYTES && image_mime(b).is_some(),
        }
    }
}

/// 按文件头认图片格式：WebP、PNG、JPEG，其余返回 None。
pub fn image_mime(b: &[u8]) -> Option<&'static str> {
    if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.starts_with(b"\x89PNG") {
        Some("image/png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else {
        None
    }
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
        let chat = ChatBody {
            text: "你好".into(),
        };
        assert_eq!(decode::<ChatBody>(&encode(&chat)), Ok(chat));
        let create = RoomCreateBody {
            title: "开黑".into(),
            rule: Rule::DEFAULT,
        };
        assert_eq!(decode::<RoomCreateBody>(&encode(&create)), Ok(create));
        let hs = HeartsteelBody {
            rule_epoch: 3,
            triggers: vec![Trigger {
                event_id: [9u8; 32],
                ts_ms: 42,
            }],
        };
        assert_eq!(decode::<HeartsteelBody>(&encode(&hs)), Ok(hs));
    }

    #[test]
    fn garbage_does_not_decode() {
        assert_eq!(decode::<ChatBody>(&[0xff, 0x00, 0x13]), Err(BodyError));
        assert_eq!(
            decode::<ChatBody>(&encode(&ProfileBody {
                nickname: "x".into(),
                avatar: None,
            })),
            Err(BodyError)
        );
    }

    fn webp(len: usize) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        b.resize(len, 7);
        b
    }

    #[test]
    fn profile_with_avatar_is_compatible_both_ways() {
        // 旧版本的 Profile 结构只有昵称
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct OldProfile {
            nickname: String,
        }
        let new = ProfileBody {
            nickname: "阿钢".into(),
            avatar: Some(Avatar::Image(webp(100))),
        };
        assert_eq!(decode::<ProfileBody>(&encode(&new)), Ok(new.clone()));
        assert_eq!(
            decode::<OldProfile>(&encode(&new)),
            Ok(OldProfile {
                nickname: "阿钢".into()
            })
        );
        let old = encode(&OldProfile {
            nickname: "乙".into(),
        });
        assert_eq!(
            decode::<ProfileBody>(&old),
            Ok(ProfileBody {
                nickname: "乙".into(),
                avatar: None
            })
        );
        // 没有头像时不写这个字段，和旧格式一模一样
        assert_eq!(
            encode(&ProfileBody {
                nickname: "乙".into(),
                avatar: None
            }),
            old
        );
        let preset = ProfileBody {
            nickname: String::new(),
            avatar: Some(Avatar::Preset { icon: 3, color: 7 }),
        };
        assert_eq!(decode::<ProfileBody>(&encode(&preset)), Ok(preset));
    }

    #[test]
    fn avatar_validity() {
        assert!(Avatar::Image(webp(AVATAR_IMAGE_MAX_BYTES)).is_valid());
        assert!(!Avatar::Image(webp(AVATAR_IMAGE_MAX_BYTES + 1)).is_valid());
        assert!(Avatar::Image(b"\x89PNG\r\n\x1a\n".to_vec()).is_valid());
        assert!(Avatar::Image(vec![0xff, 0xd8, 0xff, 0xe0]).is_valid());
        assert!(!Avatar::Image(b"<svg></svg>".to_vec()).is_valid());
        assert!(!Avatar::Image(vec![]).is_valid());
        assert!(
            Avatar::Preset {
                icon: AVATAR_PRESET_ICONS - 1,
                color: AVATAR_PRESET_COLORS - 1
            }
            .is_valid()
        );
        assert!(
            !Avatar::Preset {
                icon: AVATAR_PRESET_ICONS,
                color: 0
            }
            .is_valid()
        );
        assert!(
            !Avatar::Preset {
                icon: 0,
                color: AVATAR_PRESET_COLORS
            }
            .is_valid()
        );
        assert_eq!(image_mime(&webp(20)), Some("image/webp"));
    }

    #[test]
    fn largest_bodies_fit() {
        // 阈值 10 的叠层事件、900 字节的聊天都放得下
        let hs = HeartsteelBody {
            rule_epoch: u32::MAX,
            triggers: (0..10)
                .map(|i| Trigger {
                    event_id: [0xab; 32],
                    ts_ms: u64::MAX - i,
                })
                .collect(),
        };
        let n = encode(&hs).len();
        assert!(n <= MAX_BODY_LEN, "叠层 body {n} 字节");
        let chat = ChatBody {
            text: "a".repeat(CHAT_TEXT_MAX_BYTES),
        };
        assert!(encode(&chat).len() <= MAX_BODY_LEN);
        // 32 个四字节字符的昵称加上最大的头像图片
        let profile = ProfileBody {
            nickname: "😀".repeat(NICKNAME_MAX_CHARS),
            avatar: Some(Avatar::Image(webp(AVATAR_IMAGE_MAX_BYTES))),
        };
        let n = encode(&profile).len();
        assert!(n <= MAX_BODY_LEN, "Profile body {n} 字节");
    }

    #[test]
    fn rule_ranges() {
        assert!(Rule::DEFAULT.is_valid());
        assert!(
            !Rule {
                window_ms: 999,
                ..Rule::DEFAULT
            }
            .is_valid()
        );
        assert!(
            !Rule {
                window_ms: 60_001,
                ..Rule::DEFAULT
            }
            .is_valid()
        );
        assert!(
            !Rule {
                threshold: 1,
                ..Rule::DEFAULT
            }
            .is_valid()
        );
        assert!(
            !Rule {
                threshold: 11,
                ..Rule::DEFAULT
            }
            .is_valid()
        );
        assert!(
            !Rule {
                cooldown_ms: 999,
                ..Rule::DEFAULT
            }
            .is_valid()
        );
        assert!(
            !Rule {
                cooldown_ms: 3_600_001,
                ..Rule::DEFAULT
            }
            .is_valid()
        );
        assert!(
            Rule {
                window_ms: 1000,
                threshold: 10,
                cooldown_ms: 3_600_000,
                rule_epoch: 9
            }
            .is_valid()
        );
    }
}
