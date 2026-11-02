//! 同步决策（架构 7.2–7.4），全部是纯函数，不碰网络和数据库。
//!
//! 实现流程：
//! 1. `Holdings`：本机每位作者、每个 stream 持有哪些序号。Have 里报「从 1 起连续持有到的最大序号」，
//!    有空洞时只报到空洞之前，避免同伴来要本机其实没有的事件。
//! 2. `state_gaps`：对比同伴的 Have，找出对方有、本机缺的状态事件区间，一次最多要 64 个。聊天补齐在 M2。
//! 3. `dominates`：别人的 Have 已覆盖我的全部序号，我待发的 Have 就可以取消（抑制），避免所有人同时回应。
//! 4. `ready_to_activate`：已拿到能对上 owner_tag 的建房事件、至少见过一个同伴、
//!    且本机的状态事件不少于任何一个同伴报的数，才从 Joining 转 Active。

use std::collections::{BTreeMap, BTreeSet};

use crate::protocol::frame::{HaveEntry, ShortId, WantRange};

pub const MAX_WANT_EVENTS: u64 = 64;

fn idx(stream: u8) -> usize {
    if stream == 0 { 0 } else { 1 }
}

fn contiguous_max(seqs: &BTreeSet<u64>) -> u64 {
    let mut n = 0;
    for &s in seqs {
        if s == n + 1 {
            n = s;
        } else if s > n + 1 {
            break;
        }
    }
    n
}

/// 本机持有的序号：作者短 ID → [状态流, 聊天流] 的序号集合。
#[derive(Clone, Debug, Default)]
pub struct Holdings {
    by_author: BTreeMap<ShortId, [BTreeSet<u64>; 2]>,
}

impl Holdings {
    pub fn add(&mut self, author: ShortId, stream: u8, seq: u64) {
        self.by_author.entry(author).or_default()[idx(stream)].insert(seq);
    }

    pub fn has(&self, author: &ShortId, stream: u8, seq: u64) -> bool {
        self.by_author
            .get(author)
            .is_some_and(|s| s[idx(stream)].contains(&seq))
    }

    pub fn contiguous(&self, author: &ShortId, stream: u8) -> u64 {
        self.by_author
            .get(author)
            .map_or(0, |s| contiguous_max(&s[idx(stream)]))
    }

    pub fn vector(&self) -> Vec<HaveEntry> {
        self.by_author
            .iter()
            .map(|(a, s)| HaveEntry {
                author: *a,
                state_max: contiguous_max(&s[0]),
                chat_max: contiguous_max(&s[1]),
            })
            .collect()
    }
}

/// 对方有、本机缺的状态事件，按对方向量的顺序排，合计不超过 64 个。
pub fn state_gaps(mine: &Holdings, peer: &[HaveEntry]) -> Vec<WantRange> {
    let mut out = Vec::new();
    let mut budget = MAX_WANT_EVENTS;
    for e in peer {
        let mut start: Option<u64> = None;
        let mut seq = 1;
        while seq <= e.state_max && budget > 0 {
            if mine.has(&e.author, 0, seq) {
                if let Some(s) = start.take() {
                    out.push(WantRange {
                        author: e.author,
                        stream: 0,
                        from_seq: s,
                        to_seq: seq - 1,
                    });
                }
            } else {
                start.get_or_insert(seq);
                budget -= 1;
            }
            seq += 1;
        }
        if let Some(s) = start {
            out.push(WantRange {
                author: e.author,
                stream: 0,
                from_seq: s,
                to_seq: seq - 1,
            });
        }
        if budget == 0 {
            break;
        }
    }
    out
}

/// 对方的向量是否覆盖了我的全部序号（我为 0 的项不用覆盖）。
pub fn dominates(theirs: &[HaveEntry], mine: &[HaveEntry]) -> bool {
    mine.iter().all(|m| {
        (m.state_max == 0 && m.chat_max == 0)
            || theirs.iter().any(|t| {
                t.author == m.author && t.state_max >= m.state_max && t.chat_max >= m.chat_max
            })
    })
}

pub fn ready_to_activate(owner_known: bool, mine: &Holdings, peers: &[Vec<HaveEntry>]) -> bool {
    owner_known
        && !peers.is_empty()
        && peers.iter().all(|v| {
            v.iter()
                .all(|e| mine.contiguous(&e.author, 0) >= e.state_max)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ShortId = [1u8; 8];
    const B: ShortId = [2u8; 8];

    fn holdings(entries: &[(ShortId, &[u64], &[u64])]) -> Holdings {
        let mut h = Holdings::default();
        for (author, state, chat) in entries {
            for s in *state {
                h.add(*author, 0, *s);
            }
            for c in *chat {
                h.add(*author, 1, *c);
            }
        }
        h
    }

    fn have(author: ShortId, state_max: u64, chat_max: u64) -> HaveEntry {
        HaveEntry {
            author,
            state_max,
            chat_max,
        }
    }

    fn range(author: ShortId, from_seq: u64, to_seq: u64) -> WantRange {
        WantRange {
            author,
            stream: 0,
            from_seq,
            to_seq,
        }
    }

    #[test]
    fn vector_reports_contiguous_max_only() {
        let h = holdings(&[(A, &[1, 2, 4], &[1, 2, 3]), (B, &[2], &[])]);
        assert_eq!(h.vector(), vec![have(A, 2, 3), have(B, 0, 0)]);
    }

    #[test]
    fn state_gaps_fill_holes_and_tail() {
        let mine = holdings(&[(A, &[1, 2, 4], &[])]);
        assert_eq!(
            state_gaps(&mine, &[have(A, 5, 0)]),
            vec![range(A, 3, 3), range(A, 5, 5)]
        );
        // 对方比我少：没有缺口
        assert!(state_gaps(&mine, &[have(A, 1, 0)]).is_empty());
        // 我完全没有的作者：从 1 要到对方的最大值
        assert_eq!(state_gaps(&mine, &[have(B, 3, 9)]), vec![range(B, 1, 3)]);
    }

    #[test]
    fn state_gaps_are_capped_at_64_events() {
        let mine = Holdings::default();
        let gaps = state_gaps(&mine, &[have(A, 50, 0), have(B, 50, 0)]);
        let total: u64 = gaps.iter().map(|r| r.to_seq - r.from_seq + 1).sum();
        assert_eq!(total, MAX_WANT_EVENTS);
        assert_eq!(gaps, vec![range(A, 1, 50), range(B, 1, 14)]);
    }

    #[test]
    fn dominates_means_they_have_everything_i_have() {
        let mine = vec![have(A, 3, 5), have(B, 0, 0)];
        assert!(dominates(&[have(A, 3, 5)], &mine));
        assert!(dominates(&[have(A, 4, 9), have(B, 1, 1)], &mine));
        assert!(!dominates(&[have(A, 3, 4)], &mine));
        assert!(!dominates(&[have(B, 9, 9)], &mine));
    }

    #[test]
    fn activation_needs_owner_a_peer_and_no_state_gaps() {
        let mine = holdings(&[(A, &[1, 2, 3], &[])]);
        let peers = vec![vec![have(A, 3, 10)]];
        assert!(
            !ready_to_activate(false, &mine, &peers),
            "还没拿到能对上的建房事件"
        );
        assert!(!ready_to_activate(true, &mine, &[]), "还没见到同伴");
        assert!(
            !ready_to_activate(true, &mine, &[vec![have(A, 4, 0)]]),
            "状态事件还有缺口"
        );
        assert!(ready_to_activate(true, &mine, &peers), "聊天落后不影响激活");
    }
}
