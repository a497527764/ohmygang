//! 同步决策，全部是纯函数，不碰网络和数据库。2.0 只有状态事件，去掉了 1.x 的聊天补齐分界。
//!
//! 实现流程：
//! 1. `Holdings`：本机每位作者持有哪些序号。Have 里报「从 1 起连续持有到的最大序号」，
//!    有空洞时只报到空洞之前，避免同伴来要本机其实没有的事件。
//! 2. `state_gaps`：对比同伴的 Have，找出对方有、本机缺的事件区间，一次最多要 64 个。
//! 3. `dominates`：别人的 Have 已覆盖我的全部序号，我待发的 Have 就可以取消（抑制），避免所有人同时回应。
//! 4. `ready_to_activate`：已拿到能对上 owner_tag 的建队事件、至少见过一个同伴、
//!    本机的事件不少于任何一个同伴报的数，才从 Joining 转 Active。

use std::collections::{BTreeMap, BTreeSet};

use crate::protocol::frame::{HaveEntry, ShortId, WantRange};

pub const MAX_WANT_EVENTS: u64 = 64;

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

/// 本机持有的序号：作者短 ID → 序号集合。
#[derive(Clone, Debug, Default)]
pub struct Holdings {
    by_author: BTreeMap<ShortId, BTreeSet<u64>>,
}

impl Holdings {
    pub fn add(&mut self, author: ShortId, seq: u64) {
        self.by_author.entry(author).or_default().insert(seq);
    }

    pub fn has(&self, author: &ShortId, seq: u64) -> bool {
        self.by_author.get(author).is_some_and(|s| s.contains(&seq))
    }

    pub fn contiguous(&self, author: &ShortId) -> u64 {
        self.by_author.get(author).map_or(0, contiguous_max)
    }

    /// 本机在 [1, up_to] 里实际持有多少个序号。遍历的是本机持有的集合，
    /// 不是 up_to 本身——对方报一个天文数字的 max 也不会把这里拖死。
    pub fn count_within(&self, author: &ShortId, up_to: u64) -> u64 {
        self.by_author
            .get(author)
            .map_or(0, |s| s.range(1..=up_to).count() as u64)
    }

    pub fn vector(&self) -> Vec<HaveEntry> {
        self.by_author
            .iter()
            .map(|(a, s)| HaveEntry {
                author: *a,
                max: contiguous_max(s),
            })
            .collect()
    }
}

/// 对方有、本机缺的事件区间：每位作者在 [1, 对方最大值] 里找，按对方向量的顺序排，合计不超过 64 个。
pub fn state_gaps(mine: &Holdings, peer: &[HaveEntry]) -> Vec<WantRange> {
    let mut out = Vec::new();
    let mut budget = MAX_WANT_EVENTS;
    for e in peer {
        let range = |from_seq, to_seq| WantRange {
            author: e.author,
            from_seq,
            to_seq,
        };
        let mut start: Option<u64> = None;
        let mut seq = 1;
        while seq <= e.max && budget > 0 {
            if mine.has(&e.author, seq) {
                if let Some(s) = start.take() {
                    out.push(range(s, seq - 1));
                }
            } else {
                start.get_or_insert(seq);
                budget -= 1;
            }
            seq += 1;
        }
        if let Some(s) = start {
            out.push(range(s, seq - 1));
        }
        if budget == 0 {
            break;
        }
    }
    out
}

/// 对方有、本机缺的事件总数，不受 64 的限制（判断在途的 Want 有没有进展用）。
/// 算法：[1, max] 共 max 个槽位，减去本机在范围内的持有数。不逐个遍历序号，
/// 恶意或异常的超大 max（比如 u64 上限）不会拖死接收方。
pub fn missing_total(mine: &Holdings, peer: &[HaveEntry]) -> u64 {
    peer.iter()
        .map(|e| e.max.saturating_sub(mine.count_within(&e.author, e.max)))
        .sum()
}

/// 对方的向量是否覆盖了我的全部序号（我为 0 的项不用覆盖）。
pub fn dominates(theirs: &[HaveEntry], mine: &[HaveEntry]) -> bool {
    mine.iter().all(|m| {
        m.max == 0
            || theirs
                .iter()
                .any(|t| t.author == m.author && t.max >= m.max)
    })
}

