//! 队友导入表情的静态图怎么传过来（纯逻辑，时间由调用方传入）。做法移植自 lolPing `src/main/emoteTransfer.ts`
//! （MIT，© 2026 Danrui Meng），简化成全部广播：一份分块大家都能用。
//!
//! 实现流程：
//! 1. 队友的在线状态里报了他轮盘上导入表情的哈希。本机没有的，向他发 Want（广播，`to` 写他）。
//! 2. 收到发给我的 Want：只给自己在线状态里正在报的哈希，读一次静态图，切成 900 字节一块排队；
//!    每 100 毫秒最多发 2 块（局域网约 18 KB/s，中继再由发送预算按最低优先级慢慢发）。几个人要同一张只排一份。
//! 3. 收到分块：只收报过这个哈希的人发来的；凑齐后按顺序拼起来，SHA-256 前 16 字节对得上、
//!    文件头检查也过了（`store::check_still`）才存，所以谁也换不掉别人的图。
//! 4. 3 秒没新分块就再要缺的那几块，最多 3 次；60 秒还没齐、或那人不再报这个哈希，就放弃，等下一次在线状态再来。
//!    同时最多拼 16 张、256 KB。

use std::collections::{BTreeSet, HashMap};

use crate::crypto::MemberId;

use super::store::{self, STILL_MAX_BYTES};

pub const CHUNK_BYTES: usize = 900;
/// 12 KB 的静态图最多切成这么多块。
pub const CHUNKS_MAX: u8 = STILL_MAX_BYTES.div_ceil(CHUNK_BYTES) as u8;
pub const TICK_MS: u64 = 100;
const CHUNKS_PER_TICK: usize = 2;
const MAX_ASSEMBLIES: usize = 16;
const MAX_PENDING_BYTES: usize = 256 * 1024;
const ASSEMBLY_TIMEOUT_MS: u64 = 60_000;
const REASK_MS: u64 = 3_000;
const REASKS_MAX: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// 向 `to` 要这张图；`need` 为空表示全要。
    Want {
        to: MemberId,
        hash: String,
        need: Vec<u8>,
    },
    Chunk {
        hash: String,
        i: u8,
        n: u8,
        bytes: Vec<u8>,
    },
    /// 收齐、验过了，存进缓存。
    Store { hash: String, bytes: Vec<u8> },
}

struct Assembly {
    /// 一共几块，收到第一块才知道（之前是 0）。
    n: u8,
    parts: HashMap<u8, Vec<u8>>,
    bytes: usize,
    first_ms: u64,
    last_ms: u64,
    /// 向谁要。
    ask: MemberId,
    reasks: u32,
}

struct Sending {
    bytes: Vec<u8>,
    n: u8,
    need: BTreeSet<u8>,
}

#[derive(Default)]
pub struct Transfer {
    announcers: HashMap<MemberId, Vec<String>>,
    assemblies: HashMap<String, Assembly>,
    sends: HashMap<String, Sending>,
}

impl Transfer {
    /// `peer` 的在线状态报了这些哈希：本机没有的去要。
    pub fn announced(
        &mut self,
        peer: MemberId,
        hashes: Vec<String>,
        have: impl Fn(&str) -> bool,
        now_ms: u64,
    ) -> Vec<Out> {
        let mut out = Vec::new();
        for hash in &hashes {
            if have(hash) || self.assemblies.contains_key(hash) {
                continue;
            }
            if self.assemblies.len() >= MAX_ASSEMBLIES {
                break;
            }
            self.assemblies
                .insert(hash.clone(), Assembly::new(peer, now_ms));
            out.push(Out::Want {
                to: peer,
                hash: hash.clone(),
                need: Vec::new(),
            });
        }
        self.announcers.insert(peer, hashes);
        out
    }

    /// 有人要我的图。`mine` 是我现在报的哈希，`still` 读我导入的静态图。
    pub fn on_want(
        &mut self,
        hash: &str,
        need: &[u8],
        mine: &[String],
        still: impl FnOnce(&str) -> Option<Vec<u8>>,
    ) {
        if !mine.iter().any(|m| m == hash) {
            return;
        }
        if !self.sends.contains_key(hash) {
            // 每张图只读一次盘：一大堆 Want 也不会让它反复读
            let Some(bytes) = still(hash).filter(|b| !b.is_empty() && b.len() <= STILL_MAX_BYTES)
            else {
                return;
            };
            let n = bytes.len().div_ceil(CHUNK_BYTES) as u8;
            self.sends.insert(
                hash.to_string(),
                Sending {
                    bytes,
                    n,
                    need: BTreeSet::new(),
                },
            );
        }
        let s = self.sends.get_mut(hash).expect("刚放进去");
        if need.is_empty() {
            s.need.extend(0..s.n);
        } else {
            s.need.extend(need.iter().filter(|i| **i < s.n));
        }
    }

