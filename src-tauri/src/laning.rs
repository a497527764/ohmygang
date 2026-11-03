//! 对线判定（2.0 架构 6.1）：纯函数，时间和位置都由调用方传进来。
//!
//! 实现流程：
//! 1. 对手：开着对线模式的队友。收到他的鼠标帧就记下「第几号显示器 + 比例」和收到的时间；
//!    局域网 1 秒、中继 2.5 秒没有新帧就当掉线。
//! 2. 位置换算：对手的比例按我同号显示器的物理像素算出位置，和我的鼠标比距离。不在同一号显示器就不算在范围里。
//! 3. 范围（「700 码」）：距离不超过这块显示器较短边的 35%。在范围里连续 3 秒充能完成；
//!    出范围、掉线、冷却中都会让充能中断、重新计时。开始充能和中断各报一次，交给调用方发 Charge 帧。
//! 4. 出手：落点离某个充能完成的对手不超过较短边的 6%（至少 60 像素），所有满足条件的都命中（团战）。
//!    一个都没打中，所有充能作废。判定以出手一方看到的位置为准（射手优先）。
//! 5. 冷却：对同一个对手 30 秒内不能再叠，由调用方按命中记录给出每个对手的冷却结束时间。

use std::collections::HashMap;

use crate::crypto::MemberId;
use crate::heartsteel::CHARGE_MS;

/// 范围：显示器较短边的比例。实测后再调。
pub const RANGE_RATIO: f64 = 0.35;
/// 命中半径：显示器较短边的比例，和最小值（物理像素）。
pub const HIT_RATIO: f64 = 0.06;
pub const HIT_MIN_PX: f64 = 60.0;
/// 多久没有鼠标帧就当掉线：局域网、中继。
pub const STALE_LAN_MS: u64 = 1000;
pub const STALE_RELAY_MS: u64 = 2500;

/// 本人鼠标：第几号显示器、在这块显示器里的物理像素位置、这块显示器的物理像素大小。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MyPos {
    pub display: u8,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl MyPos {
    fn short_side(&self) -> f64 {
        self.width.min(self.height)
    }

    pub fn range(&self) -> f64 {
        self.short_side() * RANGE_RATIO
    }

    pub fn hit_radius(&self) -> f64 {
        (self.short_side() * HIT_RATIO).max(HIT_MIN_PX)
    }

    /// 对手的比例位置落到我这块显示器上的物理像素位置。
    fn place(&self, rx: u16, ry: u16) -> (f64, f64) {
        (
            f64::from(rx) / 65535.0 * self.width,
            f64::from(ry) / 65535.0 * self.height,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// 不在范围里（或不在同一号显示器）。
    Far,
    /// 在范围里，从 `since` 起算。
    Charging { since: u64 },
    /// 充能完成：充能是从 `since` 开始的。
    Charged { since: u64 },
    /// 冷却到 `until`。
    Cooldown { until: u64 },
}

#[derive(Clone, Copy, Debug)]
struct Opponent {
    display: u8,
    x: u16,
    y: u16,
    received: u64,
    stale_after: u64,
    phase: Phase,
}

/// 交给调用方的事：开始充能、充能中断（发 Charge 帧）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneEvent {
    ChargeStarted(MemberId),
    ChargeStopped(MemberId),
}

/// 一次出手打中的人和他的充能时长。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub target: MemberId,
    pub charged_ms: u32,
}

/// 画面要的：一个对手在哪、什么状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpponentView {
    pub member: MemberId,
    pub display: u8,
    pub x: u16,
    pub y: u16,
    pub phase: Phase,
}

#[derive(Default)]
pub struct Laning {
    opponents: HashMap<MemberId, Opponent>,
}

fn is_charging(p: Phase) -> bool {
    matches!(p, Phase::Charging { .. } | Phase::Charged { .. })
}

