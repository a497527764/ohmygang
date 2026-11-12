//! 路径判定（架构 6.5）：每位同伴现在经哪条路径能联系上，以及本人的事件要不要发到中继。纯函数。
//!
//! 实现流程：
//! 1. 每位同伴在中继路径上记「最近一次收到包的时间」和「对方报的 Have 周期」。
//!    任何包都刷新时间；只有 Have 带周期，其他包沿用上次的；从没收到过 Have 时用默认周期。
//! 2. 超过周期 3 倍没收到包，这条路径算断开。中继通就是 `Relay`，都断开是 `Unknown`（离线）。
//! 3. 发送：所有包都经中继发出（局域网已去掉，云端是唯一路径）。

use std::time::{Duration, Instant};

pub const RELAY_HAVE_PERIOD: Duration = Duration::from_secs(30);

/// 包从哪条路径来。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Relay,
}

/// 同伴当前的联系路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Relay,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
struct Seen {
    at: Instant,
    period: Duration,
}

impl Seen {
    fn alive(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) < self.period * 3
    }
}

/// 一位同伴在路径上的最近出现情况。
#[derive(Clone, Copy, Debug, Default)]
pub struct PeerPaths {
    relay: Option<Seen>,
}

impl PeerPaths {
    /// 收到这位同伴的包；Have 带上对方报的周期，其他包传 None。
    pub fn seen(&mut self, via: Via, now: Instant, period: Option<Duration>) {
        let (slot, default) = match via {
            Via::Relay => (&mut self.relay, RELAY_HAVE_PERIOD),
        };
        let period = period.or(slot.map(|s| s.period)).unwrap_or(default);
        *slot = Some(Seen { at: now, period });
    }

    pub fn kind(&self, now: Instant) -> PathKind {
        if self.relay.is_some_and(|s| s.alive(now)) {
            PathKind::Relay
        } else {
            PathKind::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn path_follows_where_the_peer_was_seen() {
        let t0 = Instant::now();
        let mut p = PeerPaths::default();
        assert_eq!(p.kind(t0), PathKind::Unknown);

        p.seen(Via::Relay, t0, Some(RELAY_HAVE_PERIOD));
        assert_eq!(p.kind(t0), PathKind::Relay);

        // 中继 3 个周期（90 秒）没消息就断开
        assert_eq!(p.kind(t0 + secs(89)), PathKind::Relay);
        assert_eq!(p.kind(t0 + secs(91)), PathKind::Unknown);
    }

    #[test]
    fn period_comes_from_the_peers_have() {
        let t0 = Instant::now();
        let mut p = PeerPaths::default();
        p.seen(Via::Relay, t0, Some(secs(10)));
        // 非 Have 的包只刷新时间，周期沿用对方上次报的
        p.seen(Via::Relay, t0 + secs(5), None);
        assert_eq!(p.kind(t0 + secs(34)), PathKind::Relay);
        assert_eq!(p.kind(t0 + secs(36)), PathKind::Unknown);
    }

    #[test]
    fn unknown_period_uses_the_path_default() {
        let t0 = Instant::now();
        let mut p = PeerPaths::default();
        p.seen(Via::Relay, t0, None);
        assert_eq!(p.kind(t0 + secs(89)), PathKind::Relay);
        assert_eq!(p.kind(t0 + secs(91)), PathKind::Unknown);
    }
}
