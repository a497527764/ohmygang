//! 对线模式的运行时（2.0 架构第 6 节）：把对线判定（`laning`，纯函数）接到鼠标、节点、透明窗口和手势上。
//!
//! 实现流程：
//! 1. 一个专门的线程，开着对线模式时每 33 毫秒走一步，关着时只等消息。别处只往它的通道里发消息。
//! 2. 每一步：读本人鼠标（钩子记下的物理像素）→ 换成「第几号显示器 + 比例」，动了就发 Cursor 帧
//!    （没动也每 250 毫秒发一次，免得对手当我掉线）；交给 `Laning` 走一步，开始和中断的充能发 Charge 帧；
//!    「有没有充能完成的对手」变了就告诉手势状态机（有的话触发键加单击是普攻）；对手的位置和状态交给透明窗口画。
//! 3. 出手（手势交来的 Attack）：按落点判定，打中的每个人在节点任务里写一条 Hit 事件广播出去，
//!    本机马上在对手身上放心之钢特效；记下冷却（30 秒，从重放里取的为准）。
//! 4. 队友的 Hit 被重放判为有效（`CoreEvent::HitLanded`）时，在目标身上放特效：目标是我就放在我的鼠标处。
//! 5. 关掉对线模式：所有充能中断、不再发鼠标、透明窗口清空、手势不再普攻；在线状态里的「对线」也跟着变。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::api::AppState;
use crate::crypto::MemberId;
use crate::heartsteel::COOLDOWN_MS;
use crate::input::Command;
use crate::laning::{LaneEvent, Laning, MyPos, Phase};
use crate::node::now_ms;
use crate::overlay::displays::{self, Display};
use crate::overlay::{LaneOpponent, Msg};
use crate::protocol::live::{LiveBody, to_ratio};

const STEP: Duration = Duration::from_millis(33);
/// 鼠标没动时多久也发一次位置：对手 1 秒收不到就当我掉线，留足余量。
const KEEPALIVE: Duration = Duration::from_millis(250);
/// 多久重新查一次显示器。
const DISPLAYS_EVERY: Duration = Duration::from_secs(2);

pub enum LaneMsg {
    /// 开关对线模式（快捷键、托盘、控制面板）。None 表示切换。
    Set(Option<bool>),
    Cursor {
        from: MemberId,
        display: u8,
        x: u16,
        y: u16,
        via_relay: bool,
    },
    /// 有人对我开始或中断充能。
    Charge {
        from: MemberId,
        target: MemberId,
        on: bool,
    },
    /// 对手关了对线模式或离开队伍。
    Gone(MemberId),
    /// 手势交来的普攻，物理像素。
    Attack { x: i32, y: i32 },
    /// 一条新的有效命中（自己的或队友的）。
    Hit { author: MemberId, target: MemberId },
    /// 队员名字（画在对手身上）。
    Names(Vec<(MemberId, String)>),
}

#[derive(Clone)]
pub struct Lane {
    tx: Sender<LaneMsg>,
    on: Arc<AtomicBool>,
}

impl Lane {
    pub fn start(app: &AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();
        let on = Arc::new(AtomicBool::new(false));
        let runner = Runner {
            app: app.clone(),
            on: on.clone(),
            laning: Laning::default(),
            displays: Vec::new(),
            displays_at: None,
            names: HashMap::new(),
            cooldowns: HashMap::new(),
            charging_me: HashMap::new(),
            last_sent: None,
            attack_ready: false,
            me: None,
        };
        std::thread::Builder::new()
            .name("laning".into())
            .spawn(move || runner.run(rx))
            .expect("能开线程");
        Self { tx, on }
    }

    pub fn send(&self, m: LaneMsg) {
        let _ = self.tx.send(m);
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }
}

struct Runner {
    app: AppHandle,
    on: Arc<AtomicBool>,
    laning: Laning,
    /// 按编号排好的显示器。
    displays: Vec<Display>,
    displays_at: Option<Instant>,
    names: HashMap<MemberId, String>,
    /// 我对每个对手的冷却结束时间（Unix 毫秒）。
    cooldowns: HashMap<MemberId, u64>,
    /// 正在对我充能的人，和他开始的时间。
    charging_me: HashMap<MemberId, u64>,
    /// 上一次发出去的鼠标帧和时间。
    last_sent: Option<((u8, u16, u16), Instant)>,
    attack_ready: bool,
    me: Option<MemberId>,
}

