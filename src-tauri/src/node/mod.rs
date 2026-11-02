//! 节点（架构第 7 节的 M1-3 部分）：独占 Core，把界面命令、局域网收包、定时器串在一个后台任务里。
//!
//! 实现流程：
//! 1. 命令：界面通过 `NodeHandle::call` 发来一个闭包，在节点任务里拿到 `&mut Core` 执行；
//!    闭包把新写的事件放进 `Outbox`，节点随后用 `Events { live: true }` 广播出去。
//! 2. 收包：按包头 room_id 找到房间密钥解密。Events 交给 Core 入库；Have 记下同伴（地址、向量、周期），
//!    再做抑制、补齐和激活检查；Want 把对方要的事件放进回应队列，按限速慢慢发出。
//! 3. 定时器：周期 Have（±20% 抖动）；延迟 Have（收到实时事件后 1–5 秒、发现新同伴后 0–2 秒，
//!    期间被别人的 Have 覆盖就取消）；Want 超时换人；同伴超过 3 个周期没消息视为离线；Joining 房间检查能否激活。
//! 4. 已解散的房间不主动发 Have；同伴的 Have 显示它还不知道解散时，回一次解散事件（同一同伴 1 分钟内最多一次）。
//! 5. 通知通过 sink 交出去：测试里收集起来检查，界面层转发成 Tauri 事件。
//!
//! 注意：Core 里的 SQLite 连接不能被多线程共享，所以等待网络发送时不能借用着 Core；
//! 发包的函数只拿传输层和密钥（`send_frame`），不借用整个节点。

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot};
use tokio::time::MissedTickBehavior;

use crate::app_core::{Core, CoreEvent, RoomStatus};
use crate::crypto::{MemberId, RoomId, RoomKey};
use crate::protocol::event::SignedEvent;
use crate::protocol::frame::{Frame, HaveEntry, ShortId, WantRange, WireEvent, short_id};
use crate::protocol::packet::{PacketError, open_packet, peek_room_id, seal_packets};
use crate::sync::plan::{MAX_WANT_EVENTS, dominates, ready_to_activate, state_gaps};
use crate::transport::lan::{Inbox, LanConfig, LanTransport};

/// 节点交给界面的通知。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeEvent {
    Core(CoreEvent),
    RoomStatusChanged { room_id: RoomId, status: RoomStatus },
    ProtocolMismatch { room_id: RoomId, remote_proto: u8 },
}

pub type Sink = Arc<dyn Fn(NodeEvent) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub lan: LanConfig,
    pub have_period: Duration,
    pub live_have_delay: (Duration, Duration),
    pub new_peer_have_delay: (Duration, Duration),
    pub want_timeout: Duration,
    pub want_max_tries: u32,
    /// 回应 Want 的发送速率（包/秒）。
    pub backfill_per_sec: u32,
    pub tick: Duration,
}

impl Default for NodeConfig {
    fn default() -> Self {
        NodeConfig {
            lan: LanConfig::default(),
            have_period: Duration::from_secs(5),
            live_have_delay: (Duration::from_secs(1), Duration::from_secs(5)),
            new_peer_have_delay: (Duration::ZERO, Duration::from_secs(2)),
            want_timeout: Duration::from_secs(5),
            want_max_tries: 3,
            backfill_per_sec: 50,
            tick: Duration::from_millis(100),
        }
    }
}

/// 命令执行过程中要发出去的事件和要交给界面的通知。
#[derive(Default)]
pub struct Outbox {
    broadcasts: Vec<(RoomId, Vec<SignedEvent>)>,
    pub notes: Vec<NodeEvent>,
}

impl Outbox {
    pub fn broadcast(&mut self, room: RoomId, events: Vec<SignedEvent>) {
        if !events.is_empty() {
            self.broadcasts.push((room, events));
        }
    }
}

