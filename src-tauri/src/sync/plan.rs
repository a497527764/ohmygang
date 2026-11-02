//! 同步决策（架构 7.2–7.4），全部是纯函数，不碰网络和数据库。
//!
//! 实现流程：
//! 1. `Holdings`：本机每位作者、每个 stream 持有哪些序号。Have 里报「从 1 起连续持有到的最大序号」，
//!    有空洞时只报到空洞之前，避免同伴来要本机其实没有的事件。
//!    聊天例外：新成员只补最近 n 条，分界（chat_floor）之前的不需要有，所以从分界起算。
//!    否则新成员报的聊天序号永远是 0，作者的消息一直显示「发送中」，别人也不会向他要他其实有的聊天。
//! 2. `state_gaps`：对比同伴的 Have，找出对方有、本机缺的状态事件区间，一次最多要 64 个。
//!    `chat_gaps`：聊天同理，但每位作者从分界（chat_floor，新成员只要最近 n 条）开始，没有分界的从 1 开始。
//! 3. `dominates`：别人的 Have 已覆盖我的全部序号，我待发的 Have 就可以取消（抑制），避免所有人同时回应。
//! 4. `ready_to_activate`：已拿到能对上 owner_tag 的建房事件、至少见过一个同伴、
//!    本机的状态事件不少于任何一个同伴报的数，且本人自己的聊天也不少于同伴报的（恢复备份后，
//!    否则新消息的序号会和同伴那里的旧消息重复），才从 Joining 转 Active。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::protocol::frame::{HaveEntry, ShortId, WantRange};

pub const MAX_WANT_EVENTS: u64 = 64;

fn idx(stream: u8) -> usize {
    if stream == 0 { 0 } else { 1 }
}

fn contiguous_max(seqs: &BTreeSet<u64>) -> u64 {
    contiguous_from(seqs, 0)
}

/// 把 1..=held_below 当作已有，从那里起连续持有到的最大序号。
fn contiguous_from(seqs: &BTreeSet<u64>, held_below: u64) -> u64 {
    let mut n = held_below;
    for &s in seqs.range(held_below + 1..) {
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
    chat_floors: BTreeMap<ShortId, u64>,
}

impl Holdings {
    pub fn add(&mut self, author: ShortId, stream: u8, seq: u64) {
        self.by_author.entry(author).or_default()[idx(stream)].insert(seq);
    }

    pub fn set_chat_floor(&mut self, author: ShortId, floor: u64) {
        self.chat_floors.insert(author, floor);
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
                chat_max: contiguous_from(
                    &s[1],
                    self.chat_floors.get(a).map_or(0, |f| f.saturating_sub(1)),
                ),
            })
            .collect()
    }
}

