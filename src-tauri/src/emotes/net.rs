//! 导入表情传输的运行时：把 `transfer`（纯逻辑）接到节点、表情文件和设置上。
//! 3.4 起大头像（个人资料，200×200）也走同一个分块机制：再挂一个 `Transfer` 实例，
//! 两个实例独立攒块、独立校验，同一个 Want/Chunk 谁认识谁处理（哈希不同不会串）。
//!
//! 实现流程：
//! 1. 一个专门的线程，有传输进行时每 100 毫秒走一步，没有时只等消息（3.2.0）；别处（节点交来的实时帧）只往它的通道里发消息。
//! 2. 每条消息和每一步都先看设置：关了「显示队友的表情」或表情轮盘，就清空表情的进行中的，不收也不发；
//!    头像不受这两个开关影响（个人资料的基本组成部分）。
//! 3. Want 只处理发给我的；`transfer` 交出来的 Want 和分块发给队伍，收齐验过的图存进缓存。
//! 4. 我的头像哈希：从身份表读（`IdentityView.avatar_hash`），报在在线状态里，队友没有就来要。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::api::AppState;
use crate::crypto::MemberId;
use crate::node::now_ms;
use crate::protocol::live::LiveBody;

use super::store::Store;
use super::transfer::{Out, TICK_MS, Transfer};

pub enum NetMsg {
    /// 队友的在线状态报了这些哈希；头像哈希（在线状态里单列的字段）分开传。
    Announced {
        from: MemberId,
        hashes: Vec<String>,
        avatar: Option<String>,
    },
    Want {
        from: MemberId,
        to: MemberId,
        hash: String,
        need: Vec<u16>,
    },
    Chunk {
        from: MemberId,
        hash: String,
        i: u16,
        n: u16,
        bytes: Vec<u8>,
    },
    /// 队员离开或退出程序。
    Gone(MemberId),
}

#[derive(Clone)]
pub struct EmoteNet {
    tx: Sender<NetMsg>,
}

impl EmoteNet {
    pub fn start(app: &AppHandle, store: Arc<Store>) -> Self {
        let (tx, rx) = mpsc::channel();
        let runner = Runner {
            app: app.clone(),
            store,
            transfer: Transfer::default(),
            avatars: Transfer::default(),
            me: None,
        };
        std::thread::Builder::new()
            .name("emote-net".into())
            .spawn(move || runner.run(rx))
            .expect("能开线程");
        Self { tx }
    }

    pub fn send(&self, m: NetMsg) {
        let _ = self.tx.send(m);
    }
}

struct Runner {
    app: AppHandle,
    store: Arc<Store>,
    /// 表情的分块传输。
    transfer: Transfer,
    /// 大头像的分块传输（机制同表情，各攒各的）。
    avatars: Transfer,
    me: Option<MemberId>,
}

