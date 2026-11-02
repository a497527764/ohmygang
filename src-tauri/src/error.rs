//! 统一错误类型（接口文档第 0 节）。业务函数都返回 `AppResult<T>`，UI 层按变体显示提示。

use crate::crypto::CryptoError;
use crate::protocol::room_code::RoomCodeError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("房间码不对：{0}")]
    InvalidRoomCode(#[from] RoomCodeError),
    #[error("房间不存在")]
    RoomNotFound,
    #[error("房间当前不能发言")]
    RoomNotActive,
    #[error("消息太长，最多 900 字节")]
    TextTooLong,
    #[error("群名为空或太长，最多 64 个字符")]
    InvalidTitle,
    #[error("昵称太长，最多 32 个字符")]
    NicknameTooLong,
    #[error("心之钢规则超出范围")]
    InvalidRule,
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