pub fn ready_to_activate(owner_known: bool, mine: &Holdings, peers: &[Vec<HaveEntry>]) -> bool {
    owner_known
        && !peers.is_empty()
        && peers
            .iter()
            .all(|v| v.iter().all(|e| mine.contiguous(&e.author) >= e.max))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ShortId = [1u8; 8];
    const B: ShortId = [2u8; 8];

    fn holdings(entries: &[(ShortId, &[u64])]) -> Holdings {
        let mut h = Holdings::default();
        for (author, seqs) in entries {
            for s in *seqs {
                h.add(*author, *s);
            }
        }
        h
    }

    fn have(author: ShortId, max: u64) -> HaveEntry {
        HaveEntry { author, max }
    }

    fn range(author: ShortId, from_seq: u64, to_seq: u64) -> WantRange {
        WantRange {
            author,
            from_seq,
            to_seq,
        }
    }

    #[test]
    fn vector_reports_contiguous_max_only() {
        let h = holdings(&[(A, &[1, 2, 4]), (B, &[2])]);
        assert_eq!(h.vector(), vec![have(A, 2), have(B, 0)]);
    }

    #[test]
    fn gaps_fill_holes_and_tail() {
        let mine = holdings(&[(A, &[1, 2, 4])]);
        assert_eq!(
            state_gaps(&mine, &[have(A, 5)]),
            vec![range(A, 3, 3), range(A, 5, 5)]
        );
        assert!(
            state_gaps(&mine, &[have(A, 1)]).is_empty(),
            "对方比我少：没有缺口"
        );
        assert_eq!(
            state_gaps(&mine, &[have(B, 3)]),
            vec![range(B, 1, 3)],
            "没有的作者从 1 要"
        );
    }

    #[test]
    fn gaps_are_capped_at_64_events() {
        let gaps = state_gaps(&Holdings::default(), &[have(A, 50), have(B, 50)]);
        let total: u64 = gaps.iter().map(|r| r.to_seq - r.from_seq + 1).sum();
        assert_eq!(total, MAX_WANT_EVENTS);
        assert_eq!(gaps, vec![range(A, 1, 50), range(B, 1, 14)]);
    }

    #[test]
    fn missing_total_is_not_capped() {
        let mine = holdings(&[(A, &[1])]);
        assert_eq!(missing_total(&mine, &[have(A, 100)]), 99);
    }

    #[test]
    fn missing_total_survives_absurd_max() {
        // 恶意节点报一个天文数字的 max：不能逐序号遍历（会卡死），数数就得出结果
        let mine = holdings(&[(A, &[1, 2, 3])]);
        assert_eq!(
            missing_total(&mine, &[have(A, u64::MAX - 3)]),
            u64::MAX - 3 - 3
        );
        // 超过本机持有的部分也按持有数扣减，不会溢出
        assert_eq!(
            missing_total(&Holdings::default(), &[have(A, u64::MAX)]),
            u64::MAX
        );
    }

    #[test]
    fn dominates_means_they_have_everything_i_have() {
        let mine = vec![have(A, 3), have(B, 0)];
        assert!(dominates(&[have(A, 3)], &mine));
        assert!(dominates(&[have(A, 4), have(B, 1)], &mine));
        assert!(!dominates(&[have(A, 2)], &mine));
        assert!(!dominates(&[have(B, 9)], &mine));
    }

    #[test]
    fn activation_needs_owner_a_peer_and_no_gaps() {
        let mine = holdings(&[(A, &[1, 2, 3])]);
        let peers = vec![vec![have(A, 3)]];
        assert!(
            !ready_to_activate(false, &mine, &peers),
            "还没拿到能对上的建队事件"
        );
        assert!(!ready_to_activate(true, &mine, &[]), "还没见到同伴");
        assert!(
            !ready_to_activate(true, &mine, &[vec![have(A, 4)]]),
            "还有缺口"
        );
        assert!(ready_to_activate(true, &mine, &peers));
    }
}
