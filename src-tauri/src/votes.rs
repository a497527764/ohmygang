//! 队伍投票（2.0 架构第 9 节）：投降、准备确认、踢人。规则移植自 lolPing `src/shared/voteTally.ts`（MIT，© 2026 Danrui Meng）。
//!
//! 实现流程：
//! 1. 同一时间只有一个投票。发起时定下投票人：本人加上此刻在线、没关投票的队友；踢人时被踢的人不在其中。发起人自动算同意。
//! 2. 每人第一次的回答算数，之后改不了；投票人离开或掉线，没回答的算反对。
//! 3. 需要的同意票：投降 70%（向上取整），准备确认要所有人，踢人要过半。结果一定下来（够了，或者剩下的人全同意也不够）就结束；
//!    到时间（投降、踢人 60 秒，准备确认 15 秒）还没定，没回答的算反对。时长是本机的常数，不听发起人的。
//! 4. 纯逻辑，时间由调用方传入。

use crate::crypto::MemberId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoteKind {
    /// 投降。
    Ff,
    /// 准备确认。
    Ready,
    Kick,
}

impl VoteKind {
    pub fn duration_ms(self) -> u64 {
        match self {
            VoteKind::Ff | VoteKind::Kick => 60_000,
            VoteKind::Ready => 15_000,
        }
    }

    /// 线上的编号。
    pub fn code(self) -> u8 {
        match self {
            VoteKind::Ff => 0,
            VoteKind::Ready => 1,
            VoteKind::Kick => 2,
        }
    }

    pub fn from_code(c: u8) -> Option<Self> {
        match c {
            0 => Some(VoteKind::Ff),
            1 => Some(VoteKind::Ready),
            2 => Some(VoteKind::Kick),
            _ => None,
        }
    }
}

/// 要多少张同意票（见文件头第 3 条）。
pub fn needed(kind: VoteKind, voters: usize) -> usize {
    match kind {
        VoteKind::Ff => (voters * 7).div_ceil(10),
        VoteKind::Ready => voters,
        VoteKind::Kick => voters / 2 + 1,
    }
}

/// 开着的投票。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    pub id: u32,
    pub kind: VoteKind,
    pub starter: MemberId,
    pub target: Option<MemberId>,
    /// 发起时定下，被踢的人不在其中。
    pub voters: Vec<MemberId>,
    /// 回答过的人：同意或反对。
    pub ballots: Vec<(MemberId, bool)>,
    pub ends_ms: u64,
}

impl Open {
    pub fn ballot_of(&self, m: &MemberId) -> Option<bool> {
        self.ballots.iter().find(|(v, _)| v == m).map(|(_, y)| *y)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub id: u32,
    pub kind: VoteKind,
    pub target: Option<MemberId>,
    pub passed: bool,
    pub yes: usize,
    pub voters: usize,
    /// 反对的或走了的。
    pub declined: Vec<MemberId>,
    /// 一直没回答的。
    pub missing: Vec<MemberId>,
}

#[derive(Default)]
pub struct Tally {
    open: Option<Open>,
}

impl Tally {
    /// 开一个投票；已经有开着的就返回 false。
    pub fn start(
        &mut self,
        id: u32,
        kind: VoteKind,
        starter: MemberId,
        target: Option<MemberId>,
        voters: &[MemberId],
        now_ms: u64,
    ) -> bool {
        if self.open.is_some() {
            return false;
        }
        let mut list: Vec<MemberId> = Vec::new();
        for v in voters {
            if Some(*v) != target && !list.contains(v) {
                list.push(*v);
            }
        }
        let ballots = if list.contains(&starter) {
            vec![(starter, true)]
        } else {
            Vec::new()
        };
        self.open = Some(Open {
            id,
            kind,
            starter,
            target,
            voters: list,
            ballots,
            ends_ms: now_ms + kind.duration_ms(),
        });
        true
    }

    /// 一张票：要是这个投票的投票人、第一次回答。收下了返回 true。
    pub fn ballot(&mut self, voter: MemberId, id: u32, yes: bool) -> bool {
        let Some(o) = self.open.as_mut() else {
            return false;
        };
        if o.id != id || !o.voters.contains(&voter) || o.ballot_of(&voter).is_some() {
            return false;
        }
        o.ballots.push((voter, yes));
        true
    }

    /// 投票人走了：没回答的算反对。
    pub fn left(&mut self, voter: &MemberId) {
        if let Some(o) = self.open.as_mut()
            && o.voters.contains(voter)
            && o.ballot_of(voter).is_none()
        {
            o.ballots.push((*voter, false));
        }
    }

    pub fn view(&self) -> Option<&Open> {
        self.open.as_ref()
    }

    /// 不出结果地丢掉（退出队伍了）。
    pub fn clear(&mut self) {
        self.open = None;
    }

