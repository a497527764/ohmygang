//! 节点（架构第 7 节的 M1 部分）：独占 Core，把界面命令、局域网和中继收包、定时器串在一个后台任务里。
//!
//! 实现流程：
//! 1. 命令：界面通过 `NodeHandle::call` 发来一个闭包，在节点任务里拿到 `&mut Core` 执行；
//!    闭包把新写的事件放进 `Outbox`，节点随后用 `Events { live: true }` 广播出去；改了本机设置时按新设置重建中继连接池。
//!    `NodeHandle::inspect` 读取节点自己的状态（成员路径、中继状态）。
//! 2. 两条路径（架构 6.5）：局域网 UDP 始终广播；按设置启动中继连接池，关闭「允许互联网」时没有连接池。
//!    局域网端口被占（多半是别的软件）时不退出，退回随机端口：局域网只能发不能收，中继照常，界面据此提示。
//!    本人的事件在有成员只能经中继联系、或局域网上没人在线时，再发一份到中继。
//! 3. 收包：两条路径的包走同一套处理，只是记下来源。按包头 room_id 找到房间密钥解密。Events 交给 Core 入库；
//!    Have 记下同伴（局域网地址、向量、每条路径的出现时间和周期），再做抑制、补齐和激活检查；
//!    Want 把对方要的事件放进回应队列，从哪条路径来就从哪条路径回：局域网每秒 50 包，中继每秒 3 包。
//! 4. 定时器：当前打开的房间局域网 Have 每 5 秒、中继每 30 秒，其余房间降频到 30 秒、120 秒（都有 ±20% 抖动）；
//!    每 5 秒看一次网卡有没有变，变了就当作网络恢复：所有房间 0–2 秒内发 Have，中继立刻重连；延迟 Have（收到实时事件后 1–5 秒、
//!    发现新同伴后 0–2 秒，期间被别人的 Have 覆盖就取消），需要时也发一份到中继；Want 超时换人；
//!    同伴在两条路径上都超过 3 个周期没消息视为离线；Joining 房间检查能否激活。
//! 5. 送达（架构 7.1）：本人的新事件广播后排进重发计划，没被确认就在 6、15、30 秒各重发一次；
//!    同伴的 Have 覆盖到的本人事件标记为已发送并通知界面。启动时把仍未确认的本人事件重新排进计划。
//! 6. 已解散的房间不主动发 Have；同伴的 Have 显示它还不知道解散时，回一次解散事件（同一同伴 1 分钟内最多一次）。
//! 7. 通知通过 sink 交出去：测试里收集起来检查，界面层转发成 Tauri 事件。
//!
//! 注意：Core 里的 SQLite 连接不能被多线程共享，所以等待网络发送时不能借用着 Core；
//! 发包的函数只拿传输层和密钥（`send_frame`），不借用整个节点。

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot};
use tokio::time::MissedTickBehavior;

use crate::app_core::{Core, CoreEvent, RoomStatus};
use crate::crypto::{EventId, MemberId, RoomId, RoomKey};
use crate::protocol::event::{SignedEvent, kind};
use crate::protocol::frame::{
    FloorEntry, Frame, HaveEntry, ShortId, WantRange, WireEvent, short_id,
};
use crate::protocol::packet::{PacketError, open_packet, peek_room_id, seal_packets};
use crate::sync::path::{self, PathKind, PeerPaths, Via};
use crate::sync::plan::{
    MAX_WANT_EVENTS, chat_gaps, dominates, missing_total, ready_to_activate, state_gaps,
};
use crate::sync::resend::ResendQueue;
use crate::transport::budget::Priority;
use crate::transport::lan::{Inbox, LanConfig, LanTransport};
use crate::transport::relay::{
    RelayConfig, RelayHandle, RelayHealth, RelayInbox, RelayPacket, RelayPool,
};

/// 节点交给界面的通知。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeEvent {
    Core(CoreEvent),
    RoomStatusChanged {
        room_id: RoomId,
        status: RoomStatus,
    },
    ProtocolMismatch {
        room_id: RoomId,
        remote_proto: u8,
    },
    /// 本人这些事件已被同伴确认持有（界面把「发送中」去掉）。
    Delivered {
        room_id: RoomId,
        event_ids: Vec<EventId>,
    },
}

pub type Sink = Arc<dyn Fn(NodeEvent) + Send + Sync>;

/// 局域网端口情况。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanStatus {
    pub port: u16,
    /// 配置的端口被占，用的是随机端口，收不到别人的局域网广播。
    pub degraded: bool,
}

