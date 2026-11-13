//! 一个人一段话的抖动缓冲：把乱序、成批到达的 Opus 小帧排好，每 20 毫秒交出一个。
//!
//! 实现流程：
//! 1. 收到的帧按「第一个小帧的序号」放进有序表；比正在放的位置还旧的（来晚了）不要。
//! 2. 开始放之前先攒：攒够 240 毫秒、或者这段话已经结束（很短的一句）、或者第一帧到了已经 240 毫秒，就开始。
//!    语音帧每 200 毫秒成批到一次，攒 240 毫秒能盖住一批的迟到再加网络的抖动。从收到的最小序号开始放（前面丢的不补）。
//!    （原来攒 500 毫秒，是为 400 毫秒一批的帧留余量；帧改小后按比例收紧，端到端延迟直降约半秒。）
//! 3. 每 20 毫秒取一个：有就交出去；没有但后面还有（中间丢了），交「丢了」让解码器补一段（丢包补偿）；
//!    后面什么都没有、也没结束，说明网络卡了：先补 100 毫秒，还没来就停下重新攒。
//! 4. 放到结束帧为止。说的人掉线（2 秒没收到新帧、手里也没有了）也算结束。

use std::collections::BTreeMap;

/// 开始放之前攒多少（毫秒）。
pub const JITTER_MS: u64 = 240;
const FRAME_MS: u64 = 20;
/// 网络卡住时最多连续补多少个小帧（100 毫秒），之后停下重新攒。
const MAX_CONCEAL: u32 = 5;
/// 多久没收到新帧、手里也没有了，就当这段话结束。
const STALE_MS: u64 = 2000;

#[derive(Debug, PartialEq, Eq)]
pub enum Pull {
    /// 下一个小帧。
    Frame(Vec<u8>),
    /// 这个位置的小帧丢了：让解码器补一段。
    Lost,
    /// 还在攒（或网络卡了），这 20 毫秒不出声。
    Wait,
    /// 这段话放完了。
    Done,
}

#[derive(Default)]
pub struct Jitter {
    frames: BTreeMap<u32, Vec<u8>>,
    /// 下一个要放的序号。
    next: u32,
    /// 结束帧之后的序号（放到这里为止）。
    end: Option<u32>,
    playing: bool,
    /// 这一轮开始攒的时间（第一帧到达，或者卡住后重新攒）。
    wait_since: Option<u64>,
    last_arrival: u64,
    misses: u32,
}

impl Jitter {
    pub fn push(&mut self, first: u32, frames: Vec<Vec<u8>>, end: bool, now_ms: u64) {
        let count = frames.len() as u32;
        for (k, f) in frames.into_iter().enumerate() {
            let i = first + k as u32;
            if i >= self.next {
                self.frames.insert(i, f);
            }
        }
        if end {
            self.end = Some(first + count);
        }
        self.last_arrival = now_ms;
        self.wait_since.get_or_insert(now_ms);
    }

    /// 正在出声（开始放了、还没放完）。
    pub fn speaking(&self) -> bool {
        self.playing
    }

