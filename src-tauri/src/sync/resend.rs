//! 自动重发（架构 7.1）：本人的新事件广播后，没被确认就在 6、15、30 秒各重发一次，之后停止，交给周期 Have 补齐。
//!
//! 实现流程：
//! 1. 每个事件记下首次发送时刻和已经重发了几次；时刻表由调用方给（测试里缩短）。
//! 2. `due`：到点的事件交出去重发。节点卡住很久才检查时只补发一次，错过的时刻直接跳过，不会连发三次。
//! 3. `confirm`：同伴的 Have 覆盖到的事件从队列里删掉；同一事件重复加入只算一个。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::crypto::{EventId, RoomId};

struct Item<T> {
    room: RoomId,
    payload: T,
    first: Instant,
    sent: usize,
}

pub struct ResendQueue<T> {
    after: [Duration; 3],
    items: HashMap<EventId, Item<T>>,
}

impl<T: Clone> ResendQueue<T> {
    pub fn new(after: [Duration; 3]) -> Self {
        ResendQueue {
            after,
            items: HashMap::new(),
        }
    }

    pub fn add(&mut self, room: RoomId, id: EventId, payload: T, now: Instant) {
        self.items.entry(id).or_insert(Item {
            room,
            payload,
            first: now,
            sent: 0,
        });
    }

    pub fn confirm(&mut self, ids: &[EventId]) {
        for id in ids {
            self.items.remove(id);
        }
    }

    /// 到点该重发的事件，按首次发送时刻排序。
    pub fn due(&mut self, now: Instant) -> Vec<(RoomId, T)> {
        let after = self.after;
        let mut out: Vec<(Instant, EventId, RoomId, T)> = Vec::new();
        for (id, it) in self.items.iter_mut() {
            if now < it.first + after[it.sent] {
                continue;
            }
            out.push((it.first, *id, it.room, it.payload.clone()));
            it.sent = after.iter().filter(|d| it.first + **d <= now).count();
        }
        self.items.retain(|_, it| it.sent < after.len());
        out.sort_by_key(|a| (a.0, a.1));
        out.into_iter().map(|(_, _, room, p)| (room, p)).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: RoomId = [1u8; 16];

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn schedule() -> [Duration; 3] {
        [ms(6000), ms(15000), ms(30000)]
    }

    #[test]
    fn resends_at_six_fifteen_and_thirty_seconds_then_stops() {
        let t0 = Instant::now();
        let mut q: ResendQueue<u8> = ResendQueue::new(schedule());
        q.add(ROOM, [9u8; 32], 7, t0);
        assert!(q.due(t0 + ms(5999)).is_empty());
        assert_eq!(q.due(t0 + ms(6000)), vec![(ROOM, 7)]);
        assert!(q.due(t0 + ms(6001)).is_empty(), "同一次不重复");
        assert!(q.due(t0 + ms(14999)).is_empty());
        assert_eq!(q.due(t0 + ms(15000)), vec![(ROOM, 7)]);
        assert_eq!(q.due(t0 + ms(30000)), vec![(ROOM, 7)]);
        assert!(q.due(t0 + ms(90000)).is_empty(), "三次之后不再重发");
        assert!(q.is_empty());
    }

    #[test]
    fn late_tick_sends_once_not_three_times() {
        let t0 = Instant::now();
        let mut q: ResendQueue<u8> = ResendQueue::new(schedule());
        q.add(ROOM, [9u8; 32], 7, t0);
        // 节点卡住很久后才检查：只补发一次，下一次按原时刻表
        assert_eq!(q.due(t0 + ms(20000)), vec![(ROOM, 7)]);
        assert!(q.due(t0 + ms(20001)).is_empty());
        assert_eq!(q.due(t0 + ms(30000)), vec![(ROOM, 7)]);
    }

    #[test]
    fn confirmed_events_are_not_resent() {
        let t0 = Instant::now();
        let mut q: ResendQueue<u8> = ResendQueue::new(schedule());
        q.add(ROOM, [1u8; 32], 1, t0);
        q.add(ROOM, [2u8; 32], 2, t0);
        q.confirm(&[[1u8; 32]]);
        assert_eq!(q.due(t0 + ms(6000)), vec![(ROOM, 2)]);
        // 重复加入同一事件不会多发
        q.add(ROOM, [2u8; 32], 2, t0);
        assert!(q.due(t0 + ms(6001)).is_empty());
    }
}