#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub lan: LanConfig,
    /// 当前打开的房间的 Have 周期：局域网、中继。
    pub have_period: Duration,
    pub relay_have_period: Duration,
    /// 其余房间（后台）的 Have 周期：局域网、中继。
    pub background_have_period: Duration,
    pub background_relay_have_period: Duration,
    pub live_have_delay: (Duration, Duration),
    pub new_peer_have_delay: (Duration, Duration),
    pub want_timeout: Duration,
    pub want_max_tries: u32,
    /// 经局域网回应 Want 的发送速率（包/秒）。
    pub backfill_per_sec: u32,
    /// 经中继回应 Want 的发送速率（包/秒）。
    pub relay_backfill_per_sec: u32,
    pub tick: Duration,
    pub relay: RelayConfig,
    /// 设置后不用本机设置里的中继列表（测试用）；空列表表示不连中继。
    pub relay_urls: Option<Vec<String>>,
    /// 未确认的本人事件在首次发送后多久重发（架构 7.1：6、15、30 秒）。
    pub resend_after: [Duration; 3],
}

impl Default for NodeConfig {
    fn default() -> Self {
        NodeConfig {
            lan: LanConfig::default(),
            have_period: path::LAN_HAVE_PERIOD,
            relay_have_period: path::RELAY_HAVE_PERIOD,
            background_have_period: Duration::from_secs(30),
            background_relay_have_period: Duration::from_secs(120),
            live_have_delay: (Duration::from_secs(1), Duration::from_secs(5)),
            new_peer_have_delay: (Duration::ZERO, Duration::from_secs(2)),
            want_timeout: Duration::from_secs(5),
            want_max_tries: 3,
            backfill_per_sec: 50,
            relay_backfill_per_sec: 3,
            tick: Duration::from_millis(100),
            relay: RelayConfig::default(),
            relay_urls: None,
            resend_after: [
                Duration::from_secs(6),
                Duration::from_secs(15),
                Duration::from_secs(30),
            ],
        }
    }
}

/// 命令执行过程中要发出去的事件和要交给界面的通知。
#[derive(Default)]
pub struct Outbox {
    broadcasts: Vec<(RoomId, Vec<SignedEvent>)>,
    pub notes: Vec<NodeEvent>,
    /// 改了本机设置，节点要按新设置重建中继连接池。
    pub settings_changed: bool,
}

impl Outbox {
    pub fn broadcast(&mut self, room: RoomId, events: Vec<SignedEvent>) {
        if !events.is_empty() {
            self.broadcasts.push((room, events));
        }
    }
}

type Job = Box<dyn FnOnce(&mut Node) -> Outbox + Send>;

#[derive(Clone)]
pub struct NodeHandle {
    jobs: mpsc::Sender<Job>,
}

impl NodeHandle {
    /// 在节点任务里执行 `f` 并取回结果。节点已停止时返回 None。
    pub async fn call<R, F>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut Core, &mut Outbox) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move |node| {
            let mut out = Outbox::default();
            let _ = tx.send(f(&mut node.core, &mut out));
            out
        });
        self.jobs.send(job).await.ok()?;
        rx.await.ok()
    }

    /// 界面当前打开的房间（None 表示没打开任何房间）。这个房间用活跃周期发 Have，其余降频。
    pub async fn set_active_room(&self, room: Option<RoomId>) {
        let job: Job = Box::new(move |node| {
            node.set_active_room(room);
            Outbox::default()
        });
        let _ = self.jobs.send(job).await;
    }

    /// 在节点任务里读取节点自己的状态（成员路径、中继状态）。节点已停止时返回 None。
    pub async fn inspect<R, F>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&Node) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move |node| {
            let _ = tx.send(f(node));
            Outbox::default()
        });
        self.jobs.send(job).await.ok()?;
        rx.await.ok()
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn random_between(lo: Duration, hi: Duration) -> Duration {
    if hi <= lo {
        return lo;
    }
    let span = (hi - lo).as_millis() as u64;
    lo + Duration::from_millis(getrandom::u64().unwrap_or(0) % (span + 1))
}

/// 周期 ±20% 的随机抖动，避免所有人同一时刻发 Have。
fn jitter(base: Duration) -> Duration {
    random_between(base.mul_f64(0.8), base.mul_f64(1.2))
}

/// 包发往哪里。
#[derive(Clone, Copy, Debug)]
enum Route {
    /// 局域网广播；`relay` 不为 None 时再以这个优先级发一份到中继。
    Broadcast { relay: Option<Priority> },
    /// 局域网单播。
    Lan(SocketAddr),
    /// 只发到中继。
    Relay(Priority),
}

/// 包从哪里来。
#[derive(Clone, Copy, Debug)]
enum Source {
    Lan(SocketAddr),
    Relay,
}

impl Source {
    fn via(self) -> Via {
        match self {
            Source::Lan(_) => Via::Lan,
            Source::Relay => Via::Relay,
        }
    }

    /// 回应走原路。
    fn reply(self, priority: Priority) -> Route {
        match self {
            Source::Lan(addr) => Route::Lan(addr),
            Source::Relay => Route::Relay(priority),
        }
    }
}