impl Runner {
    fn run(mut self, rx: Receiver<LaneMsg>) {
        loop {
            let msg = if self.on.load(Ordering::Relaxed) {
                match rx.recv_timeout(STEP) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => return,
                }
            };
            if let Some(m) = msg {
                self.handle(m);
            }
            if self.on.load(Ordering::Relaxed) {
                self.step();
            }
        }
    }

    fn state(&self) -> Option<tauri::State<'_, AppState>> {
        self.app.try_state::<AppState>()
    }

    fn handle(&mut self, m: LaneMsg) {
        let now = now_ms();
        match m {
            LaneMsg::Set(want) => {
                let on = want.unwrap_or(!self.on.load(Ordering::Relaxed));
                if on == self.on.load(Ordering::Relaxed) {
                    return;
                }
                self.on.store(on, Ordering::Relaxed);
                if on {
                    self.load_cooldowns();
                } else {
                    let events = self.laning.clear();
                    self.send_charges(&events);
                    self.charging_me.clear();
                    self.last_sent = None;
                    self.set_attack_ready(false);
                    self.draw();
                }
                crate::team::after_laning(&self.app, on);
            }
            LaneMsg::Cursor {
                from,
                display,
                x,
                y,
                via_relay,
            } => {
                if self.on.load(Ordering::Relaxed) {
                    self.laning.on_cursor(from, display, x, y, via_relay, now);
                }
            }
            LaneMsg::Charge { from, target, on } => {
                if Some(target) != self.me() {
                    return;
                }
                if on {
                    self.charging_me.insert(from, now);
                } else {
                    self.charging_me.remove(&from);
                }
            }
            LaneMsg::Gone(from) => {
                let events: Vec<LaneEvent> = self.laning.remove(&from).into_iter().collect();
                self.send_charges(&events);
                self.charging_me.remove(&from);
            }
            LaneMsg::Attack { x, y } => self.attack(x, y, now),
            LaneMsg::Hit { author, target } => self.hit_fx(author, target),
            LaneMsg::Names(list) => self.names = list.into_iter().collect(),
        }
    }

    fn me(&mut self) -> Option<MemberId> {
        if self.me.is_none() {
            let node = self.state()?.node.clone();
            self.me =
                tauri::async_runtime::block_on(node.call(|core, _| core.identity().member_id));
        }
        self.me
    }

    /// 打开对线模式时，从重放里取我对每个人的冷却。
    fn load_cooldowns(&mut self) {
        let Some(node) = self.state().map(|s| s.node.clone()) else {
            return;
        };
        let last = tauri::async_runtime::block_on(node.call(|core, _| {
            let room = core.current_team().ok().flatten()?.room_id;
            core.my_last_hits(&room).ok()
        }));
        self.cooldowns = last
            .flatten()
            .unwrap_or_default()
            .into_iter()
            .map(|(m, t)| (m, t + COOLDOWN_MS))
            .collect();
    }

    fn refresh_displays(&mut self) {
        if self
            .displays_at
            .is_some_and(|t| t.elapsed() < DISPLAYS_EVERY)
        {
            return;
        }
        if let Some(list) = crate::overlay::current_displays(&self.app) {
            self.displays = list;
        }
        self.displays_at = Some(Instant::now());
    }

    /// 物理像素的点 → 第几号显示器上的 `MyPos`。
    fn locate(&self, x: i32, y: i32) -> Option<MyPos> {
        let i = displays::display_at(&self.displays, x, y)?;
        let d = &self.displays[i];
        Some(MyPos {
            display: u8::try_from(i + 1).unwrap_or(u8::MAX),
            x: f64::from(x - d.rect.x),
            y: f64::from(y - d.rect.y),
            width: f64::from(d.rect.width),
            height: f64::from(d.rect.height),
        })
    }

    fn step(&mut self) {
        let now = now_ms();
        self.refresh_displays();
        let me = crate::input::cursor().and_then(|(x, y)| self.locate(x, y));
        if let Some(p) = me {
            let frame = (p.display, to_ratio(p.x, p.width), to_ratio(p.y, p.height));
            let due = match self.last_sent {
                Some((last, at)) => last != frame || at.elapsed() >= KEEPALIVE,
                None => true,
            };
            if due {
                self.last_sent = Some((frame, Instant::now()));
                if let Some(s) = self.state() {
                    s.node.send_live(LiveBody::Cursor {
                        display: frame.0,
                        x: frame.1,
                        y: frame.2,
                    });
                }
            }
        }
        let cooldowns = self.cooldowns.clone();
        let events = self.laning.step(me, now, |m| cooldowns.get(m).copied());
        self.send_charges(&events);
        // 充能提示 4 秒没更新（中断帧丢了）就不再显示
        self.charging_me
            .retain(|_, since| now.saturating_sub(*since) < 4000);
        let ready = self.laning.any_charged();
        self.set_attack_ready(ready);
        self.draw();
    }

    fn send_charges(&self, events: &[LaneEvent]) {
        let Some(s) = self.state() else { return };
        for e in events {
            let (target, on) = match *e {
                LaneEvent::ChargeStarted(m) => (m, true),
                LaneEvent::ChargeStopped(m) => (m, false),
            };
            s.node.send_live(LiveBody::Charge { target, on });
        }
    }

    fn set_attack_ready(&mut self, ready: bool) {
        if ready == self.attack_ready {
            return;
        }
        self.attack_ready = ready;
        if let Some(s) = self.state()
            && let Some(input) = &s.input
        {
            input.send(Command::AttackReady(ready));
        }
    }

    /// 把对手和「谁在对我充能」交给透明窗口。
    fn draw(&self) {
        let Some(s) = self.state() else { return };
        let now = now_ms();
        let opponents = self
            .laning
            .view()
            .into_iter()
            .map(|o| {
                let (phase, progress, cooldown_s) = match o.phase {
                    Phase::Far => ("far", 0.0, 0),
                    Phase::Charging { since } => (
                        "charging",
                        (now.saturating_sub(since) as f64
                            / f64::from(crate::heartsteel::CHARGE_MS))
                        .min(1.0),
                        0,
                    ),
                    Phase::Charged { .. } => ("charged", 1.0, 0),
                    Phase::Cooldown { until } => {
                        ("cooldown", 0.0, until.saturating_sub(now).div_ceil(1000))
                    }
                };
                LaneOpponent {
                    member: o.member,
                    name: self
                        .names
                        .get(&o.member)
                        .cloned()
                        .unwrap_or_else(|| "队友".into()),
                    display: o.display,
                    x: o.x,
                    y: o.y,
                    phase,
                    progress,
                    cooldown_s,
                }
            })
            .collect();
        let charging_me = self
            .charging_me
            .keys()
            .map(|m| self.names.get(m).cloned().unwrap_or_else(|| "队友".into()))
            .collect();
        s.overlays.send(Msg::Lane {
            opponents,
            charging_me,
        });
    }

    fn attack(&mut self, x: i32, y: i32, now: u64) {
        if !self.on.load(Ordering::Relaxed) {
            return;
        }
        self.refresh_displays();
        let Some(at) = self.locate(x, y) else { return };
        let (hits, events) = self.laning.attack(at, now);
        self.send_charges(&events);
        let Some((overlays, node)) = self.state().map(|s| (s.overlays.clone(), s.node.clone()))
        else {
            return;
        };
        for h in &hits {
            self.cooldowns.insert(h.target, now + COOLDOWN_MS);
            if let Some((display, rx, ry)) = self.laning.position_of(&h.target) {
                overlays.send(Msg::LaneHit {
                    display,
                    x: rx,
                    y: ry,
                    label: "心之钢 +1".into(),
                });
            }
            let node = node.clone();
            let (target, charged_ms) = (h.target, h.charged_ms);
            tauri::async_runtime::spawn(async move {
                node.call(move |core, out| {
                    let room = core.current_team().ok().flatten()?.room_id;
                    let (events, _) = core.hit(&room, &target, charged_ms, now_ms()).ok()?;
                    out.broadcast(room, events);
                    Some(())
                })
                .await;
            });
        }
        self.set_attack_ready(self.laning.any_charged());
    }

    /// 队友的有效命中：在目标身上放特效。目标是我就放在我的鼠标处；我自己打中的已经在出手时放过了。
    fn hit_fx(&mut self, author: MemberId, target: MemberId) {
        let me = self.me();
        if Some(author) == me {
            return;
        }
        let Some(overlays) = self.state().map(|s| s.overlays.clone()) else {
            return;
        };
        let by = self
            .names
            .get(&author)
            .cloned()
            .unwrap_or_else(|| "队友".into());
        if Some(target) == me {
            self.refresh_displays();
            if let Some(p) = crate::input::cursor().and_then(|(x, y)| self.locate(x, y)) {
                overlays.send(Msg::LaneHit {
                    display: p.display,
                    x: to_ratio(p.x, p.width),
                    y: to_ratio(p.y, p.height),
                    label: format!("被{by}叠了一层"),
                });
            }
        } else if let Some((display, x, y)) = self.laning.position_of(&target) {
            overlays.send(Msg::LaneHit {
                display,
                x,
                y,
                label: format!("{by} 叠了一层"),
            });
        }
    }
}
