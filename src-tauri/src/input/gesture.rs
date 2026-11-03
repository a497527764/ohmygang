//! 手势状态机（架构 2.2）：决定每个键鼠事件拦下还是放行，以及要不要开轮盘、回放什么。
//! 逐条移植自 lolPing `native/hook-helper/src/decision.cpp`（MIT，© 2026 Danrui Meng）。
//! M1 移植了信号和表情两个轮盘；打字键和 Tab 面板键在 M5 再移植，只给 AI 队友用的问询轮盘、
//! 双击修饰键、滚轮监视不移植。
//!
//! 实现流程：
//! 1. 纯逻辑，不调系统接口：平台层把钩子收到的事件换成 `InputEvent`（键用 Windows 虚拟键码，坐标是物理像素），
//!    调 `on_event`，按返回的 `Outcome` 拦下或放行，再把 `emits` 交给画面、把 `injects` 回放给系统。
//! 2. 状态：`Idle` → 按住触发键按下左键进 `Pending`（按下被拦下）→ 拖过阈值进 `Wheel`（开轮盘）→ 松开回 `Idle`。
//!    `SwallowUp` 是手势被取消后等着拦下配对的抬起：拦下的按下必须配对拦下抬起，或者两个都回放，绝不留下没配对的按下。
//! 3. 没拖动就松开：单击信号打开时放一个信号，否则把按下和抬起原样回放，下面的程序照常收到一次单击。
//! 4. 修饰键补丁：Alt 或 Win 按住期间拦下过东西，它们弹起前先注入一个无效键（0xFC），
//!    不然 Windows 会把单独的 Alt / Win 弹起当成打开菜单栏、开始菜单。
//! 5. 别的程序的组合键（Alt+Tab、Win+Tab、Alt+F4）期间单击原样放行，直到触发键全部松开。
//! 6. 自定义按键当触发键时先扣住：单独按一下就在松开时补打这个键；拖动用掉了就不补；按住时又按了别的键说明在打字，
//!    先把它补发出去。Caps Lock 当触发键时不补（补了会切换大小写）。
//! 7. 看门狗：手势 10 秒没有鼠标事件就取消；平台层每秒用系统的按键状态同步一次，忘掉抬起没收到的键。

/// 鼠标按钮。X1、X2 是侧键（常见的「后退」「前进」）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    None,
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvKind {
    MouseDown,
    MouseUp,
    MouseMove,
    KeyDown,
    KeyUp,
}

/// 平台层交来的一个事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputEvent {
    pub kind: EvKind,
    pub btn: Btn,
    /// Windows 虚拟键码，只对键盘事件有意义。
    pub vk: u32,
    /// 物理像素坐标。
    pub x: i32,
    pub y: i32,
    /// 自己回放的事件（带标记），直接放行，不进状态机。
    pub self_injected: bool,
    pub time_ms: u64,
}

impl InputEvent {
    pub fn key(kind: EvKind, vk: u32, time_ms: u64) -> Self {
        Self {
            kind,
            btn: Btn::None,
            vk,
            x: 0,
            y: 0,
            self_injected: false,
            time_ms,
        }
    }

    pub fn mouse(kind: EvKind, btn: Btn, x: i32, y: i32, time_ms: u64) -> Self {
        Self {
            kind,
            btn,
            vk: 0,
            x,
            y,
            self_injected: false,
            time_ms,
        }
    }
}

/// 轮盘的触发方式：按住某个键用左键拖，或者直接用某个侧键拖。`None` 表示这个轮盘关掉。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Alt,
    Ctrl,
    Shift,
    Win,
    CapsLock,
    Mouse4,
    Mouse5,
    /// 自定义按键（虚拟键码）。
    Custom(u32),
    None,
}

/// 手势属于哪个轮盘，各有各的触发键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelKind {
    Ping,
    Emote,
}