async fn send_frame(
    lan: &LanTransport,
    relay: Option<&RelayHandle>,
    key: &RoomKey,
    room: &RoomId,
    frame: &Frame,
    route: Route,
) {
    let Ok(packets) = seal_packets(key, room, frame) else {
        return;
    };
    for p in packets {
        match route {
            Route::Broadcast { relay: priority } => {
                lan.broadcast(&p).await;
                if let (Some(r), Some(priority)) = (relay, priority) {
                    r.publish(*room, p, priority);
                }
            }
            Route::Lan(addr) => lan.send_to(addr, &p).await,
            Route::Relay(priority) => {
                if let Some(r) = relay {
                    r.publish(*room, p, priority);
                }
            }
        }
    }
}

async fn recv_relay(inbox: &mut Option<RelayInbox>) -> Option<RelayPacket> {
    match inbox {
        Some(i) => i.recv().await,
        None => std::future::pending().await,
    }
}

struct Peer {
    /// 最近一次经局域网联系时的地址。
    lan_addr: Option<SocketAddr>,
    paths: PeerPaths,
    vector: Vec<HaveEntry>,
    last_seen: Instant,
}

struct InFlightWant {
    peer: MemberId,
    sent_at: Instant,
    missing_before: u64,
}

struct RoomSync {
    key: RoomKey,
    status: RoomStatus,
    peers: HashMap<MemberId, Peer>,
    next_have: Instant,
    next_relay_have: Instant,
    pending_have: Option<Instant>,
    want: Option<InFlightWant>,
    failures: HashMap<MemberId, u32>,
    told_dissolve: HashMap<MemberId, Instant>,
    floors: FloorAsk,
}

/// 新成员问「最近 n 条聊天从哪开始」的进度（架构 7.4）。已经是成员的房间一开始就是 Done，聊天从 1 开始补。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FloorAsk {
    Needed,
    Asked { at: Instant },
    Done,
}

pub struct Node {
    core: Core,
    lan: LanTransport,
    lan_degraded: bool,
    relay: Option<RelayHandle>,
    relay_in: Option<RelayInbox>,
    cfg: NodeConfig,
    sink: Sink,
    me: MemberId,
    rooms: HashMap<RoomId, RoomSync>,
    backfill: VecDeque<(SocketAddr, Vec<u8>)>,
    relay_backfill: VecDeque<(RoomId, Vec<u8>)>,
    /// 下一个中继补齐包最早什么时候发。
    relay_backfill_at: Instant,
    resend: ResendQueue<SignedEvent>,
    active_room: Option<RoomId>,
    net_signature: Vec<std::net::IpAddr>,
    next_net_check: Instant,
}

/// 多久看一次网卡有没有变。
const NET_CHECK: Duration = Duration::from_secs(5);

/// 发给某位同伴的路径：在局域网上就单播，否则经中继（帧里带目标，其他人收到会忽略）。
fn route_to(p: &Peer, now: Instant) -> Route {
    match (p.paths.kind(now), p.lan_addr) {
        (PathKind::Lan, Some(addr)) => Route::Lan(addr),
        _ => Route::Relay(Priority::Control),
    }
}

impl Node {
    pub async fn spawn(core: Core, cfg: NodeConfig, sink: Sink) -> std::io::Result<NodeHandle> {
        let (lan, inbox, lan_degraded) = match LanTransport::bind(&cfg.lan).await {
            Ok((lan, inbox)) => (lan, inbox, false),
            Err(_) => {
                let mut fallback = cfg.lan.clone();
                fallback.bind.set_port(0);
                let (lan, inbox) = LanTransport::bind(&fallback).await?;
                (lan, inbox, true)
            }
        };
        let (tx, rx) = mpsc::channel::<Job>(64);
        let me = core.identity().member_id;
        let cfg_resend = cfg.resend_after;
        let mut node = Node {
            core,
            lan,
            lan_degraded,
            relay: None,
            relay_in: None,
            cfg,
            sink,
            me,
            rooms: HashMap::new(),
            backfill: VecDeque::new(),
            relay_backfill: VecDeque::new(),
            relay_backfill_at: Instant::now(),
            resend: ResendQueue::new(cfg_resend),
            active_room: None,
            net_signature: crate::transport::lan::interface_signature(),
            next_net_check: Instant::now() + NET_CHECK,
        };
        node.refresh_rooms();
        node.apply_settings();
        node.queue_pending();
        tokio::spawn(node.run(rx, inbox));
        Ok(NodeHandle { jobs: tx })
    }

