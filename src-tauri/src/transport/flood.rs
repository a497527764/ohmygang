//! 洪水防护（架构 6.3）：解密之前先限速，挡住有人狂发垃圾包把 CPU 占满。
//!
//! 实现流程：
//! 1. 令牌桶：每个来源 IP 一个桶，每秒回补 100 个令牌、最多存 100 个；另有一个总桶，每秒 800 个。
//!    收到一个包消耗一个令牌，任一个桶不够就丢弃这个包。
//! 2. 令牌用「千分之一个」做单位，整数运算，不用浮点。时间由调用方传入，方便测试。
//! 3. 最多跟踪 1024 个地址，满了就淘汰最久没出现的。

use std::collections::HashMap;
use std::net::IpAddr;

struct Bucket {
    milli: u64,
    last_ms: u64,
}

impl Bucket {
    fn full(rate: u64, now_ms: u64) -> Self {
        Bucket {
            milli: rate * 1000,
            last_ms: now_ms,
        }
    }

    /// 每毫秒回补 rate 个「千分之一令牌」，即每秒 rate 个，上限 rate 个。
    fn refill(&mut self, rate: u64, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.last_ms);
        self.milli = (self.milli + elapsed * rate).min(rate * 1000);
        self.last_ms = self.last_ms.max(now_ms);
    }
}

pub struct FloodGuard {
    per_ip_rate: u64,
    total_rate: u64,
    max_tracked: usize,
    per_ip: HashMap<IpAddr, (Bucket, u64)>,
    total: Bucket,
}

impl Default for FloodGuard {
    fn default() -> Self {
        Self::new(100, 800, 1024)
    }
}

impl FloodGuard {
    pub fn new(per_ip_rate: u64, total_rate: u64, max_tracked: usize) -> Self {
        FloodGuard {
            per_ip_rate,
            total_rate,
            max_tracked,
            per_ip: HashMap::new(),
            total: Bucket::full(total_rate, 0),
        }
    }

    pub fn allow(&mut self, ip: IpAddr, now_ms: u64) -> bool {
        if !self.per_ip.contains_key(&ip) && self.per_ip.len() >= self.max_tracked {
            let oldest = self
                .per_ip
                .iter()
                .min_by_key(|(_, (_, seen))| *seen)
                .map(|(k, _)| *k);
            if let Some(k) = oldest {
                self.per_ip.remove(&k);
            }
        }
        let rate = self.per_ip_rate;
        let (bucket, seen) = self
            .per_ip
            .entry(ip)
            .or_insert_with(|| (Bucket::full(rate, now_ms), now_ms));
        bucket.refill(rate, now_ms);
        *seen = now_ms;
        self.total.refill(self.total_rate, now_ms);
        if bucket.milli < 1000 || self.total.milli < 1000 {
            return false;
        }
        bucket.milli -= 1000;
        self.total.milli -= 1000;
        true
    }

    pub fn tracked(&self) -> usize {
        self.per_ip.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn ip(n: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 168, 0, n))
    }

    #[test]
    fn per_ip_limit_then_refill() {
        let mut g = FloodGuard::default();
        let allowed = (0..150).filter(|_| g.allow(ip(1), 1000)).count();
        assert_eq!(allowed, 100);
        let refilled = (0..100).filter(|_| g.allow(ip(1), 1499)).count();
        assert_eq!(refilled, 49, "过了 499 毫秒只回补 49 个");
        assert!(g.allow(ip(1), 2500), "一秒后恢复");
    }

    #[test]
    fn total_limit_across_ips() {
        let mut g = FloodGuard::default();
        let allowed = (1..=9u8)
            .flat_map(|n| (0..100).map(move |_| n))
            .filter(|n| g.allow(ip(*n), 1000))
            .count();
        assert_eq!(allowed, 800);
    }

    #[test]
    fn evicts_least_recently_seen_address_when_full() {
        let mut g = FloodGuard::new(2, 800, 2);
        assert!(g.allow(ip(1), 0));
        assert!(g.allow(ip(1), 0));
        assert!(!g.allow(ip(1), 0), "ip1 用完了");
        assert!(g.allow(ip(2), 1));
        assert!(g.allow(ip(3), 2), "表满了，淘汰最久没出现的 ip1");
        assert_eq!(g.tracked(), 2);
        assert!(g.allow(ip(1), 3), "ip1 被淘汰后重新开始计数");
    }
}
