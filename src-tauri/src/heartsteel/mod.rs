//! 心之钢：作者端判定与重放（架构第 5 节）。
//!
//! 实现流程：
//! 1. `evaluate_emit`（作者端，只在本人发消息时调用）：先算出上一次有效叠层的冷却结束时间，
//!    只看冷却结束之后的本人消息，取最近「阈值」条，首尾时间差不超过窗口就触发，返回触发消息。
//! 2. `replay_author`（所有人都算，层数的唯一来源）：把某位作者的全部叠层事件按 (seq, event_id) 排序，
//!    逐条检查条数、时间递增、窗口、冷却、触发消息不可复用、解散之后无效；通过的才算一层。
//!    它是事件集合的纯函数：同一批事件不管以什么顺序到达，算出的层数都相同。
//! 3. 规则缺失（规则事件还没同步到）时跳过该事件，等规则到达后重算即可计入。
//! 4. 读写数据库的部分（重算、层数缓存、发言时判定）在 `service` 子模块。

pub mod service;

use std::collections::{HashMap, HashSet};

use crate::crypto::EventId;
use crate::protocol::body::{HeartsteelBody, Rule, Trigger};

/// rule_epoch → 规则。所有历史版本都要保留，否则校验不了旧事件。
pub type Rules = HashMap<u32, Rule>;

/// 本人的一条聊天消息，判定只需要 ID 和时间。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatPoint {
    pub event_id: EventId,
    pub ts_ms: u64,
}

/// 一条已解码的叠层事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HsEvent {
    pub seq: u64,
    pub event_id: EventId,
    pub body: HeartsteelBody,
}

/// 某条有效叠层事件的冷却结束时刻（含边界：等于该时刻的消息可以计入下一次）。
fn cooldown_end(h: &HsEvent, rules: &Rules) -> Option<u64> {
    let last = h.body.triggers.last()?.ts_ms;
    let rule = rules.get(&h.body.rule_epoch)?;
    Some(last + u64::from(rule.cooldown_ms))
}

/// 架构 5.3。`my_chats` 含刚发出的这条；`last_valid` 是本人最后一条有效叠层事件。
pub fn evaluate_emit(
    my_chats: &[ChatPoint],
    last_valid: Option<&HsEvent>,
    rules: &Rules,
    current: &Rule,
) -> Option<Vec<Trigger>> {
    let since = match last_valid {
        Some(h) => cooldown_end(h, rules)?,
        None => 0,
    };
    let mut eligible: Vec<&ChatPoint> = my_chats.iter().filter(|c| c.ts_ms >= since).collect();
    eligible.sort_by_key(|c| (c.ts_ms, c.event_id));
    let n = current.threshold as usize;
    if n == 0 || eligible.len() < n {
        return None;
    }
    let t = &eligible[eligible.len() - n..];
    if t[n - 1].ts_ms - t[0].ts_ms > u64::from(current.window_ms) {
        return None;
    }
    Some(
        t.iter()
            .map(|c| Trigger {
                event_id: c.event_id,
                ts_ms: c.ts_ms,
            })
            .collect(),
    )
}

/// 架构 5.4。返回有效事件（按 seq 升序），层数 = 返回的条数。
pub fn replay_author<'a>(
    events: &'a [HsEvent],
    rules: &Rules,
    dissolve_ts: Option<u64>,
) -> Vec<&'a HsEvent> {
    let mut sorted: Vec<&HsEvent> = events.iter().collect();
    sorted.sort_by_key(|h| (h.seq, h.event_id));

    let mut valid: Vec<&HsEvent> = Vec::new();
    let mut used: HashSet<EventId> = HashSet::new();
    let mut prev_seq = None;
    for h in sorted {
        if prev_seq == Some(h.seq) {
            continue; // 同一 seq 只取 event_id 最小的一条（分叉）
        }
        prev_seq = Some(h.seq);

        let Some(rule) = rules.get(&h.body.rule_epoch) else {
            continue; // 规则还没到，暂不计入
        };
        let t = &h.body.triggers;
        if t.is_empty() || t.len() != rule.threshold as usize {
            continue;
        }
        if !t.windows(2).all(|w| w[0].ts_ms < w[1].ts_ms) {
            continue;
        }
        let (first, last) = (t[0].ts_ms, t[t.len() - 1].ts_ms);
        if last - first > u64::from(rule.window_ms) {
            continue;
        }
        if let Some(p) = valid.last() {
            match cooldown_end(p, rules) {
                Some(end) if first >= end => {}
                _ => continue,
            }
        }
        if t.iter().any(|x| used.contains(&x.event_id)) {
            continue;
        }
        if dissolve_ts.is_some_and(|d| last > d) {
            continue;
        }
        used.extend(t.iter().map(|x| x.event_id));
        valid.push(h);
    }
    valid
}

#[cfg(test)]
mod tests;