impl Laning {
    /// 收到对手的一帧鼠标。`via_relay` 决定多久没新帧算掉线。
    pub fn on_cursor(
        &mut self,
        from: MemberId,
        display: u8,
        x: u16,
        y: u16,
        via_relay: bool,
        now: u64,
    ) {
        let stale_after = if via_relay {
            STALE_RELAY_MS
        } else {
            STALE_LAN_MS
        };
        let phase = self.opponents.get(&from).map_or(Phase::Far, |o| o.phase);
        self.opponents.insert(
            from,
            Opponent {
                display,
                x,
                y,
                received: now,
                stale_after,
                phase,
            },
        );
    }

    /// 对手关了对线模式、离开队伍。在充能的话报一次中断。
    pub fn remove(&mut self, from: &MemberId) -> Option<LaneEvent> {
        let o = self.opponents.remove(from)?;
        is_charging(o.phase).then_some(LaneEvent::ChargeStopped(*from))
    }

    /// 全部清掉（本人关了对线模式）。
    pub fn clear(&mut self) -> Vec<LaneEvent> {
        let ids: Vec<MemberId> = self.opponents.keys().copied().collect();
        ids.iter().filter_map(|m| self.remove(m)).collect()
    }

    /// 往前走一步：按本人鼠标、时间、各对手的冷却结束时间更新每个对手的状态，返回开始和中断的充能。
    pub fn step(
        &mut self,
        me: Option<MyPos>,
        now: u64,
        cooldown_until: impl Fn(&MemberId) -> Option<u64>,
    ) -> Vec<LaneEvent> {
        let mut out = Vec::new();
        self.opponents.retain(|m, o| {
            let alive = now.saturating_sub(o.received) <= o.stale_after;
            if !alive && is_charging(o.phase) {
                out.push(LaneEvent::ChargeStopped(*m));
            }
            alive
        });
        for (m, o) in &mut self.opponents {
            let before = o.phase;
            let cooling = cooldown_until(m).filter(|until| *until > now);
            let in_range = me.is_some_and(|me| {
                let (ox, oy) = me.place(o.x, o.y);
                me.display == o.display && (ox - me.x).hypot(oy - me.y) <= me.range()
            });
            o.phase = match (cooling, in_range, before) {
                (Some(until), _, _) => Phase::Cooldown { until },
                (None, false, _) => Phase::Far,
                (None, true, Phase::Charging { since })
                | (None, true, Phase::Charged { since }) => {
                    if now.saturating_sub(since) >= u64::from(CHARGE_MS) {
                        Phase::Charged { since }
                    } else {
                        Phase::Charging { since }
                    }
                }
                (None, true, Phase::Far | Phase::Cooldown { .. }) => Phase::Charging { since: now },
            };
            match (is_charging(before), is_charging(o.phase)) {
                (false, true) => out.push(LaneEvent::ChargeStarted(*m)),
                (true, false) => out.push(LaneEvent::ChargeStopped(*m)),
                _ => {}
            }
        }
        out
    }

    /// 有没有充能完成的对手（这时触发键加单击是「普攻」）。
    pub fn any_charged(&self) -> bool {
        self.opponents
            .values()
            .any(|o| matches!(o.phase, Phase::Charged { .. }))
    }

    /// 在 `at`（我这块显示器上的物理像素）出手。返回打中的人；打中的和没打中的充能都结束（报中断）。
    pub fn attack(&mut self, at: MyPos, now: u64) -> (Vec<Hit>, Vec<LaneEvent>) {
        let mut hits = Vec::new();
        for (m, o) in &self.opponents {
            if let Phase::Charged { since } = o.phase {
                let (ox, oy) = at.place(o.x, o.y);
                if at.display == o.display && (ox - at.x).hypot(oy - at.y) <= at.hit_radius() {
                    hits.push(Hit {
                        target: *m,
                        charged_ms: u32::try_from(now.saturating_sub(since)).unwrap_or(u32::MAX),
                    });
                }
            }
        }
        // 打中了：这些人进冷却（下一步由调用方给的冷却时间决定）；打空了：所有充能作废
        let mut events = Vec::new();
        for (m, o) in &mut self.opponents {
            let hit = hits.iter().any(|h| h.target == *m);
            if (hit || hits.is_empty()) && is_charging(o.phase) {
                o.phase = Phase::Far;
                events.push(LaneEvent::ChargeStopped(*m));
            }
        }
        (hits, events)
    }

