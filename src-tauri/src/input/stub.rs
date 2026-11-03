//! macOS 和 Linux 的输入在 M6 才做（架构 2.3），在那之前只保证能编译：启动时报不支持，控制面板显示这句话。

use super::Command;
use super::gesture::Config;

pub struct Handle;

impl Handle {
    pub fn send(&self, _c: Command) {}
}

pub fn start(_cfg: Config, _sink: super::Sink) -> Result<Handle, String> {
    Err("这个平台的输入还没做好".to_string())
}