    async fn run(mut self, mut jobs: mpsc::Receiver<Job>, mut inbox: Inbox) {
        let mut tick = tokio::time::interval(self.cfg.tick);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                job = jobs.recv() => {
                    let Some(job) = job else { break };
                    let out = job(&mut self);
                    // 导入备份可能换了身份：原来记的同伴状态和重发计划都属于旧身份，全部丢掉重新同步
                    let me = self.core.identity().member_id;
                    if me != self.me {
                        self.me = me;
                        self.rooms.clear();
                        self.resend = ResendQueue::new(self.cfg.resend_after);
                    }
                    // 先只加新房间、不移走已退出的：退出时写的 Leave 还要发出去
                    self.sync_rooms(false);
                    if out.settings_changed {
                        self.apply_settings();
                    }
                    for (room, events) in out.broadcasts {
                        self.send_events(room, events).await;
                    }
                    self.refresh_rooms();
                    for n in out.notes {
                        (self.sink)(n);
                    }
                }
                pkt = inbox.recv() => {
                    let Some((from, bytes)) = pkt else { break };
                    self.on_packet(Source::Lan(from), &bytes).await;
                }
                pkt = recv_relay(&mut self.relay_in) => match pkt {
                    Some(p) => self.on_packet(Source::Relay, &p.packet).await,
                    None => self.relay_in = None,
                },
                _ = tick.tick() => self.on_tick().await,
            }
        }
    }

    /// 和 Core 的房间列表对齐：新房间取密钥，已有的更新状态，已退出的移除；房间有增减时更新中继订阅。
    fn refresh_rooms(&mut self) {
        self.sync_rooms(true);
    }

    /// `prune` 为 false 时不移走已退出的房间。
    fn sync_rooms(&mut self, prune: bool) {
        let Ok(list) = self.core.sync_rooms() else {
            return;
        };
        let before: HashSet<RoomId> = self.rooms.keys().copied().collect();
        if prune {
            self.rooms.retain(|id, _| list.iter().any(|(r, _)| r == id));
        }
        for (room, status) in list {
            if let Some(rs) = self.rooms.get_mut(&room) {
                rs.status = status;
                continue;
            }
            let Ok(key) = self.core.room_key(&room) else {
                continue;
            };
            let now = Instant::now();
            self.rooms.insert(
                room,
                RoomSync {
                    key,
                    status,
                    peers: HashMap::new(),
                    next_have: now,
                    next_relay_have: now,
                    pending_have: None,
                    want: None,
                    failures: HashMap::new(),
                    told_dissolve: HashMap::new(),
                    floors: if status == RoomStatus::Joining {
                        FloorAsk::Needed
                    } else {
                        FloorAsk::Done
                    },
                },
            );
        }
        let after: HashSet<RoomId> = self.rooms.keys().copied().collect();
        if let Some(r) = &self.relay
            && after != before
        {
            r.set_rooms(after.into_iter().collect());
        }
    }

    /// 按本机设置（测试可用 `relay_urls` 覆盖中继列表）重建中继连接池；关闭互联网或列表为空时不连。
    fn apply_settings(&mut self) {
        // 先丢弃旧连接池，它的连接随之断开
        self.relay = None;
        self.relay_in = None;
        let Ok(settings) = self.core.settings() else {
            return;
        };
        if !settings.internet_allowed {
            return;
        }
        let urls = self.cfg.relay_urls.clone().unwrap_or(settings.relays);
        if urls.is_empty() {
            return;
        }
        let rooms = self.rooms.keys().copied().collect();
        let (handle, inbox) = RelayPool::start(urls, rooms, self.cfg.relay.clone());
        self.relay = Some(handle);
        self.relay_in = Some(inbox);
    }

    /// 某个房间里每位在线同伴当前的联系路径。
    pub fn member_paths(&self, room: &RoomId) -> Vec<(MemberId, PathKind)> {
        let now = Instant::now();
        self.rooms
            .get(room)
            .map(|rs| {
                rs.peers
                    .iter()
                    .map(|(m, p)| (*m, p.paths.kind(now)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn lan_status(&self) -> LanStatus {
        LanStatus {
            port: self.lan.local_addr().port(),
            degraded: self.lan_degraded,
        }
    }

    fn set_active_room(&mut self, room: Option<RoomId>) {
        if self.active_room == room {
            return;
        }
        self.active_room = room;
        // 刚切到的房间可能还在后台周期里，马上报一次
        if let Some(rs) = room.and_then(|r| self.rooms.get_mut(&r)) {
            let now = Instant::now();
            rs.next_have = now;
            rs.next_relay_have = now;
        }
    }

    /// 某个房间现在用的 Have 周期：(局域网, 中继)。
    pub fn have_periods(&self, room: &RoomId) -> (Duration, Duration) {
        if self.active_room.as_ref() == Some(room) {
            (self.cfg.have_period, self.cfg.relay_have_period)
        } else {
            (
                self.cfg.background_have_period,
                self.cfg.background_relay_have_period,
            )
        }
    }

    /// 网卡变了，当作网络恢复：所有房间尽快发 Have，中继不再等退避。
    fn check_network(&mut self, now: Instant) {
        if now < self.next_net_check {
            return;
        }
        self.next_net_check = now + NET_CHECK;
        let sig = crate::transport::lan::interface_signature();
        if sig == self.net_signature {
            return;
        }
        self.net_signature = sig;
        let rooms: Vec<RoomId> = self.rooms.keys().copied().collect();
        let delay = self.cfg.new_peer_have_delay;
        for room in rooms {
            self.schedule_have(&room, delay);
        }
        if let Some(r) = &self.relay {
            r.reconnect_now();
        }
    }

    /// 各中继的连接状态；关闭互联网时为 None。
    pub fn relay_status(&self) -> Option<Vec<RelayHealth>> {
        self.relay.as_ref().map(RelayHandle::status)
    }

    fn key_of(&self, room: &RoomId) -> Option<RoomKey> {
        self.rooms.get(room).map(|r| r.key)
    }

    /// 本人的包除了局域网广播，是否还要发一份到中继。
    fn needs_relay(&self, room: &RoomId) -> bool {
        let now = Instant::now();
        let peers = self
            .rooms
            .get(room)
            .into_iter()
            .flat_map(|rs| rs.peers.values())
            .map(|p| p.paths.kind(now));
        path::also_send_to_relay(self.relay.is_some(), peers)
    }

    /// 启动时把仍未确认的本人事件排进重发计划。
    fn queue_pending(&mut self) {
        let now = Instant::now();
        let rooms: Vec<RoomId> = self.rooms.keys().copied().collect();
        for room in rooms {
            for e in self.core.pending_events(&room).unwrap_or_default() {
                if worth_resending(&e) {
                    self.resend.add(room, e.event_id, e, now);
                }
            }
        }
    }

    /// 本人新写的事件：实时广播，需要时也发到中继；排进重发计划，等同伴确认。
    async fn send_events(&mut self, room: RoomId, events: Vec<SignedEvent>) {
        let now = Instant::now();
        for e in events.iter().filter(|e| worth_resending(e)) {
            self.resend.add(room, e.event_id, e.clone(), now);
        }
        self.broadcast_events(room, events).await;
    }

    async fn broadcast_events(&mut self, room: RoomId, events: Vec<SignedEvent>) {
        let Some(key) = self.key_of(&room) else {
            return;
        };
        let frame = Frame::Events {
            live: true,
            events: events.iter().map(WireEvent::from_signed).collect(),
        };
        let relay = self.needs_relay(&room).then_some(Priority::Live);
        let route = Route::Broadcast { relay };
        send_frame(&self.lan, self.relay.as_ref(), &key, &room, &frame, route).await;
    }

    /// 在一条路径上发 Have，周期按这条路径填。
    async fn send_have(&mut self, room: RoomId, via: Via) {
        let (Some(key), Ok(vector)) = (self.key_of(&room), self.core.have_vector(&room)) else {
            return;
        };
        let (lan_period, relay_period) = self.have_periods(&room);
        let (period, route) = match via {
            Via::Lan => (lan_period, Route::Broadcast { relay: None }),
            Via::Relay => (relay_period, Route::Relay(Priority::Control)),
        };
        let period_s = (period.as_millis() as u64).div_ceil(1000).max(1) as u16;
        let frame = Frame::Have {
            from: self.me,
            period_s,
            vector,
        };
        send_frame(&self.lan, self.relay.as_ref(), &key, &room, &frame, route).await;
    }

    fn schedule_have(&mut self, room: &RoomId, (lo, hi): (Duration, Duration)) {
        if let Some(rs) = self.rooms.get_mut(room)
            && rs.status != RoomStatus::Dissolved
            && rs.pending_have.is_none()
        {
            rs.pending_have = Some(Instant::now() + random_between(lo, hi));
        }
    }

    fn emit(&self, e: NodeEvent) {
        (self.sink)(e);
    }

    /// 入库等操作可能改变房间状态（比如收到解散），变了就刷新并通知界面。
    fn note_status_change(&mut self, room: RoomId, before: Option<RoomStatus>) {
        let after = self.core.room_status(&room).ok();
        if after != before {
            self.refresh_rooms();
            if let Some(status) = after {
                self.emit(NodeEvent::RoomStatusChanged {
                    room_id: room,
                    status,
                });
            }
        }
    }

    async fn on_packet(&mut self, source: Source, bytes: &[u8]) {
        let Some(room) = peek_room_id(bytes) else {
            return;
        };
        let Some(key) = self.key_of(&room) else {
            return; // 不是本机加入的房间
        };
        let frame = match open_packet(&key, bytes) {
            Ok(f) => f,
            Err(PacketError::ProtocolMismatch(p)) => {
                self.emit(NodeEvent::ProtocolMismatch {
                    room_id: room,
                    remote_proto: p,
                });
                return;
            }
            Err(_) => return,
        };
        match frame {
            Frame::Events { live, events } => self.on_events(room, live, events),
            Frame::Have {
                from: member,
                period_s,
                vector,
            } => self.on_have(room, source, member, period_s, vector).await,
            Frame::Want { to, ranges } => self.on_want(room, source, to, ranges),
            Frame::WantFloors { to, n } => self.on_want_floors(room, source, to, n).await,
            Frame::Floors { vector } => self.on_floors(room, vector).await,
        }
        self.maybe_activate(room).await;
    }

    fn on_events(&mut self, room: RoomId, live: bool, wire: Vec<WireEvent>) {
        let events: Vec<SignedEvent> = wire
            .into_iter()
            .filter_map(|w| w.into_signed(room))
            .collect();
        let before = self.core.room_status(&room).ok();
        let Ok(notes) = self.core.ingest_events(&room, &events, live, now_ms()) else {
            return;
        };
        let got_new = !notes.is_empty();
        for n in notes {
            self.emit(NodeEvent::Core(n));
        }
        self.note_status_change(room, before);
        if live && got_new {
            let delay = self.cfg.live_have_delay;
            self.schedule_have(&room, delay);
        }
    }

    async fn on_have(
        &mut self,
        room: RoomId,
        source: Source,
        member: MemberId,
        period_s: u16,
        vector: Vec<HaveEntry>,
    ) {
        if member == self.me {
            return; // 自己的广播绕回来了
        }
        let Ok(mine) = self.core.have_vector(&room) else {
            return;
        };
        let Some(rs) = self.rooms.get_mut(&room) else {
            return;
        };
        let now = Instant::now();
        let new_peer = !rs.peers.contains_key(&member);
        let period = Duration::from_secs(u64::from(period_s.max(1)));
        let peer = rs.peers.entry(member).or_insert_with(|| Peer {
            lan_addr: None,
            paths: PeerPaths::default(),
            vector: Vec::new(),
            last_seen: now,
        });
        peer.vector = vector.clone();
        peer.last_seen = now;
        peer.paths.seen(source.via(), now, Some(period));
        if let Source::Lan(addr) = source {
            peer.lan_addr = Some(addr);
        }
        if let Ok(ids) = self.core.mark_delivered(&room, &vector)
            && !ids.is_empty()
        {
            self.resend.confirm(&ids);
            self.emit(NodeEvent::Delivered {
                room_id: room,
                event_ids: ids,
            });
        }
        let Some(rs) = self.rooms.get_mut(&room) else {
            return;
        };
        if rs.pending_have.is_some() && dominates(&vector, &mine) {
            rs.pending_have = None; // 别人已经替我报了，不用再发（抑制）
        }
        let dissolved = rs.status == RoomStatus::Dissolved;
        if new_peer {
            let delay = self.cfg.new_peer_have_delay;
            self.schedule_have(&room, delay);
        }
        if dissolved {
            self.tell_dissolve(room, member, source, &vector).await;
        }
        self.plan_want(room).await;
    }

    async fn tell_dissolve(
        &mut self,
        room: RoomId,
        member: MemberId,
        source: Source,
        vector: &[HaveEntry],
    ) {
        let Ok(Some(d)) = self.core.dissolve_event(&room) else {
            return;
        };
        let owner = short_id(&d.event.author_id);
        if vector
            .iter()
            .any(|e| e.author == owner && e.state_max >= d.event.seq)
        {
            return; // 对方已经知道
        }
        let Some(rs) = self.rooms.get_mut(&room) else {
            return;
        };
        if rs
            .told_dissolve
            .get(&member)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        rs.told_dissolve.insert(member, Instant::now());
        let key = rs.key;
        let frame = Frame::Events {
            live: false,
            events: vec![WireEvent::from_signed(&d)],
        };
        let route = source.reply(Priority::Backfill);
        send_frame(&self.lan, self.relay.as_ref(), &key, &room, &frame, route).await;
    }

    fn on_want(&mut self, room: RoomId, source: Source, to: ShortId, ranges: Vec<WantRange>) {
        if to != short_id(&self.me) {
            return;
        }
        let Ok(mut events) = self.core.events_for(&room, &ranges) else {
            return;
        };
        events.truncate(MAX_WANT_EVENTS as usize);
        if events.is_empty() {
            return; // 没有就不回复，由对方超时换人
        }
        let Some(key) = self.key_of(&room) else {
            return;
        };
        let frame = Frame::Events {
            live: false,
            events: events.iter().map(WireEvent::from_signed).collect(),
        };
        let Ok(packets) = seal_packets(&key, &room, &frame) else {
            return;
        };
        match source {
            Source::Lan(addr) => self.backfill.extend(packets.into_iter().map(|p| (addr, p))),
            Source::Relay => self
                .relay_backfill
                .extend(packets.into_iter().map(|p| (room, p))),
        }
    }

    /// 没有在途的 Want 时，挑最近联系过、还没失败太多次的同伴，要它有、本机缺的事件：
    /// 先补状态事件（任何同伴有就先要），都齐了再补聊天（新成员要等问到分界之后）。
    async fn plan_want(&mut self, room: RoomId) {
        let Ok(holdings) = self.core.holdings(&room) else {
            return;
        };
        let Some(rs) = self.rooms.get(&room) else {
            return;
        };
        if rs.want.is_some() || rs.status == RoomStatus::Dissolved {
            return;
        }
        let now = Instant::now();
        let mut candidates: Vec<(&MemberId, &Peer)> = rs
            .peers
            .iter()
            .filter(|(m, _)| rs.failures.get(*m).copied().unwrap_or(0) < self.cfg.want_max_tries)
            .collect();
        candidates.sort_by_key(|(_, p)| std::cmp::Reverse(p.last_seen));
        let floors = self.floors_if_known(&room);
        let state = candidates.iter().find_map(|(m, p)| {
            let gaps = state_gaps(&holdings, &p.vector);
            (!gaps.is_empty()).then_some((**m, route_to(p, now), gaps))
        });
        let choice = state.or_else(|| {
            let floors = floors.as_ref()?;
            candidates.iter().find_map(|(m, p)| {
                let gaps = chat_gaps(&holdings, &p.vector, floors);
                (!gaps.is_empty()).then_some((**m, route_to(p, now), gaps))
            })
        });
        let Some((peer, route, gaps)) = choice else {
            return;
        };
        let key = rs.key;
        let missing_before = rs
            .peers
            .get(&peer)
            .map_or(0, |p| missing_total(&holdings, &p.vector, floors.as_ref()));
        let frame = Frame::Want {
            to: short_id(&peer),
            ranges: gaps,
        };
        send_frame(&self.lan, self.relay.as_ref(), &key, &room, &frame, route).await;
        if let Some(rs) = self.rooms.get_mut(&room) {
            rs.want = Some(InFlightWant {
                peer,
                sent_at: Instant::now(),
                missing_before,
            });
        }
    }

    /// 聊天补齐分界已知（不是正在问分界的新成员）时返回分界表。
    fn floors_if_known(&self, room: &RoomId) -> Option<HashMap<ShortId, u64>> {
        let done = self.rooms.get(room)?.floors == FloorAsk::Done;
        done.then(|| self.core.chat_floors(room).unwrap_or_default())
    }

    /// 在途的 Want 有进展（缺的总数变少）就放行下一个；超时没进展记一次失败，换人。
    /// 缺的总数不受每次 64 个的限制，否则缺得多时每批都会被误判为没进展。
    fn check_want(&mut self, room: RoomId) {
        let Ok(holdings) = self.core.holdings(&room) else {
            return;
        };
        let floors = self.floors_if_known(&room);
        let timeout = self.cfg.want_timeout;
        let Some(rs) = self.rooms.get_mut(&room) else {
            return;
        };
        let Some(w) = &rs.want else {
            return;
        };
        let missing_now = rs
            .peers
            .get(&w.peer)
            .map_or(0, |p| missing_total(&holdings, &p.vector, floors.as_ref()));
        if missing_now < w.missing_before {
            rs.failures.remove(&w.peer);
            rs.want = None;
        } else if w.sent_at.elapsed() > timeout {
            *rs.failures.entry(w.peer).or_default() += 1;
            rs.want = None;
        }
    }

    /// 新成员（Joining）先问一位同伴最近 n 条聊天从哪开始（架构 7.4），n 按到这位同伴的路径取本机上限。
    /// 问了一直没回（比如对方是旧版本）就放弃，聊天从 1 开始补。
    async fn maybe_ask_floors(&mut self, room: RoomId) {
        let Some(rs) = self.rooms.get_mut(&room) else {
            return;
        };
        match rs.floors {
            FloorAsk::Done => return,
            FloorAsk::Asked { at, .. } => {
                if at.elapsed() > self.cfg.want_timeout * 2 {
                    rs.floors = FloorAsk::Done;
                }
                return;
            }
            FloorAsk::Needed => {}
        }
        let now = Instant::now();
        let Some((peer, p)) = rs.peers.iter().max_by_key(|(_, p)| p.last_seen) else {
            return;
        };
        let (peer, route, key) = (*peer, route_to(p, now), rs.key);
        let Ok(settings) = self.core.settings() else {
            return;
        };
        let n = match route {
            Route::Lan(_) => settings.history_limit_lan,
            _ => settings.history_limit_relay,
        };
        let frame = Frame::WantFloors {
            to: short_id(&peer),
            n,
        };
        send_frame(&self.lan, self.relay.as_ref(), &key, &room, &frame, route).await;
        if let Some(rs) = self.rooms.get_mut(&room) {
            rs.floors = FloorAsk::Asked { at: now };
        }
    }

    /// 回答 WantFloors：n 取对方要的和本机这条路径上限的较小值。
    async fn on_want_floors(&mut self, room: RoomId, source: Source, to: ShortId, n: u32) {
        if to != short_id(&self.me) {
            return;
        }
        let (Some(key), Ok(settings)) = (self.key_of(&room), self.core.settings()) else {
            return;
        };
        let limit = match source {
            Source::Lan(_) => settings.history_limit_lan,
            Source::Relay => settings.history_limit_relay,
        };
        let Ok(vector) = self.core.floors_for_last(&room, n.min(limit)) else {
            return;
        };
        let frame = Frame::Floors { vector };
        let route = source.reply(Priority::Control);
        send_frame(&self.lan, self.relay.as_ref(), &key, &room, &frame, route).await;
    }

    /// 收到分界：列出的作者按列出的记；没列出但回答者有聊天的作者，分界设在回答者已有的之后（不要旧的）。
    /// 没在问分界时收到的（比如回答别人的）不理。
    async fn on_floors(&mut self, room: RoomId, vector: Vec<FloorEntry>) {
        let Some(rs) = self.rooms.get_mut(&room) else {
            return;
        };
        if !matches!(rs.floors, FloorAsk::Asked { .. }) {
            return;
        }
        let mut floors: Vec<(ShortId, u64)> = vector.iter().map(|f| (f.author, f.floor)).collect();
        let newest = rs.peers.values().max_by_key(|p| p.last_seen);
        for e in newest.map(|p| p.vector.as_slice()).unwrap_or_default() {
            if e.chat_max > 0 && !floors.iter().any(|(a, _)| *a == e.author) {
                floors.push((e.author, e.chat_max + 1));
            }
        }
        rs.floors = FloorAsk::Done;
        let _ = self.core.set_chat_floors(&room, &floors);
        self.plan_want(room).await;
    }

    async fn maybe_activate(&mut self, room: RoomId) {
        let Some(rs) = self.rooms.get(&room) else {
            return;
        };
        if rs.status != RoomStatus::Joining {
            return;
        }
        let peers: Vec<Vec<HaveEntry>> = rs.peers.values().map(|p| p.vector.clone()).collect();
        let (Ok(owner_known), Ok(holdings)) =
            (self.core.owner_known(&room), self.core.holdings(&room))
        else {
            return;
        };
        if !ready_to_activate(owner_known, &holdings, &peers, short_id(&self.me)) {
            return;
        }
        let Ok(written) = self.core.activate(&room, now_ms()) else {
            return;
        };
        self.refresh_rooms();
        self.send_events(room, written).await;
        self.emit(NodeEvent::RoomStatusChanged {
            room_id: room,
            status: RoomStatus::Active,
        });
        let delay = self.cfg.new_peer_have_delay;
        self.schedule_have(&room, delay);
    }

    async fn on_tick(&mut self) {
        let now = Instant::now();
        let relay_on = self.relay.is_some();
        let rooms: Vec<RoomId> = self.rooms.keys().copied().collect();
        self.check_network(now);
        for room in rooms {
            let needs_relay = self.needs_relay(&room);
            let (lan_period, relay_period) = self.have_periods(&room);
            let (mut lan_have, mut relay_have) = (false, false);
            if let Some(rs) = self.rooms.get_mut(&room) {
                rs.peers
                    .retain(|_, p| p.paths.kind(now) != PathKind::Unknown);
                if rs.status != RoomStatus::Dissolved {
                    let triggered = rs.pending_have.is_some_and(|t| now >= t);
                    if now >= rs.next_have || triggered {
                        rs.next_have = now + jitter(lan_period);
                        rs.pending_have = None;
                        lan_have = true;
                    }
                    if relay_on && (now >= rs.next_relay_have || (triggered && needs_relay)) {
                        rs.next_relay_have = now + jitter(relay_period);
                        relay_have = true;
                    }
                }
            }
            if lan_have {
                self.send_have(room, Via::Lan).await;
            }
            if relay_have {
                self.send_have(room, Via::Relay).await;
            }
            self.check_want(room);
            self.maybe_ask_floors(room).await;
            self.plan_want(room).await;
            self.maybe_activate(room).await;
        }
        let per_tick =
            (u64::from(self.cfg.backfill_per_sec) * self.cfg.tick.as_millis() as u64 / 1000).max(1);
        for _ in 0..per_tick {
            let Some((addr, p)) = self.backfill.pop_front() else {
                break;
            };
            self.lan.send_to(addr, &p).await;
        }
        self.drain_relay_backfill(now);
        let mut due: HashMap<RoomId, Vec<SignedEvent>> = HashMap::new();
        for (room, e) in self.resend.due(now) {
            due.entry(room).or_default().push(e);
        }
        for (room, events) in due {
            self.broadcast_events(room, events).await;
        }
    }

    /// 中继补齐按固定间隔一个一个发；队列空着时不攒额度，免得来活时一下子发一大串。
    fn drain_relay_backfill(&mut self, now: Instant) {
        let Some(relay) = &self.relay else {
            self.relay_backfill.clear(); // 关闭了互联网，发不出去
            return;
        };
        let interval = Duration::from_secs(1) / self.cfg.relay_backfill_per_sec.max(1);
        while now >= self.relay_backfill_at {
            let Some((room, p)) = self.relay_backfill.pop_front() else {
                self.relay_backfill_at = now;
                break;
            };
            relay.publish(room, p, Priority::Backfill);
            self.relay_backfill_at += interval;
        }
    }
}

/// 图片的各片不进重发计划：一张图上百片，重发三遍太占中继预算；丢了的片靠 Have/Want 补齐。
fn worth_resending(e: &SignedEvent) -> bool {
    e.event.kind != kind::IMAGE_CHUNK
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod relay_tests;