const WHEELS: [WheelKind; 2] = [WheelKind::Ping, WheelKind::Emote];

pub const MOD_CTRL: u32 = 1;
pub const MOD_ALT: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// 信号轮盘的触发键。
    pub trigger: Trigger,
    /// 触发键加单击（没拖动）放一个信号；关着时单击原样回放。
    pub click_ping: bool,
    /// 全屏程序（游戏）占着点击的那块屏幕时，触发键加点击原样放行。
    pub game_check: bool,
    /// 表情轮盘的触发键（M4 才用，默认关）。
    pub emote_trigger: Trigger,
    pub emote_click: bool,
    /// 按下后拖过这么多物理像素才开轮盘。
    pub drag_threshold_px: i32,
    /// 暂停快捷键：修饰键位（MOD_*）加一个虚拟键码。
    pub toggle_mods: u32,
    pub toggle_vk: u32,
    /// 对线模式快捷键；vk 为 0 表示关（还没收到配置时什么都不拦）。
    pub laning_mods: u32,
    pub laning_vk: u32,
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trigger: Trigger::Alt,
            click_ping: false,
            game_check: false,
            emote_trigger: Trigger::None,
            emote_click: false,
            drag_threshold_px: 8,
            toggle_mods: MOD_CTRL | MOD_ALT,
            toggle_vk: 0x50, // P
            laning_mods: MOD_CTRL | MOD_ALT,
            laning_vk: 0,
            enabled: true,
        }
    }
}

/// 交给画面和托盘的事。坐标是物理像素，`wheel` 是这次手势属于的轮盘。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emit {
    /// 轮盘在按下的位置打开。
    WheelOpen {
        x: i32,
        y: i32,
        wheel: WheelKind,
    },
    WheelMove {
        x: i32,
        y: i32,
        wheel: WheelKind,
    },
    /// 松开：画面按松开的位置选格。
    WheelRelease {
        x: i32,
        y: i32,
        wheel: WheelKind,
    },
    /// 触发键加单击（单击信号打开时），在按下的位置放。
    Click {
        x: i32,
        y: i32,
        wheel: WheelKind,
    },
    /// 手势取消（右键、Esc、暂停、看门狗、改配置）。
    Cancel,
    /// 暂停快捷键切换了开关。
    Toggled {
        enabled: bool,
    },
    /// 对线模式下有充能完成的对手时，信号触发键加单击是「普攻」，在按下的位置出手。
    Attack {
        x: i32,
        y: i32,
    },
    /// 按了对线模式快捷键。
    LaningToggled,
}

/// 要回放给系统的输入，平台层带上自己的标记发出去。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inject {
    ButtonDown(Btn),
    ButtonUp(Btn),
    KeyDown(u32),
    KeyUp(u32),
}

/// 一个事件的处理结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// 拦下这个事件，下面的程序收不到。
    pub swallow: bool,
    pub emits: Vec<Emit>,
    pub injects: Vec<Inject>,
}

/// 无效键 VK_NONAME：插在 Alt / Win 弹起之前，挡住菜单栏和开始菜单。
pub const MASK_VK: u32 = 0xFC;
/// 手势这么久没有鼠标事件就当作卡住了，取消掉。
pub const WATCHDOG_MS: u64 = 10_000;

const VK_SHIFT: u32 = 0x10;
const VK_CTRL: u32 = 0x11;
const VK_ALT: u32 = 0x12;
const VK_CAPS: u32 = 0x14;
const VK_ESC: u32 = 0x1B;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCTRL: u32 = 0xA2;
const VK_RCTRL: u32 = 0xA3;
const VK_LALT: u32 = 0xA4;
const VK_RALT: u32 = 0xA5;

