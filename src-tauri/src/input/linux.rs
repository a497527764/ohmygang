//! Linux（X11）的全局键鼠监听（架构 2.3，实验版，未在真机上测试）。移植自 lolPing
//! `native/hook-helper/src/hooks_linux.cpp`（MIT，© 2026 Danrui Meng）。用 x11rb，不需要 `unsafe`。
//!
//! 实现流程：
//! 1. 只支持 X11 会话：Wayland 下普通程序拿不到全局键鼠，启动时报原因。要有 XInput 2.1 和 XTest。
//! 2. 鼠标：在根窗口上对左键和两个侧键做被动抓取（同步模式），按下时交给手势状态机：
//!    要拦就 AsyncPointer（之后的移动和抬起都到我们这里），不拦就 ReplayPointer（原样交给下面的程序）。
//! 3. 键盘：用 XInput2 的原始按键事件看所有按键（不抢焦点）；只对要拦的键做被动抓取：自定义触发键、
//!    暂停、文字快捷键、面板键，以及轮盘开着时的 Esc。抓到的按键 ReplayKeyboard 放行或 AsyncKeyboard 拦下。
//! 4. 回放用 XTest：先在服务器抓取下拿掉自己的被动抓取，发完再装回来，所以回放不会又被自己抓到。
//!    XTest 发的按键也会产生原始事件，按键码记个数，回来时认出是自己的。
//! 5. 键码 → Windows 键码：先按 XKB 的物理键名（和键盘布局无关），认不出再按 keysym（`keymap`）。
//! 6. 收事件的线程和处理的线程分开：一个线程等 X 事件，和命令一起送进同一个通道；处理线程每 25 毫秒至少醒一次，
//!    每秒让状态机检查看门狗。鼠标没被抓时拿不到位置，原始移动事件来了就问一次服务器（最多每 16 毫秒一次）。
//! 7. 没有全屏检查（Windows 才有），`game_check` 在这里不起作用；面板键的连发看门狗也关掉。
//! 8. 窗口管理器已经占着某个组合（常见的是按住 Alt 拖动窗口）时，抓取会报 Access 错误：
//!    记下提示，控制面板的「信号」页显示，让用户换键；改了设置就清掉，新的抓取再出错再记。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    self, Allow, ButtonIndex, ConnectionExt as _, EventMask, GrabMode, ModMask, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::{ErrorKind, Event};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::Command;
use super::gesture::{
    Btn, EvKind, Gesture, Inject, InputEvent, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN, Outcome,
    Trigger,
};
use super::keymap::{is_modifier_vk, x11_key_name_to_vk, x11_keysym_to_vk};

const STEP: Duration = Duration::from_millis(25);
const TICK: Duration = Duration::from_secs(1);
const CURSOR_EVERY: Duration = Duration::from_millis(16);
/// 自己发的原始事件几毫秒内就回来；记数最多保留这么久。
const SELF_RAW_MS: u64 = 500;
const GRAB_CONFLICT: &str = "有触发键或快捷键被窗口管理器或别的程序占着（比如按住 Alt 拖动窗口），这些组合用不了。换一个触发键或快捷键试试";

enum Msg {
    X(Event),
    Cmd(Command),
    /// 连接断了。
    Closed,
}

pub struct Handle {
    tx: Sender<Msg>,
}

impl Handle {
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(Msg::Cmd(c));
    }
}

fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub fn start(cfg: super::gesture::Config, sink: super::Sink) -> Result<Handle, String> {
    if is_wayland() {
        return Err("Linux 只支持 X11 桌面会话：Wayland 下拿不到全局键鼠".into());
    }
    let (conn, screen) = x11rb::connect(None).map_err(|e| format!("连不上 X11：{e}"))?;
    let conn = Arc::new(conn);
    let root = conn.setup().roots[screen].root;
    let xi = conn
        .xinput_xi_query_version(2, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some_and(|r| (r.major_version, r.minor_version) >= (2, 1));
    let xtest = conn
        .xtest_get_version(2, 2)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some();
    if !xi || !xtest {
        return Err("需要 X11 的 XInput 2.1 和 XTest 扩展".into());
    }
    let (tx, rx) = mpsc::channel::<Msg>();
    let reader = conn.clone();
    let events = tx.clone();
    std::thread::Builder::new()
        .name("input-x11-events".into())
        .spawn(move || {
            loop {
                match reader.wait_for_event() {
                    Ok(e) => {
                        if events.send(Msg::X(e)).is_err() {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = events.send(Msg::Closed);
                        return;
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let mut hooks = Hooks::new(conn, root, sink, cfg).map_err(|e| format!("X11 出错：{e}"))?;
    std::thread::Builder::new()
        .name("input-x11".into())
        .spawn(move || hooks.run(rx))
        .map_err(|e| e.to_string())?;
    Ok(Handle { tx })
}

fn now_ms() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn button(n: u8) -> Btn {
    match n {
        1 => Btn::Left,
        2 => Btn::Middle,
        3 => Btn::Right,
        8 => Btn::X1,
        9 => Btn::X2,
        _ => Btn::None,
    }
}

fn button_number(b: Btn) -> u8 {
    match b {
        Btn::Left => 1,
        Btn::Middle => 2,
        Btn::Right => 3,
        Btn::X1 => 8,
        Btn::X2 => 9,
        Btn::None => 0,
    }
}

/// 手势的修饰键位 → X 的修饰键掩码（Alt 是 Mod1，Win 是 Mod4）。
fn x_mods(mods: u32) -> u16 {
    let mut m = 0u16;
    if mods & MOD_CTRL != 0 {
        m |= u16::from(ModMask::CONTROL);
    }
    if mods & MOD_ALT != 0 {
        m |= u16::from(ModMask::M1);
    }
    if mods & MOD_SHIFT != 0 {
        m |= u16::from(ModMask::SHIFT);
    }
    if mods & MOD_WIN != 0 {
        m |= u16::from(ModMask::M4);
    }
    m
}

/// 去掉 Caps Lock 和 Num Lock 再比修饰键。
fn without_locks(mods: u16) -> u16 {
    mods & !(u16::from(ModMask::LOCK) | u16::from(ModMask::M2))
}

struct Hooks {
    conn: Arc<RustConnection>,
    root: Window,
    sink: super::Sink,
    gesture: Gesture,
    suspended: bool,
    /// 键码 → Windows 键码。
    vk: [u32; 256],
    swallowed: [bool; 256],
    /// 自己抓键盘时补发的松开，会回到我们这里。
    self_core: [i32; 256],
    /// 抓着键盘的那个键：之后的键都先到我们这里。0 表示没有。
    grab_key: u8,
    /// 原始事件里按下了、还没松开的键。
    raw_down: [bool; 256],
    /// 每个 Windows 键码最近一次在哪个键码上出现（回放用同一个物理键）。
    code_of_vk: [u8; 256],
    self_raw: [i32; 256],
    self_raw_until: [u64; 256],
    pointer_active: bool,
    held_buttons: Vec<u8>,
    last_cursor: Instant,
}

impl Hooks {
    fn new(
        conn: Arc<RustConnection>,
        root: Window,
        sink: super::Sink,
        cfg: super::gesture::Config,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut gesture = Gesture::new();
        gesture.set_mask_menu_keys(false);
        gesture.set_panel_repeat_gap(0);
        gesture.set_config(cfg);
        let mut h = Self {
            conn,
            root,
            sink,
            gesture,
            suspended: false,
            vk: [0; 256],
            swallowed: [false; 256],
            self_core: [0; 256],
            grab_key: 0,
            raw_down: [false; 256],
            code_of_vk: [0; 256],
            self_raw: [0; 256],
            self_raw_until: [0; 256],
            pointer_active: false,
            held_buttons: Vec::new(),
            last_cursor: Instant::now(),
        };
        h.refresh_keys()?;
        let mask = xinput::XIEventMask::RAW_KEY_PRESS
            | xinput::XIEventMask::RAW_KEY_RELEASE
            | xinput::XIEventMask::RAW_MOTION
            | xinput::XIEventMask::RAW_BUTTON_PRESS;
        h.conn.xinput_xi_select_events(
            h.root,
            &[xinput::EventMask {
                deviceid: xinput::Device::ALL_MASTER.into(),
                mask: vec![mask],
            }],
        )?;
        // 按住的键连发时不要假的松开
        if h.conn
            .xkb_use_extension(1, 0)?
            .reply()
            .is_ok_and(|r| r.supported)
        {
            let flag = xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT;
            let _ = h
                .conn
                .xkb_per_client_flags(
                    xkb::ID::USE_CORE_KBD.into(),
                    flag,
                    flag,
                    0u32.into(),
                    0u32.into(),
                    0u32.into(),
                )?
                .reply();
        }
        h.configure_grabs()?;
        // 启动时已经按着的键
        let keys = h.conn.query_keymap()?.reply()?.keys;
        for code in 8..=255u8 {
            if keys[usize::from(code / 8)] & (1 << (code % 8)) != 0 {
                h.raw_key(code, true);
            }
        }
        h.conn.flush()?;
        Ok(h)
    }

    /// 键码表：先认物理键名，认不出再看 keysym。
    fn refresh_keys(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self
            .conn
            .get_keyboard_mapping(min, max - min + 1)?
            .reply()?;
        let per = usize::from(map.keysyms_per_keycode).max(1);
        let names = self
            .conn
            .xkb_get_names(xkb::ID::USE_CORE_KBD.into(), xkb::NameDetail::KEY_NAMES)
            .ok()
            .and_then(|c| c.reply().ok());
        self.vk = [0; 256];
        for code in min..=max {
            let i = usize::from(code - min);
            let sym = map.keysyms.get(i * per).copied().unwrap_or(0);
            let physical = names
                .as_ref()
                .and_then(|n| {
                    let first = n.first_key;
                    let list = n.value_list.key_names.as_ref()?;
                    let k = list.get(usize::from(code.checked_sub(first)?))?;
                    Some(x11_key_name_to_vk(&k.name))
                })
                .unwrap_or(0);
            self.vk[usize::from(code)] = if physical != 0 {
                physical
            } else {
                x11_keysym_to_vk(sym)
            };
        }
        Ok(())
    }

    fn run(&mut self, rx: Receiver<Msg>) {
        let mut next_tick = Instant::now() + TICK;
        loop {
            match rx.recv_timeout(STEP) {
                Ok(Msg::X(e)) => self.event(e),
                Ok(Msg::Cmd(c)) => self.command(c),
                Ok(Msg::Closed) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if Instant::now() >= next_tick {
                next_tick = Instant::now() + TICK;
                let r = self.gesture.tick(now_ms());
                self.apply(r);
            }
            let _ = self.conn.flush();
        }
    }

    fn event(&mut self, e: Event) {
        match e {
            Event::XinputRawKeyPress(r) => self.raw_key(r.detail as u8, true),
            Event::XinputRawKeyRelease(r) => self.raw_key(r.detail as u8, false),
            Event::XinputRawButtonPress(_) => super::touch(),
            Event::XinputRawMotion(_) => {
                super::touch();
                if self.last_cursor.elapsed() >= CURSOR_EVERY {
                    self.last_cursor = Instant::now();
                    if let Ok(Ok(p)) = self.conn.query_pointer(self.root).map(|c| c.reply()) {
                        super::set_cursor(i32::from(p.root_x), i32::from(p.root_y));
                    }
                }
            }
            Event::KeyPress(k) => self.core_key(k.detail, true, k.time),
            Event::KeyRelease(k) => self.core_key(k.detail, false, k.time),
            Event::ButtonPress(b) => {
                self.mouse(EvKind::MouseDown, b.detail, b.root_x, b.root_y, b.time)
            }
            Event::ButtonRelease(b) => {
                self.mouse(EvKind::MouseUp, b.detail, b.root_x, b.root_y, b.time)
            }
            Event::MotionNotify(m) => self.mouse(EvKind::MouseMove, 0, m.root_x, m.root_y, m.time),
            Event::Error(e)
                if e.error_kind == ErrorKind::Access
                    && matches!(
                        e.major_opcode,
                        xproto::GRAB_KEY_REQUEST | xproto::GRAB_BUTTON_REQUEST
                    ) =>
            {
                super::set_warning(Some(GRAB_CONFLICT.to_string()));
            }
            Event::MappingNotify(_) => {
                let _ = self.refresh_keys();
                let _ = self.configure_grabs();
            }
            _ => {}
        }
    }

    /// 抓到的按键（见文件头第 3 条）。抓着键盘时原始事件不一定来，所以这里也驱动状态机；
    /// 本来是给别的程序的键，补打一次。
    fn core_key(&mut self, code: u8, down: bool, time: u32) {
        let i = usize::from(code);
        if !down && self.self_core[i] > 0 {
            self.self_core[i] -= 1;
            return;
        }
        let outsider = self.grab_key != 0 && code != self.grab_key;
        if self.raw_down[i] != down {
            self.raw_key(code, down);
        }
        if outsider && !self.swallowed[i] {
            self.fake_keys(&[(code, down)]);
        } else if down && self.swallowed[i] && self.grab_key == 0 {
            self.grab_key = code;
        } else if !down && code == self.grab_key {
            self.grab_key = 0;
        }
        let mode = if self.swallowed[i] {
            Allow::ASYNC_KEYBOARD
        } else {
            Allow::REPLAY_KEYBOARD
        };
        let _ = self.conn.allow_events(mode, time);
    }

    fn is_self_raw(&mut self, code: usize) -> bool {
        if self.self_raw[code] <= 0 {
            return false;
        }
        if now_ms() > self.self_raw_until[code] {
            self.self_raw[code] = 0;
            return false;
        }
        self.self_raw[code] -= 1;
        true
    }

    fn raw_key(&mut self, code: u8, down: bool) {
        let i = usize::from(code);
        if code < 8 || self.vk[i] == 0 || self.is_self_raw(i) {
            return;
        }
        // 抓到的按键也会走到这里，重复的松开不算
        if !down && !self.raw_down[i] {
            return;
        }
        super::touch();
        self.raw_down[i] = down;
        let vk = self.vk[i];
        self.code_of_vk[(vk & 0xFF) as usize] = code;
        let kind = if down { EvKind::KeyDown } else { EvKind::KeyUp };
        let before = self.gesture.enabled();
        let r = self.gesture.on_event(&InputEvent::key(kind, vk, now_ms()));
        self.swallowed[i] = r.swallow;
        self.apply(r);
        if before != self.gesture.enabled() {
            let _ = self.configure_grabs();
        }
    }

    fn mouse(&mut self, kind: EvKind, detail: u8, x: i16, y: i16, time: u32) {
        let (x, y) = (i32::from(x), i32::from(y));
        super::touch();
        super::set_cursor(x, y);
        let btn = if kind == EvKind::MouseMove {
            Btn::None
        } else {
            button(detail)
        };
        let was_active = self.pointer_active;
        if kind == EvKind::MouseDown {
            self.held_buttons.push(detail);
            self.gesture.sync_key_states(|vk| {
                let code = self.code_of_vk[(vk & 0xFF) as usize];
                code != 0 && self.raw_down[usize::from(code)]
            });
        }
        if kind == EvKind::MouseUp {
            self.held_buttons.retain(|b| *b != detail);
        }
        let r = self
            .gesture
            .on_event(&InputEvent::mouse(kind, btn, x, y, now_ms()));
        if kind == EvKind::MouseDown && !was_active {
            self.pointer_active = r.swallow;
            let mode = if r.swallow {
                Allow::ASYNC_POINTER
            } else {
                Allow::REPLAY_POINTER
            };
            let _ = self.conn.allow_events(mode, time);
            if !r.swallow {
                self.held_buttons.clear();
            }
            let _ = self.configure_grabs();
        }
        self.apply(r);
        if kind == EvKind::MouseUp && self.held_buttons.is_empty() {
            let _ = self.conn.ungrab_pointer(time);
            self.pointer_active = false;
            let _ = self.configure_grabs();
        }
    }

    fn command(&mut self, c: Command) {
        let r = match c {
            Command::Config(cfg) => {
                super::set_warning(None);
                self.gesture.set_config(cfg)
            }
            Command::Enabled(on) => self.gesture.set_enabled(on),
            Command::Suspended(on) => {
                self.suspended = on;
                self.gesture.set_suspended(on)
            }
        };
        self.apply(r);
        let _ = self.configure_grabs();
    }

    fn configure_grabs(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.conn
            .ungrab_button(ButtonIndex::ANY, self.root, ModMask::ANY)?;
        self.grab_buttons()?;
        self.grab_keys()?;
        Ok(())
    }

    /// 一个一个修饰键组合抓：用 AnyModifier 的话，窗口管理器占着其中一个组合就会整个失败。
    fn grab_buttons(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mask = EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
        for b in [1u8, 8, 9] {
            for mods in 0..256u16 {
                self.conn.grab_button(
                    false,
                    self.root,
                    mask,
                    GrabMode::SYNC,
                    GrabMode::ASYNC,
                    x11rb::NONE,
                    x11rb::NONE,
                    ButtonIndex::from(b),
                    ModMask::from(mods),
                )?;
            }
        }
        Ok(())
    }

    fn grab_keys(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.conn.ungrab_key(0u8, self.root, ModMask::ANY)?;
        let c = *self.gesture.config();
        let live = self.gesture.enabled() && !self.suspended;
        let own = |vk: u32, t: Trigger| live && t == Trigger::Custom(vk);
        let chord = |vk: u32, mods: u16, want_vk: u32, want_mods: u32| {
            want_vk != 0 && vk == want_vk && without_locks(mods) == x_mods(want_mods)
        };
        for code in 8..=255u8 {
            let vk = self.vk[usize::from(code)];
            if vk == 0 {
                continue;
            }
            for mods in 0..256u16 {
                let grab = (self.pointer_active && vk == 0x1B)
                    || own(vk, c.trigger)
                    || own(vk, c.emote_trigger)
                    || (live && chord(vk, mods, c.text_vk, c.text_mods))
                    // 面板键只在不按修饰键时抓：Ctrl+Tab、Alt+Tab 照常归别的程序
                    || (live && c.panel_vk != 0 && vk == c.panel_vk && without_locks(mods) == 0)
                    || (!self.suspended && chord(vk, mods, c.toggle_vk, c.toggle_mods));
                if grab {
                    self.conn.grab_key(
                        false,
                        self.root,
                        ModMask::from(mods),
                        code,
                        GrabMode::ASYNC,
                        GrabMode::SYNC,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn apply(&mut self, r: Outcome) {
        for e in r.emits {
            (self.sink)(e);
        }
        if r.injects.is_empty() {
            return;
        }
        let keys: Vec<(u8, bool)> = r
            .injects
            .iter()
            .filter_map(|i| match *i {
                Inject::KeyDown(vk) if !is_modifier_vk(vk) => {
                    self.key_code_for(vk).map(|c| (c, true))
                }
                Inject::KeyUp(vk) if !is_modifier_vk(vk) => {
                    self.key_code_for(vk).map(|c| (c, false))
                }
                _ => None,
            })
            .collect();
        self.fake_keys(&keys);
        let buttons: Vec<(u8, bool)> = r
            .injects
            .iter()
            .filter_map(|i| match *i {
                Inject::ButtonDown(b) if button_number(b) != 0 => Some((button_number(b), true)),
                Inject::ButtonUp(b) if button_number(b) != 0 => Some((button_number(b), false)),
                _ => None,
            })
            .collect();
        if !buttons.is_empty() {
            let _ = self.replay_buttons(&buttons);
        }
    }

    /// 回放按键：在服务器抓取下暂时拿掉被动抓取，回放就不会又触发一个轮盘。
    fn replay_buttons(&mut self, buttons: &[(u8, bool)]) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn.clone();
        conn.grab_server()?;
        // 我们的抓取还在时先松开物理上按着的键，抬起由我们收
        if self.pointer_active {
            for &(n, down) in buttons {
                if down {
                    conn.xtest_fake_input(xproto::BUTTON_RELEASE_EVENT, n, 0, self.root, 0, 0, 0)?;
                }
            }
        }
        conn.sync()?;
        conn.ungrab_pointer(x11rb::CURRENT_TIME)?;
        self.pointer_active = false;
        self.held_buttons.clear();
        conn.ungrab_button(ButtonIndex::ANY, self.root, ModMask::ANY)?;
        for &(n, down) in buttons {
            let t = if down {
                xproto::BUTTON_PRESS_EVENT
            } else {
                xproto::BUTTON_RELEASE_EVENT
            };
            conn.xtest_fake_input(t, n, 0, self.root, 0, 0, 0)?;
        }
        conn.sync()?;
        self.grab_buttons()?;
        conn.ungrab_server()?;
        conn.flush()?;
        Ok(())
    }

    fn key_code_for(&self, vk: u32) -> Option<u8> {
        let seen = self.code_of_vk[(vk & 0xFF) as usize];
        if seen >= 8 {
            return Some(seen);
        }
        (8..=255u8).find(|c| self.vk[usize::from(*c)] == vk)
    }

    /// 用 XTest 打出这些键：拿掉键盘的被动抓取再打，打到下面的程序、不会又被抓到。
    fn fake_keys(&mut self, keys: &[(u8, bool)]) {
        if keys.is_empty() {
            return;
        }
        let _ = self.try_fake_keys(keys);
    }

    fn try_fake_keys(&mut self, keys: &[(u8, bool)]) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn.clone();
        conn.grab_server()?;
        // 物理上还按着的键，假的按下会被当成连发丢掉：先在只有我们收事件时松开它，下面的程序看不到这个多出来的松开。
        let mut released = false;
        for &(code, down) in keys {
            if !down || !self.raw_down[usize::from(code)] {
                continue;
            }
            if !released {
                conn.grab_keyboard(
                    false,
                    self.root,
                    x11rb::CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )?;
            }
            released = true;
            conn.xtest_fake_input(xproto::KEY_RELEASE_EVENT, code, 0, self.root, 0, 0, 0)?;
            self.self_core[usize::from(code)] += 1;
        }
        conn.sync()?;
        // 结束按着的触发键开始的键盘抓取，否则回放会回到我们这里
        conn.ungrab_keyboard(x11rb::CURRENT_TIME)?;
        self.grab_key = 0;
        conn.ungrab_key(0u8, self.root, ModMask::ANY)?;
        for &(code, down) in keys {
            let t = if down {
                xproto::KEY_PRESS_EVENT
            } else {
                xproto::KEY_RELEASE_EVENT
            };
            conn.xtest_fake_input(t, code, 0, self.root, 0, 0, 0)?;
            let i = usize::from(code);
            self.self_raw[i] += 1;
            self.self_raw_until[i] = now_ms() + SELF_RAW_MS;
        }
        conn.sync()?;
        self.grab_keys()?;
        conn.ungrab_server()?;
        conn.flush()?;
        Ok(())
    }
}

/// 在 Xvfb 里跑的集成测试：用 XTest 造真的 X11 键鼠事件，看监听交出来的手势对不对。
/// 要有 X11 显示，平时跳过；CI 的 Ubuntu 上用 `xvfb-run cargo test --lib input::linux -- --ignored` 跑。
#[cfg(test)]
mod xvfb_tests {
    use std::sync::Mutex;

    use super::*;
    use crate::input::gesture::{Config, Emit, WheelKind};

    const ALT_L: u32 = 0xFFE9;
    const CONTROL_L: u32 = 0xFFE3;
    const KEY_P: u32 = 0x70;

    fn keycode(conn: &RustConnection, keysym: u32) -> u8 {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .unwrap()
            .reply()
            .unwrap();
        let per = usize::from(map.keysyms_per_keycode);
        (min..=max)
            .find(|c| map.keysyms[usize::from(c - min) * per] == keysym)
            .expect("键盘映射里有这个键")
    }

    /// 造一个事件，等它走完。
    fn fake(conn: &RustConnection, root: Window, kind: u8, detail: u8, x: i16, y: i16) {
        conn.xtest_fake_input(kind, detail, 0, root, x, y, 0)
            .unwrap();
        conn.sync().unwrap();
        std::thread::sleep(Duration::from_millis(40));
    }

    fn key(conn: &RustConnection, root: Window, code: u8, down: bool) {
        let kind = if down {
            xproto::KEY_PRESS_EVENT
        } else {
            xproto::KEY_RELEASE_EVENT
        };
        fake(conn, root, kind, code, 0, 0);
    }

    #[test]
    #[ignore = "要 X11 显示：CI 里用 xvfb-run 跑"]
    fn x11_alt_drag_opens_the_wheel_and_pause_hotkey_toggles() {
        let emits = Arc::new(Mutex::new(Vec::<Emit>::new()));
        let sink_to = emits.clone();
        // 默认配置：Alt 加左键拖动开信号轮盘，Ctrl+Alt+P 暂停
        let _handle = start(
            Config::default(),
            Box::new(move |e| sink_to.lock().unwrap().push(e)),
        )
        .expect("Xvfb 里监听起得来");
        let (conn, screen) = x11rb::connect(None).unwrap();
        let root = conn.setup().roots[screen].root;
        std::thread::sleep(Duration::from_millis(300));

        let alt = keycode(&conn, ALT_L);
        fake(&conn, root, xproto::MOTION_NOTIFY_EVENT, 0, 200, 200);
        key(&conn, root, alt, true);
        fake(&conn, root, xproto::BUTTON_PRESS_EVENT, 1, 0, 0);
        for x in [210, 230, 260] {
            fake(&conn, root, xproto::MOTION_NOTIFY_EVENT, 0, x, 200);
        }
        fake(&conn, root, xproto::BUTTON_RELEASE_EVENT, 1, 0, 0);
        key(&conn, root, alt, false);
        std::thread::sleep(Duration::from_millis(300));
        let got = emits.lock().unwrap().clone();
        assert!(
            got.iter().any(|e| matches!(
                e,
                Emit::WheelOpen {
                    wheel: WheelKind::Ping,
                    ..
                }
            )),
            "Alt 加拖动要开信号轮盘：{got:?}"
        );
        assert!(
            got.iter().any(|e| matches!(
                e,
                Emit::WheelRelease {
                    wheel: WheelKind::Ping,
                    ..
                }
            )),
            "松开要放信号：{got:?}"
        );

        emits.lock().unwrap().clear();
        let (ctrl, p) = (keycode(&conn, CONTROL_L), keycode(&conn, KEY_P));
        key(&conn, root, ctrl, true);
        key(&conn, root, alt, true);
        key(&conn, root, p, true);
        key(&conn, root, p, false);
        key(&conn, root, alt, false);
        key(&conn, root, ctrl, false);
        std::thread::sleep(Duration::from_millis(300));
        let got = emits.lock().unwrap().clone();
        assert!(
            got.contains(&Emit::Toggled { enabled: false }),
            "Ctrl+Alt+P 要暂停：{got:?}"
        );
    }
}
