//! 同步（架构第 7 节）。M1-3 只做实时广播、周期 Have、按 Want 补齐状态事件和新成员激活。

pub mod path;
pub mod plan;
pub mod resend;