fn mod_bit(vk: u32) -> u32 {
    match vk {
        VK_CTRL | VK_LCTRL | VK_RCTRL => MOD_CTRL,
        VK_ALT | VK_LALT | VK_RALT => MOD_ALT,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => MOD_SHIFT,
        VK_LWIN | VK_RWIN => MOD_WIN,
        _ => 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Pending,
    Wheel,
    SwallowUp,
}

/// 平台提供的检查：(x, y) 所在的屏幕是不是被全屏程序占着。
pub type FullscreenProbe = Box<dyn Fn(i32, i32) -> bool + Send>;

pub struct Gesture {
    cfg: Config,
    enabled: bool,
    suspended: bool,
    probe: Option<FullscreenProbe>,
    /// 对线模式里有充能完成的对手（由对线判定告诉这里）。
    attack_ready: bool,
    state: State,
    key_down: [bool; 256],
    /// 按下被拦下的键：它们的抬起也要拦下（系统从没见过按下）。
    swallow_key_ups: [bool; 256],
    swallow_right_up: bool,
    need_mask: bool,
    /// 触发键按住时按了别的普通键（Alt+Tab 之类）：这是别的程序的组合键，触发键全松开前单击都放行。
    foreign_key: bool,
    mask_menu_keys: bool,
    /// 自定义触发键按下了但还没发给系统；拖动用掉它，单独松开就补打。0 表示没有。
    held_back_vk: u32,
    /// 扣住的键因为按了别的键被补发了（在打字）：它的连发和真正的抬起从此放行。
    typed_vk: u32,
    owner: WheelKind,
    drag_btn: Btn,
    px: i32,
    py: i32,
    last_mouse_ms: u64,
}

impl Default for Gesture {
    fn default() -> Self {
        Self::new()
    }
}

impl Gesture {
    pub fn new() -> Self {
        Self {
            cfg: Config::default(),
            enabled: true,
            suspended: false,
            probe: None,
            attack_ready: false,
            state: State::Idle,
            key_down: [false; 256],
            swallow_key_ups: [false; 256],
            swallow_right_up: false,
            need_mask: false,
            foreign_key: false,
            mask_menu_keys: true,
            held_back_vk: 0,
            typed_vk: 0,
            owner: WheelKind::Ping,
            drag_btn: Btn::None,
            px: 0,
            py: 0,
            last_mouse_ms: 0,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// 全屏检查（只有 Windows 有）。没设置时 `game_check` 不起作用。
    pub fn set_fullscreen_probe(&mut self, probe: FullscreenProbe) {
        self.probe = Some(probe);
    }

    /// Windows 会把单独的 Alt / Win 弹起当成打开菜单；macOS 没有这回事，由平台层关掉。
    pub fn set_mask_menu_keys(&mut self, on: bool) {
        self.mask_menu_keys = on;
    }

    pub fn set_config(&mut self, c: Config) -> Outcome {
        let mut r = Outcome::default();
        self.cancel_gesture(&mut r);
        self.cfg = c;
        self.enabled = c.enabled;
        r
    }

    pub fn set_enabled(&mut self, on: bool) -> Outcome {
        let mut r = Outcome::default();
        if !on {
            self.cancel_gesture(&mut r);
        }
        self.enabled = on;
        r
    }

    /// 设置页在录按键时挂起：键都交给控制面板，暂停快捷键也不响应。
    /// 对线判定说现在有没有充能完成的对手。有的话信号触发键加单击是「普攻」，不放信号也不回放单击。
    pub fn set_attack_ready(&mut self, on: bool) {
        self.attack_ready = on;
    }

    pub fn set_suspended(&mut self, on: bool) -> Outcome {
        let mut r = Outcome::default();
        if on {
            self.cancel_gesture(&mut r);
        }
        self.suspended = on;
        r
    }

    /// 平台层每秒调一次。
    pub fn tick(&mut self, now_ms: u64) -> Outcome {
        let mut r = Outcome::default();
        let busy = matches!(self.state, State::Pending | State::Wheel);
        if busy && now_ms > self.last_mouse_ms && now_ms - self.last_mouse_ms > WATCHDOG_MS {
            r.emits.push(Emit::Cancel);
            // 按下被拦下了，抬起也要拦下。抬起真丢了的话，下一次按下拖动键会把 SwallowUp 复位。
            self.state = State::SwallowUp;
        }
        r
    }

    /// 和系统对一次按键状态：我们以为按着、系统说没按的键就忘掉（Win+L、Ctrl+Alt+Del、UAC 会吞掉抬起）。
    /// 从不把键标成按下；按下被我们拦下的键系统没见过，也留着。
    pub fn sync_key_states(&mut self, is_down: impl Fn(u32) -> bool) {
        for vk in 0..256u32 {
            let i = vk as usize;
            if self.key_down[i] && !self.swallow_key_ups[i] && !is_down(vk) {
                self.key_down[i] = false;
            }
        }
        if !self.any_held() {
            self.foreign_key = false;
        }
    }

    pub fn on_event(&mut self, e: &InputEvent) -> Outcome {
        let mut r = Outcome::default();
        if e.self_injected {
            return r;
        }
        match e.kind {
            EvKind::KeyDown => {
                // 自定义触发键扣着时来了别的键：说明在打字。先把扣着的键补发；这个键要是放行，
                // 就会抢在排队的补发前面，所以也拦下、排在后面回放，保持打字顺序。
                let flushed = self.flush_held_back(e.vk & 0xFF, &mut r);
                self.on_key_down(e, &mut r);
                if flushed && !r.swallow {
                    r.swallow = true;
                    r.injects.push(Inject::KeyDown(e.vk & 0xFF));
                }
            }
            EvKind::KeyUp => self.on_key_up(e, &mut r),
            EvKind::MouseDown => {
                self.last_mouse_ms = e.time_ms;
                self.on_mouse_down(e, &mut r);
            }
            EvKind::MouseUp => {
                self.last_mouse_ms = e.time_ms;
                self.on_mouse_up(e, &mut r);
            }
            EvKind::MouseMove => {
                self.last_mouse_ms = e.time_ms;
                self.on_mouse_move(e, &mut r);
            }
        }
        r
    }

    fn active(&self) -> bool {
        self.enabled && !self.suspended
    }

    fn mods(&self) -> u32 {
        (0..256u32)
            .filter(|&vk| self.key_down[vk as usize])
            .fold(0, |m, vk| m | mod_bit(vk))
    }

    fn spec(&self, k: WheelKind) -> (Trigger, bool) {
        match k {
            WheelKind::Ping => (self.cfg.trigger, self.cfg.click_ping),
            WheelKind::Emote => (self.cfg.emote_trigger, self.cfg.emote_click),
        }
    }

    fn trigger(&self, k: WheelKind) -> Trigger {
        self.spec(k).0
    }

    /// 没拖动就松开时要交出去的事：普攻、单击信号，或者没有（回放单击）。
    fn click_emit(&self) -> Option<Emit> {
        if self.owner == WheelKind::Ping && self.attack_ready {
            Some(Emit::Attack {
                x: self.px,
                y: self.py,
            })
        } else if self.click_on(self.owner) {
            Some(Emit::Click {
                x: self.px,
                y: self.py,
                wheel: self.owner,
            })
        } else {
            None
        }
    }

    fn click_on(&self, k: WheelKind) -> bool {
        self.spec(k).1
    }

    /// Caps Lock 或自定义键的键码；其他触发方式没有。
    fn key_trigger_vk(&self, k: WheelKind) -> Option<u32> {
        match self.trigger(k) {
            Trigger::CapsLock => Some(VK_CAPS),
            Trigger::Custom(vk) => Some(vk),
            _ => None,
        }
    }

    /// 只看键盘触发：侧键触发或关掉的轮盘永远不算「按着」。
    fn held(&self, k: WheelKind) -> bool {
        match self.trigger(k) {
            Trigger::Alt => self.mods() & MOD_ALT != 0,
            Trigger::Ctrl => self.mods() & MOD_CTRL != 0,
            Trigger::Shift => self.mods() & MOD_SHIFT != 0,
            Trigger::Win => self.mods() & MOD_WIN != 0,
            Trigger::CapsLock | Trigger::Custom(_) => self
                .key_trigger_vk(k)
                .is_some_and(|vk| self.key_down[(vk & 0xFF) as usize]),
            Trigger::Mouse4 | Trigger::Mouse5 | Trigger::None => false,
        }
    }

    fn any_held(&self) -> bool {
        WHEELS.iter().any(|&k| self.held(k))
    }

    fn is_trigger_vk(&self, k: WheelKind, vk: u32) -> bool {
        match self.trigger(k) {
            Trigger::Alt => mod_bit(vk) == MOD_ALT,
            Trigger::Ctrl => mod_bit(vk) == MOD_CTRL,
            Trigger::Shift => mod_bit(vk) == MOD_SHIFT,
            Trigger::Win => mod_bit(vk) == MOD_WIN,
            Trigger::CapsLock | Trigger::Custom(_) => self.key_trigger_vk(k) == Some(vk),
            Trigger::Mouse4 | Trigger::Mouse5 | Trigger::None => false,
        }
    }

    fn is_any_trigger_vk(&self, vk: u32) -> bool {
        WHEELS.iter().any(|&k| self.is_trigger_vk(k, vk))
    }

    fn is_mouse_trigger(&self, k: WheelKind) -> bool {
        matches!(self.trigger(k), Trigger::Mouse4 | Trigger::Mouse5)
    }

    fn drag_button(&self, k: WheelKind) -> Btn {
        match self.trigger(k) {
            Trigger::Mouse4 => Btn::X1,
            Trigger::Mouse5 => Btn::X2,
            _ => Btn::Left,
        }
    }

    /// 这次按下开始这个轮盘的手势：它的侧键，或者按着它的触发键按左键。
    fn starts_gesture(&self, k: WheelKind, btn: Btn) -> bool {
        btn == self.drag_button(k) && (self.is_mouse_trigger(k) || self.held(k))
    }

    /// Alt 或 Win 按着的时候拦下了东西：它们的弹起要打补丁，不然会弹出菜单。
    fn mark_swallow(&mut self) {
        if self.mask_menu_keys && self.mods() & (MOD_ALT | MOD_WIN) != 0 {
            self.need_mask = true;
        }
    }

    fn cancel_gesture(&mut self, r: &mut Outcome) {
        if matches!(self.state, State::Pending | State::Wheel) {
            r.emits.push(Emit::Cancel);
            self.state = State::SwallowUp;
        }
    }

    /// 来了别的键时把扣着的自定义触发键补发出去，并让这个键进入「打字」：之后它的连发和真正的抬起都交给系统。
    fn flush_held_back(&mut self, vk: u32, r: &mut Outcome) -> bool {
        if self.held_back_vk == 0 || vk == self.held_back_vk {
            return false;
        }
        let held = self.held_back_vk;
        self.held_back_vk = 0;
        self.typed_vk = held;
        self.swallow_key_ups[held as usize] = false; // 系统现在看到它按下了，就得让它看到抬起
        r.injects.push(Inject::KeyDown(held));
        true
    }

    fn on_key_down(&mut self, e: &InputEvent, r: &mut Outcome) {
        let vk = e.vk & 0xFF;
        let i = vk as usize;
        let repeat = self.key_down[i];
        self.key_down[i] = true;

        // 别的程序的组合键开始了（Alt+Tab 的切换、Win+Tab、Alt+F4）：触发键全松开之前，单击都归它。
        // 只在 Idle 判断：手势中途来的键（比如 Esc 取消轮盘）不代表下一次手势。
        if self.state == State::Idle
            && mod_bit(vk) == 0
            && !self.is_any_trigger_vk(vk)
            && self.any_held()
        {
            self.foreign_key = true;
        }

        if !self.suspended && vk == self.cfg.toggle_vk && self.mods() == self.cfg.toggle_mods {
            r.swallow = true;
            self.swallow_key_ups[i] = true;
            self.mark_swallow();
            if !repeat {
                if self.enabled {
                    self.cancel_gesture(r);
                }
                self.enabled = !self.enabled;
                r.emits.push(Emit::Toggled {
                    enabled: self.enabled,
                });
            }
            return;
        }
        if self.active()
            && self.cfg.laning_vk != 0
            && vk == self.cfg.laning_vk
            && self.mods() == self.cfg.laning_mods
        {
            r.swallow = true;
            self.swallow_key_ups[i] = true;
            self.mark_swallow();
            if !repeat {
                r.emits.push(Emit::LaningToggled);
            }
            return;
        }
        if self.active() {
            if vk == VK_ESC && matches!(self.state, State::Pending | State::Wheel) {
                r.swallow = true;
                self.swallow_key_ups[i] = true;
                self.mark_swallow();
                self.cancel_gesture(r);
                return;
            }
            // 补发过的触发键在打字，它的连发也照常打字。
            if vk == self.typed_vk {
                self.swallow_key_ups[i] = false;
                return;
            }
            // Caps Lock 和自定义键当触发键时会切换大小写或打出字：不给系统。
            let own_key = |k: WheelKind| self.key_trigger_vk(k) == Some(vk);
            if WHEELS.iter().any(|&k| own_key(k)) {
                r.swallow = true;
                self.swallow_key_ups[i] = true;
                // 自定义键扣住而不是丢掉，松开时没被拖动用掉就补打。Caps Lock 不补，补了会切换大小写。
                let custom = |k: WheelKind| self.trigger(k) == Trigger::Custom(vk);
                if !repeat && WHEELS.iter().any(|&k| custom(k)) {
                    self.held_back_vk = vk;
                }
                return;
            }
        }
        // 这次按下放行了，这个键之前等着拦的抬起说明丢了：清掉。
        self.swallow_key_ups[i] = false;
    }

    fn on_key_up(&mut self, e: &InputEvent, r: &mut Outcome) {
        let vk = e.vk & 0xFF;
        let i = vk as usize;
        // 只有手势自己的触发键能结束它：另一个轮盘的键随便按。
        let owner_held_before = self.held(self.owner);
        self.key_down[i] = false;
        if !self.any_held() {
            self.foreign_key = false;
        }
        let trigger_released =
            owner_held_before && !self.held(self.owner) && self.is_trigger_vk(self.owner, vk);

        if self.swallow_key_ups[i] {
            self.swallow_key_ups[i] = false;
            r.swallow = true;
        }
        // 补发过的键，它真正的抬起放行（拦截在补发时已经撤掉），打字结束。
        if vk == self.typed_vk {
            self.typed_vk = 0;
        }
        // 扣着的键按下又松开，中间什么都没发生：现在补打（按下加抬起）。真正的抬起照样拦下，系统没见过真正的按下。
        if vk == self.held_back_vk {
            self.held_back_vk = 0;
            r.swallow = true;
            r.injects.push(Inject::KeyDown(vk));
            r.injects.push(Inject::KeyUp(vk));
        }
        if self.need_mask && matches!(mod_bit(vk), MOD_ALT | MOD_WIN) {
            // 拦下真正的弹起，按顺序回放：无效键按下、抬起，再弹起这个键。
            self.need_mask = false;
            r.swallow = true;
            r.injects.push(Inject::KeyDown(MASK_VK));
            r.injects.push(Inject::KeyUp(MASK_VK));
            r.injects.push(Inject::KeyUp(vk));
        }
        if trigger_released && self.state == State::Pending {
            if let Some(emit) = self.click_emit() {
                r.emits.push(emit);
                self.state = State::SwallowUp;
            } else {
                // 普通单击：现在补上按下，真正的抬起放行。
                r.injects.push(Inject::ButtonDown(self.drag_btn));
                self.state = State::Idle;
            }
        }
        // 轮盘开着时不管触发键，和英雄联盟一样由松开鼠标选格。触控板三指拖动的抬起要晚一秒左右才到。
    }

    fn on_mouse_down(&mut self, e: &InputEvent, r: &mut Outcome) {
        if self.state == State::SwallowUp && e.btn == self.drag_btn {
            self.state = State::Idle; // 配对的抬起丢了
        }
        // 又来了右键按下，说明上一次右键抬起没到（安全桌面、Win+L、UAC）：这次的抬起不拦。
        if e.btn == Btn::Right {
            self.swallow_right_up = false;
        }
        if !self.active() {
            return;
        }
        if matches!(self.state, State::Pending | State::Wheel) && e.btn == Btn::Right {
            r.swallow = true;
            self.swallow_right_up = true;
            self.mark_swallow();
            self.cancel_gesture(r);
            return;
        }
        if self.state != State::Idle || self.foreign_key {
            return;
        }
        // 必须正好一个轮盘认领这次按下。两个都认领（Alt 和 Ctrl 都按着）就有歧义，原样放行，不去猜。
        let mut claims = WHEELS.iter().filter(|&&k| self.starts_gesture(k, e.btn));
        let (Some(&claimed), None) = (claims.next(), claims.next()) else {
            return;
        };
        // 全屏程序（游戏）占着这块屏幕：点击归它。只在真要开始手势时才查，普通点击不付这个代价。
        if self.cfg.game_check && self.probe.as_ref().is_some_and(|p| p(e.x, e.y)) {
            return;
        }
        r.swallow = true;
        self.mark_swallow();
        self.held_back_vk = 0; // 扣着的键开始了手势，不再补打
        self.state = State::Pending;
        self.owner = claimed;
        self.drag_btn = e.btn;
        self.px = e.x;
        self.py = e.y;
    }

    fn on_mouse_up(&mut self, e: &InputEvent, r: &mut Outcome) {
        if e.btn == Btn::Right && self.swallow_right_up {
            self.swallow_right_up = false;
            r.swallow = true;
            return;
        }
        if self.state == State::Idle || e.btn != self.drag_btn {
            return;
        }
        r.swallow = true;
        match self.state {
            State::Pending if self.click_emit().is_some() => {
                r.emits.extend(self.click_emit());
            }
            State::Pending => {
                r.injects.push(Inject::ButtonDown(self.drag_btn));
                r.injects.push(Inject::ButtonUp(self.drag_btn));
            }
            State::Wheel => {
                r.emits.push(Emit::WheelRelease {
                    x: e.x,
                    y: e.y,
                    wheel: self.owner,
                });
            }
            State::Idle | State::SwallowUp => {}
        }
        self.state = State::Idle;
    }

    fn on_mouse_move(&mut self, e: &InputEvent, r: &mut Outcome) {
        match self.state {
            State::Pending => {
                let dist = f64::from(e.x - self.px).hypot(f64::from(e.y - self.py));
                if dist > f64::from(self.cfg.drag_threshold_px) {
                    self.state = State::Wheel;
                    r.emits.push(Emit::WheelOpen {
                        x: self.px,
                        y: self.py,
                        wheel: self.owner,
                    });
                    r.emits.push(Emit::WheelMove {
                        x: e.x,
                        y: e.y,
                        wheel: self.owner,
                    });
                }
            }
            State::Wheel => r.emits.push(Emit::WheelMove {
                x: e.x,
                y: e.y,
                wheel: self.owner,
            }),
            State::Idle | State::SwallowUp => {}
        }
    }
}

#[cfg(test)]
mod tests;
