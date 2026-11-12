//! 传输：只收发不透明字节，不知道房间密钥，也不解析帧内容（架构第 6 节）。

pub mod budget;
pub mod flood;
pub mod nostr;
pub mod relay;

#[cfg(test)]
pub(crate) mod fake_relay;
#[cfg(test)]
mod relay_tests;
