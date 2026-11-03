//! 全局键鼠输入（架构第 2 节）。
//!
//! - `gesture`：不认平台的手势状态机（纯函数）。
//! - 各平台的监听放在自己的文件里，用 `cfg` 选：Windows 是 `win32.rs`；macOS 和 Linux 在 M6 之前用 `stub.rs`，
//!   启动时报「这个平台的输入还没做好」。
//! - 对外只有 `start` 和 `InputHandle`：启动后监听跑在自己的线程上，要画的东西（`Emit`）交给传进来的 `Sink`，
//!   设置和开关通过 `InputHandle::send` 送进去。

pub mod gesture;
#[cfg(not(windows))]
mod stub;
#[cfg(windows)]
mod win32;

#[cfg(not(windows))]
use stub as platform;
#[cfg(windows)]
use win32 as platform;

pub use gesture::{Config, Emit, Trigger, WheelKind};

/// 送进输入线程的命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Config(Config),
    /// 开关信号（托盘、控制面板）。
    Enabled(bool),
    /// 设置页录按键时挂起，键都交给控制面板。
    Suspended(bool),
}

/// 收要画的东西。在钩子回调里调用，所以只能做不阻塞的事（往通道里放）。
pub type Sink = Box<dyn Fn(Emit) + Send>;

pub struct InputHandle(platform::Handle);

impl InputHandle {
    pub fn send(&self, c: Command) {
        self.0.send(c);
    }
}

/// 启动监听线程。失败时返回给用户看的说明（装不上钩子、这个平台还没做好）。
pub fn start(cfg: Config, sink: Sink) -> Result<InputHandle, String> {
    platform::start(cfg, sink).map(InputHandle)
}