type Job = Box<dyn FnOnce(&mut Core, &mut Outbox) + Send>;

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
        let job: Job = Box::new(move |core, out| {
            let _ = tx.send(f(core, out));
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

async fn send_frame(
    lan: &LanTransport,
    key: &RoomKey,
    room: &RoomId,
    frame: &Frame,
    to: Option<SocketAddr>,
) {
    let Ok(packets) = seal_packets(key, room, frame) else {
        return;
    };
    for p in packets {
        match to {
            Some(addr) => lan.send_to(addr, &p).await,
            None => lan.broadcast(&p).await,
        }
    }
}

struct Peer {
    addr: SocketAddr,
    vector: Vec<HaveEntry>,
    period: Duration,
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
    pending_have: Option<Instant>,
    want: Option<InFlightWant>,
    failures: HashMap<MemberId, u32>,
    told_dissolve: HashMap<MemberId, Instant>,
}

pub struct Node {
    core: Core,
    lan: LanTransport,
    cfg: NodeConfig,
    sink: Sink,
    me: MemberId,
    rooms: HashMap<RoomId, RoomSync>,
    backfill: VecDeque<(SocketAddr, Vec<u8>)>,
}

fn missing_count(gaps: &[WantRange]) -> u64 {
    gaps.iter().map(|r| r.to_seq - r.from_seq + 1).sum()
}

impl Node {
    pub async fn spawn(core: Core, cfg: NodeConfig, sink: Sink) -> std::io::Result<NodeHandle> {
        let (lan, inbox) = LanTransport::bind(&cfg.lan).await?;
        let (tx, rx) = mpsc::channel::<Job>(64);
        let me = core.identity().member_id;
        let mut node = Node {
            core,
            lan,
            cfg,
            sink,
            me,
            rooms: HashMap::new(),
            backfill: VecDeque::new(),
        };
        node.refresh_rooms();
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
                    let mut out = Outbox::default();
                    job(&mut self.core, &mut out);
                    self.refresh_rooms();
                    for (room, events) in out.broadcasts {
                        self.send_events(room, events).await;
                    }
                    for n in out.notes {
                        (self.sink)(n);
                    }
                }
                pkt = inbox.recv() => {
                    let Some((from, bytes)) = pkt else { break };
                    self.on_packet(from, &bytes).await;
                }
                _ = tick.tick() => self.on_tick().await,
            }
        }
    }

    /// 和 Core 的房间列表对齐：新房间取密钥，已有的更新状态，已退出的移除。
    fn refresh_rooms(&mut self) {
        let Ok(list) = self.core.sync_rooms() else {
            return;
        };
        self.rooms.retain(|id, _| list.iter().any(|(r, _)| r == id));
        for (room, status) in list {
            if let Some(rs) = self.rooms.get_mut(&room) {
                rs.status = status;
                continue;
            }
            let Ok(key) = self.core.room_key(&room) else {
                continue;
            };
            self.rooms.insert(
                room,
                RoomSync {
                    key,
                    status,
                    peers: HashMap::new(),
                    next_have: Instant::now(),
                    pending_have: None,
                    want: None,
                    failures: HashMap::new(),
                    told_dissolve: HashMap::new(),
                },
            );
        }
    }

    fn key_of(&self, room: &RoomId) -> Option<RoomKey> {
        self.rooms.get(room).map(|r| r.key)
    }

    /// 本人新写的事件：实时广播。
    async fn send_events(&mut self, room: RoomId, events: Vec<SignedEvent>) {
        let Some(key) = self.key_of(&room) else {
            return;
        };
        let frame = Frame::Events {
            live: true,
            events: events.iter().map(WireEvent::from_signed).collect(),
        };
        send_frame(&self.lan, &key, &room, &frame, None).await;
    }

    async fn send_have(&mut self, room: RoomId) {
        let (Some(key), Ok(vector)) = (self.key_of(&room), self.core.have_vector(&room)) else {
            return;
        };
        let period_s = (self.cfg.have_period.as_millis() as u64)
            .div_ceil(1000)
            .max(1) as u16;
        let frame = Frame::Have {
            from: self.me,
            period_s,
            vector,
        };
        send_frame(&self.lan, &key, &room, &frame, None).await;
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

    async fn on_packet(&mut self, from: SocketAddr, bytes: &[u8]) {
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
            } => self.on_have(room, from, member, period_s, vector).await,
            Frame::Want { to, ranges } => self.on_want(room, from, to, ranges),
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
        addr: SocketAddr,
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
        let new_peer = !rs.peers.contains_key(&member);
        let period = Duration::from_secs(u64::from(period_s.max(1)));
        rs.peers.insert(
            member,
            Peer {
                addr,
                vector: vector.clone(),
                period,
                last_seen: Instant::now(),
            },
        );
        if rs.pending_have.is_some() && dominates(&vector, &mine) {
            rs.pending_have = None; // 别人已经替我报了，不用再发（抑制）
        }
        let dissolved = rs.status == RoomStatus::Dissolved;
        if new_peer {
            let delay = self.cfg.new_peer_have_delay;
            self.schedule_have(&room, delay);
        }
        if dissolved {
            self.tell_dissolve(room, member, addr, &vector).await;
        }
        self.plan_want(room).await;
    }

    async fn tell_dissolve(
        &mut self,
        room: RoomId,
        member: MemberId,
        addr: SocketAddr,
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
        send_frame(&self.lan, &key, &room, &frame, Some(addr)).await;
    }

    fn on_want(&mut self, room: RoomId, addr: SocketAddr, to: ShortId, ranges: Vec<WantRange>) {
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
        if let Ok(packets) = seal_packets(&key, &room, &frame) {
            self.backfill.extend(packets.into_iter().map(|p| (addr, p)));
        }
    }

    /// 没有在途的 Want 时，挑最近联系过、还没失败太多次的同伴，要它有、本机缺的状态事件。
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
        let mut candidates: Vec<(&MemberId, &Peer)> = rs
            .peers
            .iter()
            .filter(|(m, _)| rs.failures.get(*m).copied().unwrap_or(0) < self.cfg.want_max_tries)
            .collect();
        candidates.sort_by_key(|(_, p)| std::cmp::Reverse(p.last_seen));
        let choice = candidates.into_iter().find_map(|(m, p)| {
            let gaps = state_gaps(&holdings, &p.vector);
            (!gaps.is_empty()).then_some((*m, p.addr, gaps))
        });
        let Some((peer, addr, gaps)) = choice else {
            return;
        };
        let key = rs.key;
        let missing_before = missing_count(&gaps);
        let frame = Frame::Want {
            to: short_id(&peer),
            ranges: gaps,
        };
        send_frame(&self.lan, &key, &room, &frame, Some(addr)).await;
        if let Some(rs) = self.rooms.get_mut(&room) {
            rs.want = Some(InFlightWant {
                peer,
                sent_at: Instant::now(),
                missing_before,
            });
        }
    }

    /// 在途的 Want 有进展就放行下一个；超时没进展记一次失败，换人。
    fn check_want(&mut self, room: RoomId) {
        let Ok(holdings) = self.core.holdings(&room) else {
            return;
        };
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
            .map_or(0, |p| missing_count(&state_gaps(&holdings, &p.vector)));
        if missing_now < w.missing_before {
            rs.failures.remove(&w.peer);
            rs.want = None;
        } else if w.sent_at.elapsed() > timeout {
            *rs.failures.entry(w.peer).or_default() += 1;
            rs.want = None;
        }
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
        if !ready_to_activate(owner_known, &holdings, &peers) {
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
        let rooms: Vec<RoomId> = self.rooms.keys().copied().collect();
        for room in rooms {
            let mut send = false;
            if let Some(rs) = self.rooms.get_mut(&room) {
                rs.peers.retain(|_, p| p.last_seen.elapsed() < p.period * 3);
                if rs.status != RoomStatus::Dissolved {
                    let periodic = now >= rs.next_have;
                    let triggered = rs.pending_have.is_some_and(|t| now >= t);
                    if periodic || triggered {
                        rs.next_have = now + jitter(self.cfg.have_period);
                        rs.pending_have = None;
                        send = true;
                    }
                }
            }
            if send {
                self.send_have(room).await;
            }
            self.check_want(room);
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
    }
}

#[cfg(test)]
mod tests;
