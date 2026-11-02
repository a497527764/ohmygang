//! 心之钢测试：架构 5.5 的 V1–V9、R1–R7，以及收敛性质测试。

use super::*;

const RULE: Rule = Rule::DEFAULT; // 5000 / 3 / 30000

fn chat_id(n: u64) -> EventId {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&n.to_be_bytes());
    b
}

fn hs_id(n: u64) -> EventId {
    let mut b = chat_id(n);
    b[31] = 0xff; // 和聊天 ID 区分开
    b
}

fn rules_of(list: &[Rule]) -> Rules {
    list.iter().map(|r| (r.rule_epoch, *r)).collect()
}

/// 模拟一个诚实作者：每次发言都调用 evaluate_emit，触发就生成一条叠层事件。
/// `sends` 是 (发送时间, 当时使用的规则)。
fn simulate(sends: &[(u64, Rule)], rules: &Rules) -> Vec<HsEvent> {
    let mut chats = Vec::new();
    let mut events: Vec<HsEvent> = Vec::new();
    for (i, (ts, rule)) in sends.iter().enumerate() {
        chats.push(ChatPoint {
            event_id: chat_id(i as u64),
            ts_ms: *ts,
        });
        let last = replay_author(&events, rules, None)
            .last()
            .map(|h| (*h).clone());
        if let Some(triggers) = evaluate_emit(&chats, last.as_ref(), rules, rule) {
            let seq = events.len() as u64 + 1;
            events.push(HsEvent {
                seq,
                event_id: hs_id(seq),
                body: HeartsteelBody {
                    rule_epoch: rule.rule_epoch,
                    triggers,
                },
            });
        }
    }
    events
}

/// 给出发送时间（毫秒），返回每次叠层的触发时间。
fn run_vector(sends_ms: &[u64]) -> Vec<Vec<u64>> {
    let rules = rules_of(&[RULE]);
    let sends: Vec<(u64, Rule)> = sends_ms.iter().map(|t| (*t, RULE)).collect();
    let events = simulate(&sends, &rules);
    let valid = replay_author(&events, &rules, None);
    assert_eq!(
        valid.len(),
        events.len(),
        "诚实作者的事件重放时必须全部有效"
    );
    valid
        .iter()
        .map(|h| h.body.triggers.iter().map(|t| t.ts_ms).collect())
        .collect()
}

#[test]
fn v1_basic() {
    assert_eq!(run_vector(&[0, 1000, 2000]), vec![vec![0, 1000, 2000]]);
}

#[test]
fn v2_window_inclusive() {
    assert_eq!(run_vector(&[0, 2500, 5000]), vec![vec![0, 2500, 5000]]);
}

#[test]
fn v3_window_exceeded_by_1ms() {
    assert!(run_vector(&[0, 2500, 5001]).is_empty());
}

#[test]
fn v4_no_stack_during_cooldown() {
    assert_eq!(
        run_vector(&[0, 1000, 2000, 3000, 4000, 5000]),
        vec![vec![0, 1000, 2000]]
    );
}

#[test]
fn v5_cooldown_messages_do_not_count() {
    assert_eq!(
        run_vector(&[0, 1000, 2000, 28_000, 29_000, 32_500]),
        vec![vec![0, 1000, 2000]]
    );
}

#[test]
fn v6_restack_after_cooldown() {
    assert_eq!(
        run_vector(&[0, 1000, 2000, 28_000, 29_000, 32_500, 33_000, 34_000]),
        vec![vec![0, 1000, 2000], vec![32_500, 33_000, 34_000]]
    );
}

#[test]
fn v7_cooldown_end_inclusive() {
    assert_eq!(
        run_vector(&[0, 1000, 2000, 32_000, 33_000, 34_000]),
        vec![vec![0, 1000, 2000], vec![32_000, 33_000, 34_000]]
    );
}

#[test]
fn v8_sliding_window() {
    assert_eq!(
        run_vector(&[0, 10_000, 11_000, 12_000]),
        vec![vec![10_000, 11_000, 12_000]]
    );
}

#[test]
fn v9_too_slow() {
    assert!(run_vector(&[0, 3000, 6000, 9000]).is_empty());
}

/// 手工构造一条叠层事件，触发消息为 (聊天编号, 时间)。
fn hs(seq: u64, epoch: u32, triggers: &[(u64, u64)]) -> HsEvent {
    HsEvent {
        seq,
        event_id: hs_id(seq),
        body: HeartsteelBody {
            rule_epoch: epoch,
            triggers: triggers
                .iter()
                .map(|(n, ts)| Trigger {
                    event_id: chat_id(*n),
                    ts_ms: *ts,
                })
                .collect(),
        },
    }
}