    /// 某个对手现在的位置（命中特效画在这里）。
    pub fn position_of(&self, m: &MemberId) -> Option<(u8, u16, u16)> {
        self.opponents.get(m).map(|o| (o.display, o.x, o.y))
    }

    pub fn view(&self) -> Vec<OpponentView> {
        self.opponents
            .iter()
            .map(|(m, o)| OpponentView {
                member: *m,
                display: o.display,
                x: o.x,
                y: o.y,
                phase: o.phase,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: MemberId = [1u8; 32];
    const B: MemberId = [2u8; 32];

    /// 1 号显示器 1920×1080：范围 378 像素，命中半径 64.8 像素。
    fn me(x: f64, y: f64) -> MyPos {
        MyPos {
            display: 1,
            x,
            y,
            width: 1920.0,
            height: 1080.0,
        }
    }

    fn ratio(px: f64, size: f64) -> u16 {
        (px / size * 65535.0).round() as u16
    }

    /// 把对手放在 1 号显示器的 (x, y) 物理像素处。
    fn put(l: &mut Laning, who: MemberId, x: f64, y: f64, now: u64) {
        l.on_cursor(who, 1, ratio(x, 1920.0), ratio(y, 1080.0), false, now);
    }

    fn no_cooldown(_: &MemberId) -> Option<u64> {
        None
    }

    #[test]
    fn three_seconds_in_range_charges() {
        let mut l = Laning::default();
        put(&mut l, A, 1000.0, 500.0, 0);
        assert_eq!(
            l.step(Some(me(800.0, 500.0)), 0, no_cooldown),
            vec![LaneEvent::ChargeStarted(A)]
        );
        for t in [500, 1000, 2000, 2999] {
            put(&mut l, A, 1000.0, 500.0, t);
            assert!(l.step(Some(me(800.0, 500.0)), t, no_cooldown).is_empty());
            assert!(!l.any_charged());
        }
        put(&mut l, A, 1000.0, 500.0, 3000);
        l.step(Some(me(800.0, 500.0)), 3000, no_cooldown);
        assert!(l.any_charged());
    }

    #[test]
    fn leaving_range_or_another_display_breaks_the_charge() {
        let mut l = Laning::default();
        put(&mut l, A, 1000.0, 500.0, 0);
        l.step(Some(me(800.0, 500.0)), 0, no_cooldown);
        // 甩开：距离 400 > 378
        put(&mut l, A, 1000.0, 500.0, 1000);
        assert_eq!(
            l.step(Some(me(600.0, 500.0)), 1000, no_cooldown),
            vec![LaneEvent::ChargeStopped(A)]
        );
        // 回来重新计时（对手的鼠标帧一直在来）
        put(&mut l, A, 1000.0, 500.0, 1500);
        assert_eq!(
            l.step(Some(me(800.0, 500.0)), 1500, no_cooldown),
            vec![LaneEvent::ChargeStarted(A)]
        );
        put(&mut l, A, 1000.0, 500.0, 4000);
        l.step(Some(me(800.0, 500.0)), 4000, no_cooldown);
        assert!(!l.any_charged(), "从 1500 起才 2.5 秒");
        // 对手在 2 号显示器：不算在范围里
        l.on_cursor(
            A,
            2,
            ratio(1000.0, 1920.0),
            ratio(500.0, 1080.0),
            false,
            4100,
        );
        assert_eq!(
            l.step(Some(me(1000.0, 500.0)), 4100, no_cooldown),
            vec![LaneEvent::ChargeStopped(A)]
        );
    }

    #[test]
    fn stale_opponents_drop_out() {
        let mut l = Laning::default();
        put(&mut l, A, 1000.0, 500.0, 0);
        l.step(Some(me(900.0, 500.0)), 0, no_cooldown);
        assert!(
            l.step(Some(me(900.0, 500.0)), STALE_LAN_MS, no_cooldown)
                .is_empty(),
            "正好 1 秒还在"
        );
        assert_eq!(
            l.step(Some(me(900.0, 500.0)), STALE_LAN_MS + 1, no_cooldown),
            vec![LaneEvent::ChargeStopped(A)]
        );
        assert!(l.view().is_empty());
        // 中继来的帧多等一会
        l.on_cursor(B, 1, 0, 0, true, 0);
        l.step(None, 2000, no_cooldown);
        assert_eq!(l.view().len(), 1);
    }

    #[test]
    fn attack_hits_every_charged_opponent_near_the_click() {
        let mut l = Laning::default();
        put(&mut l, A, 1000.0, 500.0, 0);
        put(&mut l, B, 1030.0, 520.0, 0);
        l.step(Some(me(900.0, 500.0)), 0, no_cooldown);
        put(&mut l, A, 1000.0, 500.0, 3000);
        put(&mut l, B, 1030.0, 520.0, 3000);
        l.step(Some(me(900.0, 500.0)), 3000, no_cooldown);
        let (hits, events) = l.attack(me(1010.0, 505.0), 3200);
        let mut targets: Vec<_> = hits.iter().map(|h| (h.target, h.charged_ms)).collect();
        targets.sort();
        assert_eq!(targets, vec![(A, 3200), (B, 3200)], "团战：两个都打中");
        assert_eq!(events.len(), 2);
        assert!(!l.any_charged());
    }

    #[test]
    fn a_miss_voids_every_charge() {
        let mut l = Laning::default();
        put(&mut l, A, 1000.0, 500.0, 0);
        l.step(Some(me(900.0, 500.0)), 0, no_cooldown);
        put(&mut l, A, 1000.0, 500.0, 3000);
        l.step(Some(me(900.0, 500.0)), 3000, no_cooldown);
        let (hits, events) = l.attack(me(900.0, 500.0), 3100); // 离 A 100 像素 > 64.8
        assert!(hits.is_empty());
        assert_eq!(events, vec![LaneEvent::ChargeStopped(A)]);
        assert!(!l.any_charged());
        // 还在范围里，从头再充
        assert_eq!(
            l.step(Some(me(900.0, 500.0)), 3200, no_cooldown),
            vec![LaneEvent::ChargeStarted(A)]
        );
    }

    #[test]
    fn hit_radius_has_a_floor() {
        let small = MyPos {
            display: 1,
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        assert_eq!(small.hit_radius(), HIT_MIN_PX);
        assert!((me(0.0, 0.0).hit_radius() - 64.8).abs() < 1e-9);
        assert!((me(0.0, 0.0).range() - 378.0).abs() < 1e-9);
    }

    #[test]
    fn cooldown_blocks_charging_until_it_ends() {
        let mut l = Laning::default();
        put(&mut l, A, 1000.0, 500.0, 0);
        let cd = |m: &MemberId| (*m == A).then_some(30_000);
        assert!(l.step(Some(me(900.0, 500.0)), 0, cd).is_empty());
        assert_eq!(l.view()[0].phase, Phase::Cooldown { until: 30_000 });
        put(&mut l, A, 1000.0, 500.0, 30_000);
        assert_eq!(
            l.step(Some(me(900.0, 500.0)), 30_000, cd),
            vec![LaneEvent::ChargeStarted(A)]
        );
    }

    #[test]
    fn removing_or_clearing_reports_broken_charges() {
        let mut l = Laning::default();
        put(&mut l, A, 1000.0, 500.0, 0);
        put(&mut l, B, 100.0, 100.0, 0);
        l.step(Some(me(900.0, 500.0)), 0, no_cooldown);
        assert_eq!(l.remove(&B), None, "乙不在范围里，没在充能");
        assert_eq!(l.clear(), vec![LaneEvent::ChargeStopped(A)]);
        assert!(l.view().is_empty());
    }
}