/// 对方有、本机缺的事件区间：每位作者在 [起点, 对方最大值] 里找，按对方向量的顺序排，合计不超过 64 个。
fn gaps(
    mine: &Holdings,
    peer: &[HaveEntry],
    stream: u8,
    start_of: impl Fn(&ShortId) -> u64,
    max_of: impl Fn(&HaveEntry) -> u64,
) -> Vec<WantRange> {
    let mut out = Vec::new();
    let mut budget = MAX_WANT_EVENTS;
    for e in peer {
        let range = |from_seq, to_seq| WantRange {
            author: e.author,
            stream,
            from_seq,
            to_seq,
        };
        let mut start: Option<u64> = None;
        let mut seq = start_of(&e.author).max(1);
        while seq <= max_of(e) && budget > 0 {
            if mine.has(&e.author, stream, seq) {
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

/// 对方有、本机缺的状态事件，合计不超过 64 个。状态事件永远从 1 开始要齐。
pub fn state_gaps(mine: &Holdings, peer: &[HaveEntry]) -> Vec<WantRange> {
    gaps(mine, peer, 0, |_| 1, |e| e.state_max)
}

/// 对方有、本机缺的聊天，每位作者从分界开始（没有分界的从 1 开始），合计不超过 64 个。
pub fn chat_gaps(
    mine: &Holdings,
    peer: &[HaveEntry],
    floors: &HashMap<ShortId, u64>,
) -> Vec<WantRange> {
    gaps(
        mine,
        peer,
        1,
        |a| floors.get(a).copied().unwrap_or(1),
        |e| e.chat_max,
    )
}

/// 对方有、本机缺的事件总数，不受 64 的限制（判断在途的 Want 有没有进展用）。
/// `floors` 为 None 时只数状态事件。
pub fn missing_total(
    mine: &Holdings,
    peer: &[HaveEntry],
    floors: Option<&HashMap<ShortId, u64>>,
) -> u64 {
    let count = |author: &ShortId, stream: u8, from: u64, to: u64| {
        (from.max(1)..=to)
            .filter(|s| !mine.has(author, stream, *s))
            .count() as u64
    };
    peer.iter()
        .map(|e| {
            let state = count(&e.author, 0, 1, e.state_max);
            let chat = floors.map_or(0, |f| {
                count(
                    &e.author,
                    1,
                    f.get(&e.author).copied().unwrap_or(1),
                    e.chat_max,
                )
            });
            state + chat
        })
        .sum()
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

pub fn ready_to_activate(
    owner_known: bool,
    mine: &Holdings,
    peers: &[Vec<HaveEntry>],
    me: ShortId,
) -> bool {
    let my_chat = mine
        .vector()
        .iter()
        .find(|e| e.author == me)
        .map_or(0, |e| e.chat_max);
    owner_known
        && !peers.is_empty()
        && peers.iter().all(|v| {
            v.iter().all(|e| {
                mine.contiguous(&e.author, 0) >= e.state_max
                    && (e.author != me || my_chat >= e.chat_max)
            })
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

    fn chat_range(author: ShortId, from_seq: u64, to_seq: u64) -> WantRange {
        WantRange {
            author,
            stream: 1,
            from_seq,
            to_seq,
        }
    }

    #[test]
    fn vector_counts_chat_from_the_floor() {
        let mut h = holdings(&[(A, &[1], &[8, 9, 10])]);
        assert_eq!(h.vector(), vec![have(A, 1, 0)]);
        h.set_chat_floor(A, 8);
        assert_eq!(h.vector(), vec![have(A, 1, 10)], "分界之前的不用有");
        h.set_chat_floor(B, 5);
        assert_eq!(h.vector(), vec![have(A, 1, 10)], "一条都没有的作者不出现");
        h.add(A, 1, 12);
        assert_eq!(h.vector(), vec![have(A, 1, 10)], "空洞之后的不算");
    }

    #[test]
    fn chat_gaps_start_at_the_floor() {
        let mine = holdings(&[(A, &[], &[1, 2, 6])]);
        let floors = HashMap::from([(A, 5)]);
        assert_eq!(
            chat_gaps(&mine, &[have(A, 0, 8)], &floors),
            vec![chat_range(A, 5, 5), chat_range(A, 7, 8)],
            "分界之前的 3、4 不要"
        );
        // 没有分界的作者从 1 开始
        assert_eq!(
            chat_gaps(&mine, &[have(B, 0, 2)], &floors),
            vec![chat_range(B, 1, 2)]
        );
        // 分界在对方最大值之后：什么都不要
        let high = HashMap::from([(A, 9)]);
        assert!(chat_gaps(&mine, &[have(A, 0, 8)], &high).is_empty());
    }

    #[test]
    fn missing_total_is_not_capped() {
        let mine = holdings(&[(A, &[1], &[])]);
        let peer = [have(A, 100, 200)];
        assert_eq!(missing_total(&mine, &peer, None), 99);
        let floors = HashMap::from([(A, 151)]);
        assert_eq!(missing_total(&mine, &peer, Some(&floors)), 99 + 50);
    }

    #[test]
    fn chat_gaps_are_capped_at_64_events() {
        let gaps = chat_gaps(&Holdings::default(), &[have(A, 9, 100)], &HashMap::new());
        assert_eq!(gaps, vec![chat_range(A, 1, 64)]);
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
            !ready_to_activate(false, &mine, &peers, OTHER),
            "还没拿到能对上的建房事件"
        );
        assert!(!ready_to_activate(true, &mine, &[], OTHER), "还没见到同伴");
        assert!(
            !ready_to_activate(true, &mine, &[vec![have(A, 4, 0)]], OTHER),
            "状态事件还有缺口"
        );
        assert!(
            ready_to_activate(true, &mine, &peers, OTHER),
            "别人的聊天落后不影响激活"
        );
    }

    const OTHER: ShortId = [9u8; 8];

    #[test]
    fn activation_waits_for_my_own_latest_chat() {
        // 恢复备份后：同伴那里有我更新的聊天，本机要先补回来，否则新消息的序号会和旧的重复
        let mine = holdings(&[(A, &[1, 2], &[1, 2])]);
        assert!(ready_to_activate(true, &mine, &[vec![have(A, 2, 2)]], A));
        assert!(!ready_to_activate(true, &mine, &[vec![have(A, 2, 5)]], A));
        assert!(ready_to_activate(
            true,
            &mine,
            &[vec![have(A, 2, 2), have(B, 0, 9)]],
            A
        ));
    }
}
