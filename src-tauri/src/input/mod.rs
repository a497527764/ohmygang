//! 全局键鼠输入（架构第 2 节）。
//!
//! - `gesture`：不认平台的手势状态机（纯函数）。
//! - 各平台的监听放在自己的文件里，用 `cfg` 选：Windows 是 `win32.rs`，macOS 是 `macos.rs`（CGEventTap），
//!   Linux 是 `linux.rs`（X11，XInput2 + XTest）；macOS 和 Linux 是实验版，没在真机上测过。别的系统用 `stub.rs`。
//! - `keymap`：macOS、X11 的按键换成 Windows 键码（纯函数，每个平台都测）。
//! - 对外只有 `start` 和 `InputHandle`：启动后监听跑在自己的线程上，要画的东西（`Emit`）交给传进来的 `Sink`，
//!   设置和开关通过 `InputHandle::send` 送进去。

pub mod gesture;
pub mod keymap;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod stub;
#[cfg(windows)]
mod win32;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
use stub as platform;
#[cfg(windows)]
use win32 as platform;

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub use gesture::{Config, Emit, Trigger, WheelKind};

/// 最后一次真实键鼠输入的时间（Unix 毫秒），0 表示还没见过。平台层在钩子回调里更新，用来判断离开电脑。
static LAST_INPUT_MS: AtomicU64 = AtomicU64::new(0);

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 平台层收到一个真实（不是自己回放的）键鼠事件时调用。
#[cfg_attr(
    not(any(windows, target_os = "macos", target_os = "linux")),
    allow(dead_code)
)]
fn touch() {
    LAST_INPUT_MS.store(unix_ms(), Ordering::Relaxed);
}

/// 鼠标最后的位置（物理像素，x 在高 32 位、y 在低 32 位），对线模式要用。
static CURSOR: AtomicU64 = AtomicU64::new(u64::MAX);

/// 平台层收到真实的鼠标移动时调用。
#[cfg_attr(
    not(any(windows, target_os = "macos", target_os = "linux")),
    allow(dead_code)
)]
fn set_cursor(x: i32, y: i32) {
    CURSOR.store(
        (u64::from(x as u32) << 32) | u64::from(y as u32),
        Ordering::Relaxed,
    );
}

/// 鼠标现在的位置（物理像素）；平台层还没报告过时返回 None。
pub fn cursor() -> Option<(i32, i32)> {
    match CURSOR.load(Ordering::Relaxed) {
        u64::MAX => None,
        v => Some(((v >> 32) as u32 as i32, v as u32 as i32)),
    }
}

/// 监听起来了但有问题时给用户看的提示（Linux：有组合键被窗口管理器占着，抓不到）。
static WARNING: Mutex<Option<String>> = Mutex::new(None);

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn set_warning(w: Option<String>) {
    *WARNING.lock().expect("提示锁没有中毒") = w;
}

/// 现在的提示；没有问题时是 None。
pub fn warning() -> Option<String> {
    WARNING.lock().expect("提示锁没有中毒").clone()
}

/// 多久没碰键鼠了；平台层还没报告过输入时返回 None。
pub fn idle_ms() -> Option<u64> {
    match LAST_INPUT_MS.load(Ordering::Relaxed) {
        0 => None,
        t => Some(unix_ms().saturating_sub(t)),
    }
}

/// 送进输入线程的命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Config(Config),
    /// 开关信号（托盘、控制面板）。
    Enabled(bool),
    /// 设置页录按键时挂起，键都交给控制面板。
    Suspended(bool),
    /// 对线模式里有没有充能完成的对手（有的话信号触发键加单击是普攻）。
    AttackReady(bool),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_packs_negative_coordinates() {
        set_cursor(-1920, 1079);
        assert_eq!(cursor(), Some((-1920, 1079)));
        set_cursor(3840, -5);
        assert_eq!(cursor(), Some((3840, -5)));
    }
}