    /// 收到一块。凑齐并验过时返回 Store。
    pub fn on_chunk(
        &mut self,
        from: MemberId,
        hash: &str,
        (i, n): (u8, u8),
        bytes: Vec<u8>,
        have: impl Fn(&str) -> bool,
        now_ms: u64,
    ) -> Vec<Out> {
        let announced = self
            .announcers
            .get(&from)
            .is_some_and(|h| h.iter().any(|x| x == hash));
        if !announced || n == 0 || n > CHUNKS_MAX || i >= n || bytes.len() > CHUNK_BYTES {
            return Vec::new();
        }
        if !self.assemblies.contains_key(hash) {
            // 已经有了：迟到的分块不能再开一份
            let pending: usize = self.assemblies.values().map(|a| a.bytes).sum();
            if have(hash)
                || self.assemblies.len() >= MAX_ASSEMBLIES
                || pending + bytes.len() > MAX_PENDING_BYTES
            {
                return Vec::new();
            }
            self.assemblies
                .insert(hash.to_string(), Assembly::new(from, now_ms));
        }
        let a = self.assemblies.get_mut(hash).expect("刚放进去");
        if a.n == 0 {
            a.n = n;
        } else if a.n != n {
            // 同一个人对块数说法不一：丢掉，等下一次在线状态
            self.assemblies.remove(hash);
            return Vec::new();
        }
        if a.parts.contains_key(&i) {
            return Vec::new();
        }
        a.bytes += bytes.len();
        a.parts.insert(i, bytes);
        a.last_ms = now_ms;
        if a.parts.len() < usize::from(a.n) {
            return Vec::new();
        }
        let a = self.assemblies.remove(hash).expect("还在");
        let full: Vec<u8> = (0..a.n).flat_map(|k| a.parts[&k].iter().copied()).collect();
        if store::hash_of(&full) != hash || store::check_still(&full).is_err() {
            return Vec::new();
        }
        vec![Out::Store {
            hash: hash.to_string(),
            bytes: full,
        }]
    }

    /// 队员离开或下线。
    pub fn gone(&mut self, peer: &MemberId) {
        self.announcers.remove(peer);
        self.assemblies.retain(|_, a| a.ask != *peer);
    }

    /// 关了「显示队友的表情」：两个方向都停，进行中的丢掉。
    /// 有正在发的或正在收的：`tick` 才有事做（没有时运行时不用定时醒，3.2.0）。
    pub fn busy(&self) -> bool {
        !self.sends.is_empty() || !self.assemblies.is_empty()
    }

    pub fn clear(&mut self) {
        self.announcers.clear();
        self.assemblies.clear();
        self.sends.clear();
    }

    /// 每 100 毫秒一次：发分块、补要缺的块、放弃超时的。
    pub fn tick(&mut self, now_ms: u64, mine: &[String]) -> Vec<Out> {
        let mut out = Vec::new();
        // 不在我的轮盘上了：谁也不能再要到它
        self.sends.retain(|hash, _| mine.contains(hash));
        let mut budget = CHUNKS_PER_TICK;
        for (hash, s) in &mut self.sends {
            while budget > 0
                && let Some(i) = s.need.pop_first()
            {
                let from = usize::from(i) * CHUNK_BYTES;
                let to = (from + CHUNK_BYTES).min(s.bytes.len());
                out.push(Out::Chunk {
                    hash: hash.clone(),
                    i,
                    n: s.n,
                    bytes: s.bytes[from..to].to_vec(),
                });
                budget -= 1;
            }
        }
        self.sends.retain(|_, s| !s.need.is_empty());

        let announcers = &self.announcers;
        self.assemblies.retain(|hash, a| {
            let still_announced = announcers.get(&a.ask).is_some_and(|h| h.contains(hash));
            if now_ms.saturating_sub(a.first_ms) >= ASSEMBLY_TIMEOUT_MS || !still_announced {
                return false;
            }
            if now_ms.saturating_sub(a.last_ms) < REASK_MS {
                return true;
            }
            if a.reasks >= REASKS_MAX {
                return false;
            }
            let need = if a.n == 0 {
                Vec::new()
            } else {
                (0..a.n).filter(|i| !a.parts.contains_key(i)).collect()
            };
            out.push(Out::Want {
                to: a.ask,
                hash: hash.clone(),
                need,
            });
            a.reasks += 1;
            a.last_ms = now_ms;
            true
        });
        out
    }
}

