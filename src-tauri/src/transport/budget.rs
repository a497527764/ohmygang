//! 中继发送预算（架构 6.6）：所有房间共用一个预算，防止把中继打挂。
//!
//! 实现流程：
//! 1. 令牌桶：每秒回补 20 个、最多存 40 个。现在是自有中继（无公共中继的限流封禁），按服务器
//!    收包能力（每连接每秒 200 个事件）的十分之一设预算：语音 200 毫秒一帧用 5 个/秒，
//!    其余给信号、在线状态和表情分块留足余量。一个包发给挑出来的几个中继（见 relay.rs），只扣一个令牌。
//!    和洪水防护一样用「千分之一个令牌」做单位、时间由调用方传入。
//! 2. 三个队列按优先级出队：本人实时事件 > Have > 补齐回应。
//! 3. 每个队列都有上限，满了丢最旧的：中继长时间连不上时内存不会无限涨。
//!    补齐队列上限最小，积压时最先被挤掉；对方等不到会超时重新要。
//! 4. 同一房间的同一个包已经在队列里时不再入队。
//! 5. 队列为空时不扣令牌；没有可用中继时调用方不取包，包留在队列里等连上。

use std::collections::VecDeque;

use crate::crypto::RoomId;

const LIVE_CAP: usize = 512;
const CONTROL_CAP: usize = 64;
/// 生产预算：每秒 20 个、突发 40 个、补齐队列 128 个。测试用显式小参数（见下）。
const RATE: u64 = 20;
const BURST: u64 = 40;
const BACKFILL_CAP: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// 本人刚产生的事件（聊天、叠层、进房等）。
    Live,
    /// Have 等控制帧。
    Control,
    /// 回应别人 Want 的补齐事件。
    Backfill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub room: RoomId,
    pub packet: Vec<u8>,
}

pub struct SendBudget {
    rate: u64,
    burst: u64,
    milli: u64,
    last_ms: u64,
    queues: [VecDeque<Outgoing>; 3],
    caps: [usize; 3],
}

impl Default for SendBudget {
    fn default() -> Self {
        Self::new(RATE, BURST, BACKFILL_CAP)
    }
}

impl SendBudget {
    pub fn new(rate: u64, burst: u64, backfill_cap: usize) -> Self {
        SendBudget {
            rate,
            burst,
            milli: burst * 1000,
            last_ms: 0,
            queues: Default::default(),
            caps: [LIVE_CAP, CONTROL_CAP, backfill_cap],
        }
    }

    /// 入队；重复的包返回 false。
    pub fn push(&mut self, priority: Priority, out: Outgoing, now_ms: u64) -> bool {
        self.refill(now_ms);
        if self.queues.iter().any(|q| q.contains(&out)) {
            return false;
        }
        let i = priority as usize;
        if self.queues[i].len() >= self.caps[i] {
            self.queues[i].pop_front();
        }
        self.queues[i].push_back(out);
        true
    }

    /// 有令牌且有包时取出优先级最高的一个包，扣一个令牌。
    pub fn pop(&mut self, now_ms: u64) -> Option<Outgoing> {
        self.refill(now_ms);
        if self.milli < 1000 {
            return None;
        }
        let out = self.queues.iter_mut().find_map(VecDeque::pop_front)?;
        self.milli -= 1000;
        Some(out)
    }

    /// 还要等多少毫秒才能取下一个包；队列为空时返回 None。
    pub fn wait_ms(&self, now_ms: u64) -> Option<u64> {
        if self.is_empty() {
            return None;
        }
        let elapsed = now_ms.saturating_sub(self.last_ms);
        let milli = (self.milli + elapsed * self.rate).min(self.burst * 1000);
        Some((1000u64.saturating_sub(milli)).div_ceil(self.rate))
    }

    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(VecDeque::is_empty)
    }

    pub fn len(&self) -> usize {
        self.queues.iter().map(VecDeque::len).sum()
    }

    fn refill(&mut self, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.last_ms);
        self.milli = (self.milli + elapsed * self.rate).min(self.burst * 1000);
        self.last_ms = self.last_ms.max(now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(room: u8, byte: u8) -> Outgoing {
        Outgoing {
            room: [room; 16],
            packet: vec![byte; 8],
        }
    }

    fn drain(b: &mut SendBudget, now_ms: u64) -> Vec<u8> {
        std::iter::from_fn(|| b.pop(now_ms))
            .map(|o| o.packet[0])
            .collect()
    }

    #[test]
    fn burst_of_ten_then_waits() {
        let mut b = SendBudget::new(5, 10, 64);
        for i in 0..11 {
            assert!(b.push(Priority::Live, out(1, i), 0));
        }
        assert_eq!(drain(&mut b, 0).len(), 10);
        assert!(b.pop(0).is_none());
        // 每秒 5 个，攒够 1 个要 200 毫秒
        assert_eq!(b.wait_ms(0), Some(200));
        assert!(b.pop(199).is_none());
        assert_eq!(b.pop(200).map(|o| o.packet[0]), Some(10));
        assert_eq!(b.wait_ms(200), None);
    }

    #[test]
    fn refills_five_per_second_up_to_ten() {
        let mut b = SendBudget::new(5, 10, 64);
        for i in 0..40 {
            b.push(Priority::Live, out(1, i), 0);
        }
        assert_eq!(drain(&mut b, 0).len(), 10);
        assert_eq!(drain(&mut b, 1000).len(), 5);
        // 空闲很久也最多攒 10 个
        assert_eq!(drain(&mut b, 60_000).len(), 10);
    }

    #[test]
    fn empty_queues_do_not_spend_tokens() {
        let mut b = SendBudget::new(5, 10, 64);
        assert!(b.pop(0).is_none());
        assert_eq!(b.wait_ms(0), None);
        for i in 0..10 {
            b.push(Priority::Live, out(1, i), 5000);
        }
        assert_eq!(drain(&mut b, 5000).len(), 10);
    }

    #[test]
    fn higher_priority_goes_first() {
        let mut b = SendBudget::new(5, 10, 64);
        b.push(Priority::Backfill, out(1, 1), 0);
        b.push(Priority::Control, out(1, 2), 0);
        b.push(Priority::Live, out(1, 3), 0);
        b.push(Priority::Backfill, out(1, 4), 0);
        b.push(Priority::Live, out(1, 5), 0);
        assert_eq!(drain(&mut b, 0), vec![3, 5, 2, 1, 4]);
    }

    #[test]
    fn full_backfill_queue_drops_oldest() {
        let mut b = SendBudget::new(5, 10, 3);
        for i in 0..5 {
            assert!(b.push(Priority::Backfill, out(1, i), 0));
        }
        assert_eq!(b.len(), 3);
        assert_eq!(drain(&mut b, 0), vec![2, 3, 4]);
    }

    #[test]
    fn same_packet_is_queued_once() {
        let mut b = SendBudget::new(5, 10, 64);
        assert!(b.push(Priority::Live, out(1, 7), 0));
        assert!(!b.push(Priority::Live, out(1, 7), 0));
        assert!(!b.push(Priority::Backfill, out(1, 7), 0));
        // 同样的字节发往另一个房间不算重复
        assert!(b.push(Priority::Live, out(2, 7), 0));
        assert_eq!(b.len(), 2);
    }
}
