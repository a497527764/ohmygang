//! 心之钢：命中事件的重放（2.0 架构 6.2）。纯函数：所有人用同一批事件、同一套规则算，层数和生命值一定一样。
//!
//! 实现流程：
//! 1. 同一作者同一 seq 的分叉只留 event_id 最小的一条；作者的时间严格递增，所以按 seq 排就是按时间排。
//! 2. 一条 Hit 有效要同时满足：
//!    - 作者和目标在事件时间都是队员（各自在这之前最后一条 Join / Leave 是 Join），目标不是自己；
//!    - 充能不少于 3 秒；
//!    - 作者对同一个目标的上一条有效 Hit 至少早 30 秒；
//!    - 不在队伍解散之后。
//! 3. 层数 = 作者的有效 Hit 数；被叠次数 = 以他为目标的有效 Hit 数。
//! 4. 生命值（整数运算）：初始 1500；每叠一层，伤害 = 70 + 当前生命值 × 6 / 100，生命值 += 伤害 / 10。
//!    数值仿照心之钢「伤害随最大生命值增长、每层得到伤害 10% 的永久生命值」。

use std::collections::{BTreeMap, HashMap};

use crate::crypto::{EventId, MemberId};
use crate::protocol::body::{self, HitBody};
use crate::protocol::event::{SignedEvent, kind};
use crate::rooms::state::dedup_forks;

/// 充能至少这么久，命中才算数。
pub const CHARGE_MS: u32 = 3000;
/// 同一个人对同一个目标，两次有效命中至少隔这么久。
pub const COOLDOWN_MS: u64 = 30_000;
pub const START_HP: u32 = 1500;

/// 一条有效命中。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidHit {
    pub event_id: EventId,
    pub author: MemberId,
    pub target: MemberId,
    pub ts_ms: u64,
}

/// 重放的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Board {
    /// 层数（只列有层数的人）。
    pub stacks: BTreeMap<MemberId, u32>,
    /// 被叠次数。
    pub taken: BTreeMap<MemberId, u32>,
    pub valid: Vec<ValidHit>,
}

impl Board {
    pub fn stacks_of(&self, m: &MemberId) -> u32 {
        self.stacks.get(m).copied().unwrap_or(0)
    }

    pub fn taken_of(&self, m: &MemberId) -> u32 {
        self.taken.get(m).copied().unwrap_or(0)
    }

    /// 某个目标上一次被 `author` 有效命中的时间（冷却用）。
    pub fn last_hit(&self, author: &MemberId, target: &MemberId) -> Option<u64> {
        self.valid
            .iter()
            .filter(|h| h.author == *author && h.target == *target)
            .map(|h| h.ts_ms)
            .max()
    }
}

/// 叠了 `stacks` 层以后的生命值。
pub fn hp_after(stacks: u32) -> u32 {
    let mut hp = START_HP;
    for _ in 0..stacks {
        let damage = 70 + hp * 6 / 100;
        hp += damage / 10;
    }
    hp
}

/// 每个人的进出队时间线：(ts, 进队?)，按时间排好。
fn membership(events: &[&SignedEvent]) -> HashMap<MemberId, Vec<(u64, bool)>> {
    let mut out: HashMap<MemberId, Vec<(u64, bool)>> = HashMap::new();
    for e in events {
        let joined = match e.event.kind {
            kind::JOIN => true,
            kind::LEAVE => false,
            _ => continue,
        };
        out.entry(e.event.author_id)
            .or_default()
            .push((e.event.ts_ms, joined));
    }
    for line in out.values_mut() {
        line.sort();
    }
    out
}

fn member_at(lines: &HashMap<MemberId, Vec<(u64, bool)>>, m: &MemberId, ts: u64) -> bool {
    lines
        .get(m)
        .and_then(|line| line.iter().rev().find(|(t, _)| *t <= ts))
        .is_some_and(|(_, joined)| *joined)
}