fn stacks(events: &[HsEvent], rules: &Rules, dissolve_ts: Option<u64>) -> usize {
    replay_author(events, rules, dissolve_ts).len()
}

#[test]
fn r1_triggers_cannot_be_reused() {
    let t = [(0, 0), (1, 1000), (2, 2000)];
    assert_eq!(
        stacks(&[hs(1, 0, &t), hs(2, 0, &t)], &rules_of(&[RULE]), None),
        1
    );
}

#[test]
fn r2_second_within_cooldown() {
    let e = [
        hs(1, 0, &[(0, 0), (1, 1000), (2, 2000)]),
        hs(2, 0, &[(3, 10_000), (4, 11_000), (5, 12_000)]),
    ];
    assert_eq!(stacks(&e, &rules_of(&[RULE]), None), 1);
}

#[test]
fn r3_too_few_triggers() {
    assert_eq!(
        stacks(&[hs(1, 0, &[(0, 0), (1, 1000)])], &rules_of(&[RULE]), None),
        0
    );
}

#[test]
fn r4_window_exceeded() {
    assert_eq!(
        stacks(
            &[hs(1, 0, &[(0, 0), (1, 1000), (2, 6000)])],
            &rules_of(&[RULE]),
            None
        ),
        0
    );
}

#[test]
fn r5_r6_unknown_epoch_then_rule_arrives() {
    let e = [hs(1, 7, &[(0, 0), (1, 1000), (2, 2000)])];
    assert_eq!(stacks(&e, &rules_of(&[RULE]), None), 0);
    let epoch7 = Rule {
        rule_epoch: 7,
        ..RULE
    };
    assert_eq!(stacks(&e, &rules_of(&[RULE, epoch7]), None), 1);
}

#[test]
fn r7_after_dissolve_is_invalid() {
    let e = [
        hs(1, 0, &[(0, 0), (1, 1000), (2, 2000)]),
        hs(2, 0, &[(3, 100_000), (4, 101_000), (5, 102_000)]),
    ];
    assert_eq!(stacks(&e, &rules_of(&[RULE]), None), 2);
    assert_eq!(stacks(&e, &rules_of(&[RULE]), Some(50_000)), 1);
}

#[test]
fn fork_same_seq_takes_smallest_id() {
    let a = hs(1, 0, &[(0, 0), (1, 1000), (2, 2000)]);
    let mut b = hs(1, 0, &[(3, 0), (4, 1000), (5, 2000)]);
    b.event_id = [0u8; 32]; // 比 a 的 ID 小
    let all = [a, b.clone()];
    let valid = replay_author(&all, &rules_of(&[RULE]), None);
    assert_eq!(valid.len(), 1);
    assert_eq!(valid[0].event_id, b.event_id);
}

/// 不引入随机数库，用一个简单的 xorshift 生成可复现的伪随机数。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

#[test]
fn convergence_property() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut total_stacks = 0;
    for _ in 0..3000 {
        // 两版规则：中途可能切换（模拟房主改规则）
        let r0 = Rule {
            rule_epoch: 0,
            window_ms: rng.range(1000, 8000) as u32,
            threshold: rng.range(2, 5) as u32,
            cooldown_ms: rng.range(1000, 20_000) as u32,
        };
        let r1 = Rule {
            rule_epoch: 1,
            window_ms: rng.range(1000, 8000) as u32,
            threshold: rng.range(2, 5) as u32,
            cooldown_ms: rng.range(1000, 20_000) as u32,
        };
        let rules = rules_of(&[r0, r1]);
        let n = rng.range(1, 60);
        let switch_at = rng.range(0, n);
        let mut ts = rng.range(0, 1000);
        let mut sends = Vec::new();
        for i in 0..n {
            ts += rng.range(1, 4000); // 严格递增
            sends.push((ts, if i < switch_at { r0 } else { r1 }));
        }
        let events = simulate(&sends, &rules);
        let valid_ids: Vec<EventId> = replay_author(&events, &rules, None)
            .iter()
            .map(|h| h.event_id)
            .collect();
        assert_eq!(
            valid_ids.len(),
            events.len(),
            "诚实作者的事件重放时必须全部有效"
        );
        total_stacks += valid_ids.len();

        // 打乱到达顺序，结果不变
        let mut shuffled = events.clone();
        for i in (1..shuffled.len()).rev() {
            let j = rng.range(0, i as u64) as usize;
            shuffled.swap(i, j);
        }
        let again: Vec<EventId> = replay_author(&shuffled, &rules, None)
            .iter()
            .map(|h| h.event_id)
            .collect();
        assert_eq!(again, valid_ids);
    }
    assert!(
        total_stacks > 1000,
        "随机样本里应该有足够多的叠层，实际 {total_stacks}"
    );
}