impl Runner {
    fn run(mut self, rx: Receiver<NetMsg>) {
        let tick = Duration::from_millis(TICK_MS);
        let mut next = Instant::now() + tick;
        loop {
            let busy = self.transfer.busy() || self.avatars.busy();
            let got = if busy {
                rx.recv_timeout(next.saturating_duration_since(Instant::now()))
            } else {
                rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            match got {
                Ok(m) => self.handle(m),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if !busy {
                // 刚有了传输：从现在起每 100 毫秒走一步
                next = Instant::now() + tick;
                continue;
            }
            if Instant::now() >= next {
                next = Instant::now() + tick;
                self.step();
            }
        }
    }

    /// 我现在报的表情哈希；None 表示表情传输关着。
    fn mine(&self) -> Option<Vec<String>> {
        let state = self.app.try_state::<AppState>()?;
        let s = state.settings.lock().expect("设置锁没有中毒");
        (s.emote_trigger.is_some() && s.friend_emotes).then(|| s.announced_emotes())
    }

    /// 我的大头像哈希（报在在线状态里，队友没有这张就会来要）。
    fn my_avatar(&self) -> Option<String> {
        let state = self.app.try_state::<AppState>()?;
        let node = state.node.clone();
        let hash = tauri::async_runtime::block_on(node.call(|core, _| {
            core.identity().avatar_hash.clone()
        }))
        .flatten();
        hash.filter(|h| self.store.has_avatar(h))
    }

    fn me(&mut self) -> Option<MemberId> {
        if self.me.is_none() {
            let node = self.app.try_state::<AppState>()?.node.clone();
            self.me =
                tauri::async_runtime::block_on(node.call(|core, _| core.identity().member_id));
        }
        self.me
    }

    fn handle(&mut self, m: NetMsg) {
        let store = self.store.clone();
        match m {
            NetMsg::Announced {
                from,
                hashes,
                avatar,
            } => {
                // 表情：关了就不收
                let out = match self.mine() {
                    Some(_) => {
                        let have = |h: &str| store.has_still(h);
                        self.transfer.announced(from, hashes, have, now_ms())
                    }
                    None => {
                        self.transfer.clear();
                        Vec::new()
                    }
                };
                self.deliver_emotes(out);
                // 头像：始终收
                if let Some(h) = avatar.filter(|h| super::is_hash(h)) {
                    let have = |h: &str| store.has_avatar(h);
                    let out = self
                        .avatars
                        .announced(from, vec![h], have, now_ms());
                    self.deliver_avatars(out);
                }
            }
            NetMsg::Want { to, hash, need, .. } => {
                if Some(to) == self.me() {
                    // 同一个 Want：表情和头像谁认识谁应（哈希不会撞）；回应的分块由 tick 发出
                    if let Some(mine) = self.mine() {
                        let mine = mine.clone();
                        self.transfer
                            .on_want(&hash, &need, &mine, |h| store.own_still(h));
                    }
                    if let Some(mine) = self.my_avatar() {
                        let mine = vec![mine];
                        self.avatars
                            .on_want(&hash, &need, &mine, |h| store.own_avatar(h));
                    }
                }
            }
            NetMsg::Chunk {
                from,
                hash,
                i,
                n,
                bytes,
            } => {
                let have = |h: &str| store.has_still(h);
                let out = self.transfer.on_chunk(from, &hash, (i, n), bytes.clone(), have, now_ms());
                self.deliver_emotes(out);
                let have = |h: &str| store.has_avatar(h);
                let out = self.avatars.on_chunk(from, &hash, (i, n), bytes, have, now_ms());
                self.deliver_avatars(out);
            }
            NetMsg::Gone(from) => {
                self.transfer.gone(&from);
                self.avatars.gone(&from);
            }
        }
    }

    fn step(&mut self) {
        if let Some(mine) = self.mine() {
            let out = self.transfer.tick(now_ms(), &mine);
            self.deliver_emotes(out);
        } else {
            self.transfer.clear();
        }
        if let Some(mine) = self.my_avatar() {
            let mine = vec![mine];
            let out = self.avatars.tick(now_ms(), &mine);
            self.deliver_avatars(out);
        } else {
            self.avatars.clear();
        }
    }

    /// 表情通道的输出（发出去的分块 / 存缓存）。
    fn deliver_emotes(&self, out: Vec<Out>) {
        let Some(state) = self.app.try_state::<AppState>() else {
            return;
        };
        for o in out {
            match o {
                Out::Want { to, hash, need } => {
                    if let Some(hash) = super::hash_bytes(&hash) {
                        state.node.send_live(LiveBody::Want { to, hash, need });
                    }
                }
                Out::Chunk { hash, i, n, bytes } => {
                    if let Some(hash) = super::hash_bytes(&hash) {
                        state.node.send_live(LiveBody::Chunk { hash, i, n, bytes });
                    }
                }
                Out::Store { hash, bytes } => {
                    self.store.put_cached(&hash, &bytes);
                }
            }
        }
    }

    /// 头像通道的输出（发出去的分块 / 存文件）。
    fn deliver_avatars(&self, out: Vec<Out>) {
        let Some(state) = self.app.try_state::<AppState>() else {
            return;
        };
        for o in out {
            match o {
                Out::Want { to, hash, need } => {
                    if let Some(hash) = super::hash_bytes(&hash) {
                        state.node.send_live(LiveBody::Want { to, hash, need });
                    }
                }
                Out::Chunk { hash, i, n, bytes } => {
                    if let Some(hash) = super::hash_bytes(&hash) {
                        state.node.send_live(LiveBody::Chunk { hash, i, n, bytes });
                    }
                }
                Out::Store { hash, bytes } => {
                    if self.store.put_avatar_cached(&hash, &bytes) {
                        // 队友的头像到了：控制面板的成员列表要换图
                        let _ = self.app.emit_to("main", "team://changed", ());
                    }
                }
            }
        }
    }
}