/// 用队伍的全部事件重放命中。`dissolve_ts` 是推导出的解散时间。
pub fn replay(events: &[SignedEvent], dissolve_ts: Option<u64>) -> Board {
    let slots = dedup_forks(events);
    let all: Vec<&SignedEvent> = slots.values().copied().collect();
    let lines = membership(&all);
    let mut board = Board::default();
    // (作者, 目标) → 上一次有效命中的时间
    let mut last: HashMap<(MemberId, MemberId), u64> = HashMap::new();
    // BTreeMap 按 (作者, seq) 升序：同一作者按时间先后
    for ((author, _), e) in &slots {
        if e.event.kind != kind::HIT {
            continue;
        }
        let Ok(hit) = body::decode::<HitBody>(&e.event.body) else {
            continue;
        };
        let ts = e.event.ts_ms;
        let ok = hit.target != *author
            && hit.charged_ms >= CHARGE_MS
            && dissolve_ts.is_none_or(|d| ts <= d)
            && member_at(&lines, author, ts)
            && member_at(&lines, &hit.target, ts)
            && last
                .get(&(*author, hit.target))
                .is_none_or(|prev| ts >= prev + COOLDOWN_MS);
        if !ok {
            continue;
        }
        last.insert((*author, hit.target), ts);
        *board.stacks.entry(*author).or_default() += 1;
        *board.taken.entry(hit.target).or_default() += 1;
        board.valid.push(ValidHit {
            event_id: e.event_id,
            author: *author,
            target: hit.target,
            ts_ms: ts,
        });
    }
    board
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Identity;
    use crate::protocol::body::encode;
    use crate::protocol::event::build_event;

    const ROOM: [u8; 16] = [1u8; 16];

    fn person(n: u8) -> Identity {
        Identity::from_secret(&[n; 32])
    }

    struct Author {
        id: Identity,
        seq: u64,
    }

    impl Author {
        fn new(n: u8) -> Self {
            Self {
                id: person(n),
                seq: 0,
            }
        }
        fn me(&self) -> MemberId {
            self.id.member_id()
        }
        fn ev(&mut self, ts: u64, k: u16, body: Vec<u8>) -> SignedEvent {
            self.seq += 1;
            build_event(&self.id, ROOM, self.seq, ts, k, body)
        }
        fn join(&mut self, ts: u64) -> SignedEvent {
            self.ev(ts, kind::JOIN, vec![])
        }
        fn leave(&mut self, ts: u64) -> SignedEvent {
            self.ev(ts, kind::LEAVE, vec![])
        }
        fn hit(&mut self, ts: u64, target: MemberId, charged_ms: u32) -> SignedEvent {
            self.ev(ts, kind::HIT, encode(&HitBody { target, charged_ms }))
        }
    }

    #[test]
    fn hp_grows_like_heartsteel() {
        assert_eq!(hp_after(0), 1500);
        // 第一层：伤害 70 + 90 = 160，生命值 +16
        assert_eq!(hp_after(1), 1516);
        // 第二层：伤害 70 + 1516*6/100 = 70 + 90 = 160 → +16
        assert_eq!(hp_after(2), 1532);
        assert!(hp_after(100) > hp_after(99));
        assert_eq!(hp_after(10), {
            let mut hp = 1500u32;
            for _ in 0..10 {
                hp += (70 + hp * 6 / 100) / 10;
            }
            hp
        });
    }

    #[test]
    fn valid_hits_stack_and_count_against_the_target() {
        let (mut a, mut b) = (Author::new(1), Author::new(2));
        let events = vec![
            a.join(1000),
            b.join(1000),
            a.hit(10_000, b.me(), 3000),
            b.hit(11_000, a.me(), 3500),
        ];
        let board = replay(&events, None);
        assert_eq!((board.stacks_of(&a.me()), board.stacks_of(&b.me())), (1, 1));
        assert_eq!((board.taken_of(&a.me()), board.taken_of(&b.me())), (1, 1));
        assert_eq!(board.valid.len(), 2);
    }

    #[test]
    fn short_charge_self_hits_and_strangers_do_not_count() {
        let (mut a, mut b, c) = (Author::new(1), Author::new(2), Author::new(3));
        let events = vec![
            a.join(1000),
            b.join(1000),
            a.hit(10_000, b.me(), CHARGE_MS - 1), // 充能不够
            a.hit(11_000, a.me(), 5000),          // 打自己
            a.hit(12_000, c.me(), 5000),          // 丙不是队员
        ];
        assert_eq!(replay(&events, None), Board::default());
    }

    #[test]
    fn cooldown_is_per_target() {
        let (mut a, mut b, mut c) = (Author::new(1), Author::new(2), Author::new(3));
        let events = vec![
            a.join(1000),
            b.join(1000),
            c.join(1000),
            a.hit(10_000, b.me(), 3000),
            a.hit(20_000, b.me(), 3000), // 冷却中
            a.hit(20_500, c.me(), 3000), // 别的目标不受影响（团战）
            a.hit(40_000, b.me(), 3000), // 冷却过了
        ];
        let board = replay(&events, None);
        assert_eq!(board.stacks_of(&a.me()), 3);
        assert_eq!((board.taken_of(&b.me()), board.taken_of(&c.me())), (2, 1));
        assert_eq!(board.last_hit(&a.me(), &b.me()), Some(40_000));
        // 冷却按上一次「有效」命中算：被挡掉的那次不重新计时
        let (mut a, mut b) = (Author::new(1), Author::new(2));
        let events = vec![
            a.join(1000),
            b.join(1000),
            a.hit(10_000, b.me(), 3000),
            a.hit(39_999, b.me(), 3000),
            a.hit(40_000, b.me(), 3000),
        ];
        assert_eq!(replay(&events, None).stacks_of(&a.me()), 2);
    }

    #[test]
    fn membership_is_checked_at_the_time_of_the_hit() {
        let (mut a, mut b) = (Author::new(1), Author::new(2));
        let events = vec![
            a.join(1000),
            a.hit(5000, b.me(), 3000), // 乙还没进队
            b.join(6000),
            a.hit(7000, b.me(), 3000),
            b.leave(8000),
            a.hit(40_000, b.me(), 3000), // 乙已经走了
        ];
        assert_eq!(replay(&events, None).stacks_of(&a.me()), 1);
    }

    #[test]
    fn hits_after_dissolve_do_not_count() {
        let (mut a, mut b) = (Author::new(1), Author::new(2));
        let events = vec![
            a.join(1000),
            b.join(1000),
            a.hit(10_000, b.me(), 3000),
            a.hit(50_000, b.me(), 3000),
        ];
        assert_eq!(replay(&events, Some(20_000)).stacks_of(&a.me()), 1);
    }

    #[test]
    fn arrival_order_and_duplicates_do_not_matter() {
        let (mut a, mut b) = (Author::new(1), Author::new(2));
        let mut events = vec![
            a.join(1000),
            b.join(1000),
            a.hit(10_000, b.me(), 3000),
            b.hit(12_000, a.me(), 3000),
            a.hit(45_000, b.me(), 4000),
        ];
        let board = replay(&events, None);
        events.reverse();
        assert_eq!(replay(&events, None), board);
        events.extend(events.clone());
        assert_eq!(replay(&events, None), board);
    }
}