    /// 结果定下来了就返回结果（只返回一次），否则 None。
    pub fn check(&mut self, now_ms: u64) -> Option<Outcome> {
        let o = self.open.as_ref()?;
        let yes = o.ballots.iter().filter(|(_, y)| *y).count();
        let missing: Vec<MemberId> = o
            .voters
            .iter()
            .filter(|v| o.ballot_of(v).is_none())
            .copied()
            .collect();
        let need = needed(o.kind, o.voters.len());
        let passed = yes >= need;
        if !passed && yes + missing.len() >= need && now_ms < o.ends_ms {
            return None;
        }
        let o = self.open.take()?;
        Some(Outcome {
            id: o.id,
            kind: o.kind,
            target: o.target,
            passed,
            yes,
            voters: o.voters.len(),
            declined: o
                .ballots
                .iter()
                .filter(|(_, y)| !*y)
                .map(|(v, _)| *v)
                .collect(),
            missing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(i: u8) -> MemberId {
        [i; 32]
    }

    #[test]
    fn thresholds() {
        assert_eq!(needed(VoteKind::Ff, 5), 4); // 3.5 向上取整
        assert_eq!(needed(VoteKind::Ff, 3), 3); // 2.1 向上取整
        assert_eq!(needed(VoteKind::Ff, 10), 7);
        assert_eq!(needed(VoteKind::Ready, 4), 4);
        assert_eq!(needed(VoteKind::Kick, 4), 3);
        assert_eq!(needed(VoteKind::Kick, 3), 2);
        for k in [VoteKind::Ff, VoteKind::Ready, VoteKind::Kick] {
            assert_eq!(VoteKind::from_code(k.code()), Some(k));
        }
        assert_eq!(VoteKind::from_code(9), None);
    }

    #[test]
    fn surrender_passes_once_enough_say_yes() {
        let mut t = Tally::default();
        let all = [m(1), m(2), m(3), m(4), m(5)];
        assert!(t.start(7, VoteKind::Ff, m(1), None, &all, 0));
        assert!(
            !t.start(8, VoteKind::Ready, m(2), None, &all, 0),
            "同时只有一个"
        );
        assert_eq!(
            t.view().unwrap().ballot_of(&m(1)),
            Some(true),
            "发起人算同意"
        );
        for v in [m(2), m(3)] {
            assert!(t.ballot(v, 7, true));
            assert_eq!(t.check(1000), None);
        }
        assert!(!t.ballot(m(2), 7, false), "第一次回答算数");
        assert!(!t.ballot(m(9), 7, true), "不是投票人");
        assert!(!t.ballot(m(4), 99, true), "别的投票");
        assert!(t.ballot(m(4), 7, true));
        let o = t.check(1000).unwrap();
        assert!(o.passed && o.yes == 4 && o.voters == 5);
        assert_eq!(o.missing, vec![m(5)]);
        assert_eq!(t.check(1000), None, "结果只给一次");
        assert!(t.view().is_none());
    }

    #[test]
    fn a_vote_fails_as_soon_as_it_cannot_pass() {
        let mut t = Tally::default();
        t.start(1, VoteKind::Ff, m(1), None, &[m(1), m(2), m(3)], 0);
        t.ballot(m(2), 1, false);
        let o = t.check(10).unwrap();
        assert!(!o.passed);
        assert_eq!(o.declined, vec![m(2)]);
        // 准备确认：有人走了就不成
        t.start(2, VoteKind::Ready, m(1), None, &[m(1), m(2), m(3)], 0);
        t.ballot(m(2), 2, true);
        assert_eq!(t.check(10), None);
        t.left(&m(3));
        let o = t.check(10).unwrap();
        assert!(!o.passed && o.declined == vec![m(3)]);
    }

    #[test]
    fn time_runs_out_with_the_silent_counted_as_no() {
        let mut t = Tally::default();
        t.start(3, VoteKind::Ready, m(1), None, &[m(1), m(2)], 1000);
        assert_eq!(t.check(1000 + 14_999), None);
        let o = t.check(1000 + 15_000).unwrap();
        assert!(!o.passed && o.missing == vec![m(2)]);
    }

    #[test]
    fn kick_target_cannot_vote_and_majority_decides() {
        let mut t = Tally::default();
        t.start(
            4,
            VoteKind::Kick,
            m(1),
            Some(m(4)),
            &[m(1), m(2), m(3), m(4), m(1)],
            0,
        );
        let o = t.view().unwrap();
        assert_eq!(
            o.voters,
            vec![m(1), m(2), m(3)],
            "被踢的不在其中，重复的去掉"
        );
        assert!(!t.ballot(m(4), 4, false));
        assert!(t.ballot(m(3), 4, true));
        let o = t.check(5).unwrap();
        assert!(o.passed && o.target == Some(m(4)) && o.yes == 2);
    }

    #[test]
    fn a_starter_who_is_not_a_voter_does_not_count() {
        let mut t = Tally::default();
        t.start(5, VoteKind::Ff, m(9), None, &[m(1), m(2)], 0);
        assert!(t.view().unwrap().ballots.is_empty());
        t.clear();
        assert!(t.view().is_none());
    }
}