impl Assembly {
    fn new(ask: MemberId, now_ms: u64) -> Self {
        Self {
            n: 0,
            parts: HashMap::new(),
            bytes: 0,
            first_ms: now_ms,
            last_ms: now_ms,
            ask,
            reasks: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emotes::store::{hash_of, test_webp};

    const A: MemberId = [1u8; 32];
    const B: MemberId = [2u8; 32];

    fn chunks(out: &[Out]) -> Vec<(String, u8, u8, Vec<u8>)> {
        out.iter()
            .filter_map(|o| match o {
                Out::Chunk { hash, i, n, bytes } => Some((hash.clone(), *i, *n, bytes.clone())),
                _ => None,
            })
            .collect()
    }

    /// A 有一张图、B 没有：B 要，A 分块发，B 收齐存下。
    #[test]
    fn a_still_moves_from_announcer_to_receiver() {
        let still = test_webp(100, 100, 3000);
        let h = hash_of(&still);
        let mine = vec![h.clone()];
        let (mut a, mut b) = (Transfer::default(), Transfer::default());

        let wants = b.announced(A, mine.clone(), |_| false, 0);
        assert_eq!(
            wants,
            vec![Out::Want {
                to: A,
                hash: h.clone(),
                need: vec![]
            }]
        );
        a.on_want(&h, &[], &mine, |_| Some(still.clone()));
        let mut got = Vec::new();
        for t in 1..=10 {
            got.extend(chunks(&a.tick(t * TICK_MS, &mine)));
        }
        assert_eq!(got.len(), 4, "3000 字节切 4 块");
        assert!(got.iter().all(|c| c.2 == 4));
        let mut stored = Vec::new();
        for (hash, i, n, bytes) in got.into_iter().rev() {
            stored.extend(b.on_chunk(A, &hash, (i, n), bytes, |_| false, 500));
        }
        assert_eq!(
            stored,
            vec![Out::Store {
                hash: h,
                bytes: still
            }],
            "倒着来也能拼对"
        );
    }

    #[test]
    fn busy_only_while_sending_or_receiving() {
        let still = test_webp(100, 100, 3000);
        let h = hash_of(&still);
        let mine = vec![h.clone()];
        let mut a = Transfer::default();
        assert!(!a.busy(), "刚开始没事做，线程不用定时醒");
        a.on_want(&h, &[], &mine, |_| Some(still.clone()));
        assert!(a.busy(), "有人要图：在发");
        for t in 1..20 {
            a.tick(t * TICK_MS, &mine);
        }
        assert!(!a.busy(), "发完了");
        let mut b = Transfer::default();
        b.announced(A, mine.clone(), |_| false, 0);
        assert!(b.busy(), "在收");
        b.clear();
        assert!(!b.busy());
    }

    #[test]
    fn senders_pace_and_only_serve_what_they_announce() {
        let still = test_webp(100, 100, STILL_MAX_BYTES);
        let h = hash_of(&still);
        let mut a = Transfer::default();
        a.on_want(&h, &[], &[], |_| Some(still.clone()));
        assert!(a.tick(100, &[]).is_empty(), "没报的不给");
        let mine = vec![h.clone()];
        a.on_want(&h, &[], &mine, |_| Some(still.clone()));
        a.on_want(&h, &[], &mine, |_| panic!("同一张只读一次"));
        assert_eq!(chunks(&a.tick(100, &mine)).len(), CHUNKS_PER_TICK);
        let mut total = CHUNKS_PER_TICK;
        for t in 2..20 {
            total += chunks(&a.tick(t * 100, &mine)).len();
        }
        assert_eq!(total, usize::from(CHUNKS_MAX), "两个人要也只发一份");
        // 补要某几块
        a.on_want(&h, &[3, 99], &mine, |_| Some(still.clone()));
        let again = chunks(&a.tick(5000, &mine));
        assert_eq!(again.iter().map(|c| c.1).collect::<Vec<_>>(), vec![3]);
        // 从轮盘上拿掉了：排着的不再发
        a.on_want(&h, &[], &mine, |_| Some(still.clone()));
        assert!(a.tick(5100, &[]).is_empty());
    }

    #[test]
    fn forged_or_unasked_chunks_are_dropped() {
        let still = test_webp(100, 100, 1000);
        let h = hash_of(&still);
        let mut b = Transfer::default();
        // 没人报过这个哈希
        assert!(
            b.on_chunk(A, &h, (0, 2), still[..900].to_vec(), |_| false, 0)
                .is_empty()
        );
        b.announced(A, vec![h.clone()], |_| false, 0);
        // 别人（B 没报过）发来的不收
        assert!(
            b.on_chunk(B, &h, (0, 2), still[..900].to_vec(), |_| false, 0)
                .is_empty()
        );
        // 内容被换了：拼起来哈希对不上
        let mut fake = still.clone();
        fake[500] ^= 1;
        b.on_chunk(A, &h, (0, 2), fake[..900].to_vec(), |_| false, 0);
        assert!(
            b.on_chunk(A, &h, (1, 2), fake[900..].to_vec(), |_| false, 0)
                .is_empty()
        );
        // 块数说法不一：整份丢掉
        b.announced(A, vec![h.clone()], |_| false, 10);
        b.on_chunk(A, &h, (0, 2), still[..900].to_vec(), |_| false, 10);
        assert!(
            b.on_chunk(A, &h, (1, 3), still[900..].to_vec(), |_| false, 10)
                .is_empty()
        );
        assert!(b.assemblies.is_empty());
        // 已经有了：迟到的分块不开新的
        assert!(
            b.on_chunk(A, &h, (0, 2), still[..900].to_vec(), |_| true, 20)
                .is_empty()
        );
        assert!(b.assemblies.is_empty());
        // 超大、超界的分块
        assert!(
            b.on_chunk(A, &h, (0, CHUNKS_MAX + 1), vec![0], |_| false, 20)
                .is_empty()
        );
        assert!(
            b.on_chunk(A, &h, (0, 1), vec![0; CHUNK_BYTES + 1], |_| false, 20)
                .is_empty()
        );
    }

    #[test]
    fn receivers_reask_then_give_up() {
        let h = "ab".repeat(16);
        let mut b = Transfer::default();
        b.announced(A, vec![h.clone()], |_| false, 0);
        assert!(b.tick(1000, &[]).is_empty());
        for k in 1..=REASKS_MAX {
            let out = b.tick(u64::from(k) * REASK_MS, &[]);
            assert_eq!(
                out,
                vec![Out::Want {
                    to: A,
                    hash: h.clone(),
                    need: vec![]
                }]
            );
        }
        b.tick(u64::from(REASKS_MAX + 1) * REASK_MS, &[]);
        assert!(b.assemblies.is_empty(), "要了 3 次还没有就放弃");
        // 收到部分后只要缺的
        let still = test_webp(100, 100, 2000);
        let h = hash_of(&still);
        b.announced(A, vec![h.clone()], |_| false, 0);
        b.on_chunk(A, &h, (1, 3), still[900..1800].to_vec(), |_| false, 0);
        assert_eq!(
            b.tick(REASK_MS, &[]),
            vec![Out::Want {
                to: A,
                hash: h.clone(),
                need: vec![0, 2]
            }]
        );
        // 不再报这个哈希、或那人走了：放弃
        b.announced(A, vec![], |_| false, 3100);
        b.tick(3200, &[]);
        assert!(b.assemblies.is_empty());
        b.announced(A, vec![h.clone()], |_| false, 4000);
        b.gone(&A);
        assert!(b.assemblies.is_empty());
    }

    #[test]
    fn at_most_sixteen_assemblies_at_once() {
        let mut b = Transfer::default();
        let hashes: Vec<String> = (0..20).map(|i| format!("{i:032x}")).collect();
        let wants = b.announced(A, hashes, |_| false, 0);
        assert_eq!(wants.len(), MAX_ASSEMBLIES);
        // 已经有的不要
        let mut c = Transfer::default();
        assert!(
            c.announced(A, vec!["ab".repeat(16)], |_| true, 0)
                .is_empty()
        );
    }
}