    pub fn pull(&mut self, now_ms: u64) -> Pull {
        if !self.playing {
            if self
                .end
                .is_some_and(|e| self.frames.is_empty() && e <= self.next)
            {
                return Pull::Done;
            }
            let Some(&lowest) = self.frames.keys().next() else {
                return if now_ms.saturating_sub(self.last_arrival) > STALE_MS {
                    Pull::Done
                } else {
                    Pull::Wait
                };
            };
            let highest = *self.frames.keys().next_back().unwrap_or(&lowest);
            let buffered_ms = u64::from(highest + 1 - lowest.max(self.next)) * FRAME_MS;
            let waited = self
                .wait_since
                .is_some_and(|t| now_ms.saturating_sub(t) >= JITTER_MS);
            if buffered_ms < JITTER_MS && self.end.is_none() && !waited {
                return Pull::Wait;
            }
            self.playing = true;
            self.next = self.next.max(lowest);
        }
        if self.end.is_some_and(|e| self.next >= e) {
            self.playing = false;
            return Pull::Done;
        }
        if let Some(f) = self.frames.remove(&self.next) {
            self.next += 1;
            self.misses = 0;
            return Pull::Frame(f);
        }
        let more_ahead = self.frames.keys().next().is_some() || self.end.is_some();
        if more_ahead {
            self.next += 1;
            return Pull::Lost;
        }
        if now_ms.saturating_sub(self.last_arrival) > STALE_MS {
            self.playing = false;
            return Pull::Done;
        }
        if self.misses < MAX_CONCEAL {
            self.misses += 1;
            self.next += 1;
            return Pull::Lost;
        }
        // 卡住了：停下重新攒
        self.playing = false;
        self.misses = 0;
        self.wait_since = Some(now_ms);
        Pull::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(from: u8, n: u8) -> Vec<Vec<u8>> {
        (from..from + n).map(|i| vec![i]).collect()
    }

    /// 每 20 毫秒取一次，取 n 次。
    fn run(j: &mut Jitter, start_ms: u64, n: usize) -> Vec<Pull> {
        (0..n)
            .map(|k| j.pull(start_ms + k as u64 * FRAME_MS))
            .collect()
    }

    #[test]
    fn waits_until_enough_buffered_then_plays_in_order() {
        let mut j = Jitter::default();
        j.push(0, frames(0, 10), false, 1000);
        assert_eq!(j.pull(1000), Pull::Wait, "只有 200 毫秒，还在攒");
        assert!(!j.speaking());
        j.push(10, frames(10, 10), false, 1400);
        assert_eq!(j.pull(1400), Pull::Frame(vec![0]));
        assert!(j.speaking());
        assert_eq!(j.pull(1420), Pull::Frame(vec![1]));
    }

    #[test]
    fn a_short_phrase_plays_at_once_and_ends() {
        let mut j = Jitter::default();
        j.push(0, frames(0, 3), true, 1000);
        let got = run(&mut j, 1000, 5);
        assert_eq!(
            got,
            vec![
                Pull::Frame(vec![0]),
                Pull::Frame(vec![1]),
                Pull::Frame(vec![2]),
                Pull::Done,
                Pull::Done
            ]
        );
        assert!(!j.speaking());
    }

    #[test]
    fn a_slow_first_packet_still_starts_after_the_wait() {
        let mut j = Jitter::default();
        j.push(0, frames(0, 2), false, 1000);
        assert_eq!(j.pull(1000 + JITTER_MS - 1), Pull::Wait);
        assert_eq!(j.pull(1000 + JITTER_MS), Pull::Frame(vec![0]));
    }

    #[test]
    fn lost_and_reordered_packets() {
        let mut j = Jitter::default();
        // 第二帧（20–39）丢了，第三帧先到
        j.push(40, frames(40, 20), false, 1000);
        j.push(0, frames(0, 20), false, 1010);
        let got = run(&mut j, 1010, 41);
        assert_eq!(got[0], Pull::Frame(vec![0]), "从最小的序号开始");
        assert_eq!(got[19], Pull::Frame(vec![19]));
        assert!(
            got[20..40].iter().all(|p| *p == Pull::Lost),
            "丢的那段补出来"
        );
        assert_eq!(got[40], Pull::Frame(vec![40]));
        // 来晚了的旧帧不要
        j.push(20, frames(20, 20), false, 1900);
        assert_eq!(j.pull(1900), Pull::Frame(vec![41]));
    }

    #[test]
    fn missing_start_is_skipped_not_concealed() {
        let mut j = Jitter::default();
        j.push(20, frames(20, 20), true, 1000);
        assert_eq!(j.pull(1000), Pull::Frame(vec![20]));
    }

    #[test]
    fn a_stall_conceals_100_ms_then_rebuffers() {
        let mut j = Jitter::default();
        j.push(0, frames(0, 25), false, 1000);
        let got = run(&mut j, 1000, 25 + MAX_CONCEAL as usize + 1);
        assert_eq!(got[24], Pull::Frame(vec![24]));
        assert!(
            got[25..25 + MAX_CONCEAL as usize]
                .iter()
                .all(|p| *p == Pull::Lost)
        );
        assert_eq!(
            *got.last().unwrap(),
            Pull::Wait,
            "补了 100 毫秒还没来：停下重新攒"
        );
        assert!(!j.speaking());
        // 新的帧到了：攒够再放（补过的位置不再要）
        j.push(30, frames(30, 25), false, 1700);
        assert_eq!(j.pull(1700), Pull::Frame(vec![30]));
    }

    #[test]
    fn a_vanished_speaker_ends_after_two_seconds() {
        let mut j = Jitter::default();
        j.push(0, frames(0, 25), false, 1000);
        run(&mut j, 1000, 25 + MAX_CONCEAL as usize + 1);
        assert_eq!(j.pull(1000 + STALE_MS), Pull::Wait);
        assert_eq!(j.pull(1001 + STALE_MS), Pull::Done);
        let mut j = Jitter::default();
        assert_eq!(j.pull(STALE_MS + 1), Pull::Done, "什么都没收到");
    }
}
