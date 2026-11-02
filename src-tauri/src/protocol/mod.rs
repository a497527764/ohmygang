//! 线上格式：房间码、事件、事件内容（架构第 3、4 节）。纯函数，不做 I/O。
//! 包与帧（架构第 6 节）在 M1-3 加入。

pub mod body;
pub mod event;
pub mod frame;
pub mod packet;
pub mod room_code;
