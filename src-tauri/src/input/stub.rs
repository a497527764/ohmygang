//! Windows、macOS 以外的系统（包括 Linux，3.1 起不再支持）没有输入监听（架构 2.3），只保证能编译：启动时报不支持，控制面板显示这句话。

use super::Command;
use super::gesture::Config;

pub struct Handle;

impl Handle {
    pub fn send(&self, _c: Command) {}
}

pub fn start(_cfg: Config, _sink: super::Sink) -> Result<Handle, String> {
    Err("这个系统不支持全局键鼠".to_string())
}
