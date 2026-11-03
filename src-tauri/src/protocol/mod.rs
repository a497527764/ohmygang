//! 线上格式：队伍码、事件、事件内容、包与帧、实时帧。纯函数，不做 I/O。

pub mod body;
pub mod event;
pub mod frame;
pub mod live;
pub mod packet;
pub mod room_code;
