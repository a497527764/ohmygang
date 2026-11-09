//! 导入表情传输的运行时：把 `transfer`（纯逻辑）接到节点、表情文件和设置上。
//!
//! 实现流程：
//! 1. 一个专门的线程，有传输进行时每 100 毫秒走一步，没有时只等消息（3.2.0）；别处（节点交来的实时帧）只往它的通道里发消息。
//! 2. 每条消息和每一步都先看设置：关了「显示队友的表情」或表情轮盘，就清空进行中的，不收也不发。
//! 3. Want 只处理发给我的；`transfer` 交出来的 Want 和分块发给队伍，收齐验过的图存进缓存。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::api::AppState;
use crate::crypto::MemberId;
use crate::node::now_ms;
use crate::protocol::live::LiveBody;

use super::store::Store;
use super::transfer::{Out, TICK_MS, Transfer};

pub enum NetMsg {
    /// 队友的在线状态报了这些哈希。
    Announced { from: MemberId, hashes: Vec<String> },
    Want {
        from: MemberId,
        to: MemberId,
        hash: String,
        need: Vec<u8>,
    },
    Chunk {
        from: MemberId,
        hash: String,
        i: u8,
        n: u8,
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
    transfer: Transfer,
    me: Option<MemberId>,
}

impl Runner {
    fn run(mut self, rx: Receiver<NetMsg>) {
        let tick = Duration::from_millis(TICK_MS);
        let mut next = Instant::now() + tick;
        loop {
            let busy = self.transfer.busy();
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

    /// 我现在报的哈希；None 表示传输关着。
    fn mine(&self) -> Option<Vec<String>> {
        let state = self.app.try_state::<AppState>()?;
        let s = state.settings.lock().expect("设置锁没有中毒");
        (s.emote_trigger.is_some() && s.friend_emotes).then(|| s.announced_emotes())
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
        let Some(mine) = self.mine() else {
            self.transfer.clear();
            return;
        };
        let store = self.store.clone();
        let have = |h: &str| store.has_still(h);
        let out = match m {
            NetMsg::Announced { from, hashes } => {
                self.transfer.announced(from, hashes, have, now_ms())
            }
            NetMsg::Want { to, hash, need, .. } => {
                if Some(to) == self.me() {
                    self.transfer
                        .on_want(&hash, &need, &mine, |h| store.own_still(h));
                }
                Vec::new()
            }
            NetMsg::Chunk {
                from,
                hash,
                i,
                n,
                bytes,
            } => self
                .transfer
                .on_chunk(from, &hash, (i, n), bytes, have, now_ms()),
            NetMsg::Gone(from) => {
                self.transfer.gone(&from);
                Vec::new()
            }
        };
        self.deliver(out);
    }

    fn step(&mut self) {
        match self.mine() {
            Some(mine) => {
                let out = self.transfer.tick(now_ms(), &mine);
                self.deliver(out);
            }
            None => self.transfer.clear(),
        }
    }

    fn deliver(&self, out: Vec<Out>) {
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
}
