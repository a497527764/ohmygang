//! 统一错误类型。业务函数都返回 `AppResult<T>`，控制面板按变体显示提示。

use crate::crypto::CryptoError;
use crate::protocol::room_code::RoomCodeError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("队伍码不对：{0}")]
    InvalidRoomCode(#[from] RoomCodeError),
    #[error("队伍不存在")]
    RoomNotFound,
    #[error("只有队长能这样做")]
    NotOwner,
    #[error("队伍现在不能这样操作")]
    RoomNotActive,
    #[error("只有正在加入的队伍可以取消")]
    RoomNotJoining,
    #[error("已经在一个队伍里了，先退出再建队或进队")]
    AlreadyInTeam,
    #[error("当前还在这个队伍里，先退出再删")]
    StillInTeam,
    #[error("不能对自己这么做")]
    InvalidTarget,
    #[error("队名为空或太长，最多 64 个字符")]
    InvalidTitle,
    #[error("昵称太长，最多 32 个字符")]
    NicknameTooLong,
    #[error("头像图片太大或格式不对，请换一张")]
    InvalidAvatar,
    #[error("中继地址必须是 wss:// 开头的完整地址：{0}")]
    InvalidRelay(String),
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
