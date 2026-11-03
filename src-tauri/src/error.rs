//! 统一错误类型（接口文档第 0 节）。业务函数都返回 `AppResult<T>`，UI 层按变体显示提示。

use crate::crypto::CryptoError;
use crate::protocol::room_code::RoomCodeError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("房间码不对：{0}")]
    InvalidRoomCode(#[from] RoomCodeError),
    #[error("房间不存在")]
    RoomNotFound,
    #[error("只有房主能这样做")]
    NotOwner,
    #[error("房间当前不能发言")]
    RoomNotActive,
    #[error("只有正在加入的群可以取消")]
    RoomNotJoining,
    #[error("消息太长了，删掉一些字再发")]
    TextTooLong,
    #[error("要回复的消息不在这个群里")]
    ReplyNotFound,
    #[error("只能发 WebP、PNG、JPEG 图片")]
    InvalidImage,
    #[error("图片太大了，压缩后仍超过 192KB")]
    ImageTooLarge,
    #[error("群名为空或太长，最多 64 个字符")]
    InvalidTitle,
    #[error("昵称太长，最多 32 个字符")]
    NicknameTooLong,
    #[error("头像图片太大或格式不对，请换一张")]
    InvalidAvatar,
    #[error("心之钢规则超出范围")]
    InvalidRule,
    #[error("中继地址必须是 wss:// 开头的完整地址：{0}")]
    InvalidRelay(String),
    #[error("主题只能是跟随系统、浅色或深色")]
    InvalidTheme,
    #[error("音量必须在 0 到 100 之间")]
    InvalidVolume,
    #[error("聊天同步条数最多 100000")]
    InvalidHistoryLimit,
    #[error("备份口令至少 6 个字符")]
    BackupPassphraseTooShort,
    #[error("口令不对，或者这不是钢一下的备份文件、文件已损坏")]
    BackupInvalid,
    #[error("这个备份来自更新版本的钢一下，请先升级")]
    BackupUnsupported,
    #[error("本机已有身份和房间，导入会把它们清空，请确认覆盖后再导入")]
    IdentityExists,
    #[error("本机密钥不可用：{0}")]
    KeyStore(String),
    #[error("本机数据解密失败，可能是本机主密钥被更换")]
    Decrypt,
    #[error("数据库错误：{0}")]
    Storage(#[from] rusqlite::Error),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

pub type AppResult<T> = Result<T, AppError>;
