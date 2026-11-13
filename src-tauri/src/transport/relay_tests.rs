//! 中继连接池测试：连本地假中继，超时和退避缩短到几百毫秒。

use std::time::Duration;

use tokio::time::{sleep, timeout};

use super::budget::Priority;
use super::fake_relay::{Behavior, FakeRelay};
use super::relay::{RelayConfig, RelayHandle, RelayInbox, RelayPacket, RelayPool, RelayState};
use crate::crypto::RoomId;

const R1: RoomId = [1; 16];
const R2: RoomId = [2; 16];

fn fast() -> RelayConfig {
    RelayConfig {
        echo_timeout: Duration::from_millis(400),
        backoff: (Duration::from_millis(50), Duration::from_millis(200)),
        cooldown: Duration::from_millis(300),
        connect_timeout: Duration::from_secs(1),
        ..RelayConfig::default()
    }
}

fn start(urls: &[&str], rooms: &[RoomId]) -> (RelayHandle, RelayInbox) {
    RelayPool::start(
        urls.iter().map(|u| u.to_string()).collect(),
        rooms.to_vec(),
        fast(),
    )
}

/// 最多等 3 秒，条件成立返回 true。
async fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
    for _ in 0..150 {
        if cond() {
            return true;
        }
        sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn recv_within(inbox: &mut RelayInbox, ms: u64) -> Option<RelayPacket> {
    timeout(Duration::from_millis(ms), inbox.recv())
        .await
        .ok()
        .flatten()
}

fn state(h: &RelayHandle, i: usize) -> RelayState {
    h.status()[i].state
}

#[tokio::test]
async fn packets_reach_pools_in_the_same_room() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (a, mut a_in) = start(&[&relay.url], &[R1]);
    let (_b, mut b_in) = start(&[&relay.url], &[R1]);
    let (_c, mut c_in) = start(&[&relay.url], &[R2]);
    assert!(wait_until(|| relay.subscribers(&R1) == 2 && relay.subscribers(&R2) == 1).await);

    a.publish(R1, vec![9; 20], Priority::Live);
    let got = recv_within(&mut b_in, 2000).await.expect("B 应该收到");
    assert_eq!(got.room, R1);
    assert_eq!(got.packet, vec![9; 20]);
    assert!(
        recv_within(&mut c_in, 300).await.is_none(),
        "别的房间收不到"
    );
    assert!(
        recv_within(&mut a_in, 100).await.is_none(),
        "自己的回显不交给节点"
    );
    assert!(wait_until(|| a.status()[0].last_echo_ms.is_some()).await);
    assert_eq!(state(&a, 0), RelayState::Ok);
}

#[tokio::test]
async fn same_event_from_two_relays_arrives_once() {
    let r1 = FakeRelay::start(Behavior::default()).await;
    let r2 = FakeRelay::start(Behavior::default()).await;
    let (a, _a_in) = start(&[&r1.url, &r2.url], &[R1]);
    let (_b, mut b_in) = start(&[&r1.url, &r2.url], &[R1]);
    assert!(wait_until(|| r1.subscribers(&R1) == 2 && r2.subscribers(&R1) == 2).await);

    a.publish(R1, vec![7; 30], Priority::Live);
    assert!(recv_within(&mut b_in, 2000).await.is_some());
    assert!(
        recv_within(&mut b_in, 500).await.is_none(),
        "第二个中继来的同一事件要去重"
    );
    assert_eq!(
        (r1.accepted(), r2.accepted()),
        (1, 1),
        "还在量回显的中继都发"
    );
}

/// 连接真的断了要重连。
#[tokio::test]
async fn a_dead_connection_reconnects() {
    let relay = FakeRelay::start(Behavior {
        kill_on_event: true,
        ..Behavior::default()
    })
    .await;
    let (a, _a_in) = start(&[&relay.url], &[R1]);
    assert!(wait_until(|| relay.subscribers(&R1) == 1).await);

    a.publish(R1, vec![1; 10], Priority::Live);
    assert!(
        wait_until(|| relay.total_connections() >= 2).await,
        "连接被掐断后应该重连"
    );
}

/// 中继不回显，但它活着：不能把它当死连接。
///
/// 这是「很容易显示未连接」的真正成因——我们的云端中继曾经不回显发布者自己的
/// 事件，而客户端把「等不到回显」当成了唯一的死亡判据，于是每发一个包
/// （包括定时发的在线状态）就隔 25 秒断一次线，界面跟着闪一次「未连接」。
/// 回显只是健康度的信号之一：连接还收得到东西，就说明它活着。
#[tokio::test]
async fn no_echo_alone_does_not_kill_the_connection() {
    let relay = FakeRelay::start(Behavior {
        silent: true,
        ..Behavior::default()
    })
    .await;
    let (a, _a_in) = start(&[&relay.url], &[R1]);
    assert!(wait_until(|| relay.subscribers(&R1) == 1).await);

    a.publish(R1, vec![1; 10], Priority::Live);
    // 等过好几倍的 echo_timeout（400 毫秒）。
    sleep(Duration::from_millis(1600)).await;
    assert_eq!(
        relay.total_connections(),
        1,
        "不回显不等于连接死了，不该重连"
    );
    assert_eq!(state(&a, 0), RelayState::Ok, "连接状态还该是正常的");
}

/// 手动跑：连真实的云端中继，验证「发了包也不会因为等不到回显而重连」。
///
/// 这是「很容易显示未连接」那个 bug 的端到端验证。以前每发一个包（包括定时发的
/// 在线状态）就隔 25 秒断一次线，界面跟着闪一次「未连接」；现在应该能一直连着。
///
/// `cargo test --lib real_relay_stays_connected -- --ignored --nocapture`
/// 想指别的中继就设环境变量 `RELAY_URL`。
#[tokio::test]
#[ignore]
async fn real_relay_stays_connected() {
    let url =
        std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://14.103.234.253:31818".to_string());
    let (h, _in) = RelayPool::start(vec![url.clone()], vec![R1], RelayConfig::default());
    assert!(
        wait_until(|| h.status()[0].state == RelayState::Ok).await,
        "连不上 {url}"
    );
    // 每 3 秒发一个包，模拟在线状态这种持续的小包。
    for _ in 0..15 {
        h.publish(R1, vec![1; 40], Priority::Live);
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let st = h.status();
    assert_eq!(st[0].state, RelayState::Ok, "45 秒后还该连着");
    assert_eq!(
        st[0].connects, 1,
        "这 45 秒里不该重连过（连了 {} 次）",
        st[0].connects
    );
    assert!(st[0].last_echo_ms.is_some(), "应该收到回显：{:?}", st[0]);
}

/// 3.2.0-M10（P7）：真实公共中继上的实测，手动跑（要联网，约 5 分钟）：
/// `cargo test --lib real_relays_probe -- --ignored --nocapture`
///
/// A 的连接池设成发给全部默认中继（正式的只发最好的 3 个）：每 400 毫秒一个 1150 字节的探测包（像说话时的语音帧），说 30 秒停 15 秒，共 6 段。
/// B 对每个中继各开一个只连它的连接池，记下每个探测包从每个中继到达的时间，所以「只发其中几个」能从同一次运行里算出来：
/// 前 2 段之后按 A 的回显成功率挑 3 个，后 4 段比较「全发」和「只发这 3 个」各收到多少、多快。
/// 挑定的 3 个在后 4 段里不换（真要改时掉线会换下一个），所以「只发 3 个」的数字偏保守。房间编号是随机的，不碰真队伍。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "连真实的公共中继，手动跑"]
async fn real_relays_probe() {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    const PER_BURST: u32 = 75;
    const BURSTS: u32 = 6;
    const WARMUP: u32 = 2;
    let urls: Vec<String> = crate::settings2::DEFAULT_RELAYS
        .iter()
        .map(|u| u.to_string())
        .collect();
    let room: RoomId = crate::crypto::random_bytes::<16>().unwrap();
    let (a, _a_in) = RelayPool::start(
        urls.clone(),
        vec![room],
        RelayConfig {
            fanout: usize::MAX, // 全发，「只发其中几个」由 B 那边逐个中继的到达时间算
            ..RelayConfig::default()
        },
    );
    let arrivals: Arc<Mutex<HashMap<(usize, u32), Instant>>> = Arc::default();
    let mut listeners = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let (b, mut inbox) =
            RelayPool::start(vec![url.clone()], vec![room], RelayConfig::default());
        let arrivals = arrivals.clone();
        tokio::spawn(async move {
            while let Some(p) = inbox.recv().await {
                if let Some(seq) = p.packet.get(..4).and_then(|b| b.try_into().ok()) {
                    arrivals
                        .lock()
                        .unwrap()
                        .entry((i, u32::from_le_bytes(seq)))
                        .or_insert_with(Instant::now);
                }
            }
        });
        listeners.push(b);
    }
    for _ in 0..40 {
        if a.status()
            .iter()
            .filter(|r| r.state == RelayState::Ok)
            .count()
            >= urls.len() / 2
        {
            break;
        }
        sleep(Duration::from_millis(500)).await;
    }
    sleep(Duration::from_secs(3)).await; // 让 B 的订阅都发出去
    let connected = |h: &RelayHandle| {
        h.status()
            .iter()
            .filter(|r| r.state == RelayState::Ok)
            .count()
    };
    println!(
        "A 连上 {} / {} 个；B 连上 {} 个",
        connected(&a),
        urls.len(),
        listeners.iter().map(connected).sum::<usize>()
    );

    let mut sent_at: HashMap<u32, Instant> = HashMap::new();
    let mut best: Vec<usize> = Vec::new();
    for burst in 0..BURSTS {
        for k in 0..PER_BURST {
            let seq = burst * PER_BURST + k;
            let mut packet = seq.to_le_bytes().to_vec();
            packet.resize(1150, 0x5a);
            sent_at.insert(seq, Instant::now());
            a.publish(room, packet, Priority::Live);
            sleep(Duration::from_millis(400)).await;
        }
        if burst + 1 == WARMUP {
            sleep(Duration::from_secs(5)).await; // 最后几个的回显
            let s = a.status();
            let mut order: Vec<usize> = (0..s.len())
                .filter(|&i| s[i].state == RelayState::Ok)
                .collect();
            order.sort_by_key(|&i| {
                (
                    std::cmp::Reverse(s[i].echo_pct.unwrap_or(0)),
                    s[i].last_echo_ms.unwrap_or(u64::MAX),
                )
            });
            best = order.into_iter().take(3).collect();
            println!("热身后 A 的回显：");
            for r in &s {
                println!(
                    "  {:<40} {:?} 发 {} 回 {} 最近 {:?}% {:?} 毫秒",
                    r.url, r.state, r.sent, r.echoed, r.echo_pct, r.last_echo_ms
                );
            }
            println!(
                "挑的 3 个：{:?}",
                best.iter().map(|&i| urls[i].as_str()).collect::<Vec<_>>()
            );
        }
        sleep(Duration::from_secs(15)).await;
    }
    sleep(Duration::from_secs(15)).await;

    let arrivals = arrivals.lock().unwrap().clone();
    let eval: Vec<u32> = (WARMUP * PER_BURST..BURSTS * PER_BURST).collect();
    let delay = |set: &[usize], seq: u32| -> Option<u128> {
        set.iter()
            .filter_map(|&i| arrivals.get(&(i, seq)))
            .min()
            .map(|t| t.duration_since(sent_at[&seq]).as_millis())
    };
    let report = |name: &str, set: &[usize]| {
        let mut d: Vec<u128> = eval.iter().filter_map(|&s| delay(set, s)).collect();
        d.sort_unstable();
        let pct = |p: usize| {
            d.get((d.len() * p / 100).min(d.len().saturating_sub(1)))
                .copied()
                .unwrap_or(0)
        };
        println!(
            "{name:<24} 收到 {:>3} / {} = {:>5.1}%  延迟中位 {:>4} 毫秒  95% {:>5} 毫秒  最慢 {:>5} 毫秒",
            d.len(),
            eval.len(),
            d.len() as f64 * 100.0 / eval.len() as f64,
            pct(50),
            pct(95),
            d.last().copied().unwrap_or(0)
        );
    };
    let all: Vec<usize> = (0..urls.len()).collect();
    println!("\n后 {} 段（{} 个探测包）：", BURSTS - WARMUP, eval.len());
    report("全发", &all);
    report("只发挑的 3 个", &best);
    report("只发列表前 3 个", &[0, 1, 2]);
    for (i, url) in urls.iter().enumerate() {
        report(url.trim_start_matches("wss://"), &[i]);
    }
    let s = a.status();
    println!("\n结束时 A 的回显：");
    for r in &s {
        println!(
            "  {:<40} {:?} 发 {} 回 {} 最近 {:?}%",
            r.url, r.state, r.sent, r.echoed, r.echo_pct
        );
    }
}

/// 回显统计：转发的中继回显 100%，只收不转发的中继超时断开后算 0%，发出的数都记着。
#[tokio::test]
async fn echo_success_is_counted_per_relay() {
    let good = FakeRelay::start(Behavior::default()).await;
    let silent = FakeRelay::start(Behavior {
        silent: true,
        ..Behavior::default()
    })
    .await;
    let (a, _a_in) = start(&[&good.url, &silent.url], &[R1]);
    assert!(wait_until(|| good.subscribers(&R1) == 1 && silent.subscribers(&R1) == 1).await);
    assert_eq!(a.status()[0].echo_pct, None, "还没发过");

    a.publish(R1, vec![1; 10], Priority::Live);
    a.publish(R1, vec![2; 10], Priority::Live);
    assert!(wait_until(|| a.status()[0].echoed == 2 && a.status()[1].echo_pct.is_some()).await);
    let s = a.status();
    assert_eq!((s[0].sent, s[0].echoed, s[0].echo_pct), (2, 2, Some(100)));
    assert_eq!((s[1].sent, s[1].echoed, s[1].echo_pct), (2, 0, Some(0)));
    assert!(s[0].last_echo_ms.is_some() && s[1].last_echo_ms.is_none());
}

#[tokio::test]
async fn rate_limited_relay_cools_down_then_resumes() {
    let relay = FakeRelay::start(Behavior {
        rate_limit_first: 1,
        ..Behavior::default()
    })
    .await;
    let (a, _a_in) = start(&[&relay.url], &[R1]);
    let (_b, mut b_in) = start(&[&relay.url], &[R1]);
    assert!(wait_until(|| relay.subscribers(&R1) == 2).await);

    a.publish(R1, vec![1; 10], Priority::Live);
    assert!(wait_until(|| state(&a, 0) == RelayState::Cooling).await);
    // 冷却期间的包留在队列里，冷却结束后发出
    a.publish(R1, vec![2; 10], Priority::Live);
    let got = recv_within(&mut b_in, 2000)
        .await
        .expect("冷却结束后应该发出");
    assert_eq!(got.packet, vec![2; 10]);
    assert_eq!(state(&a, 0), RelayState::Ok);
    assert_eq!(relay.accepted(), 1);
}

#[tokio::test]
async fn rejecting_relay_is_retired() {
    let relay = FakeRelay::start(Behavior {
        reject: Some("blocked: not on the list".into()),
        ..Behavior::default()
    })
    .await;
    let (a, _a_in) = start(&[&relay.url], &[R1]);
    assert!(wait_until(|| relay.subscribers(&R1) == 1).await);

    a.publish(R1, vec![1; 10], Priority::Live);
    assert!(wait_until(|| state(&a, 0) == RelayState::Retired).await);
    assert!(
        wait_until(|| relay.connections() == 0).await,
        "停用后断开连接"
    );
    sleep(Duration::from_millis(300)).await;
    assert_eq!(relay.total_connections(), 1, "停用后不再重连");
}

#[tokio::test]
async fn unreachable_relay_does_not_block_others() {
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("ws://{}", l.local_addr().unwrap())
    };
    let relay = FakeRelay::start(Behavior::default()).await;
    let (a, _a_in) = start(&[&dead, &relay.url], &[R1]);
    let (_b, mut b_in) = start(&[&dead, &relay.url], &[R1]);
    assert!(wait_until(|| relay.subscribers(&R1) == 2).await);

    a.publish(R1, vec![5; 10], Priority::Live);
    assert!(recv_within(&mut b_in, 2000).await.is_some());
    assert_eq!(state(&a, 0), RelayState::Connecting);
    assert_eq!(state(&a, 1), RelayState::Ok);
}

#[tokio::test]
async fn changing_rooms_resubscribes() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (a, _a_in) = start(&[&relay.url], &[R1]);
    let (b, mut b_in) = start(&[&relay.url], &[R2]);
    assert!(wait_until(|| relay.subscribers(&R1) == 1 && relay.subscribers(&R2) == 1).await);

    b.set_rooms(vec![R1]);
    assert!(wait_until(|| relay.subscribers(&R1) == 2 && relay.subscribers(&R2) == 0).await);
    a.publish(R1, vec![3; 10], Priority::Live);
    assert_eq!(recv_within(&mut b_in, 2000).await.map(|p| p.room), Some(R1));
}

#[tokio::test]
async fn dropping_the_pool_closes_connections() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (a, _a_in) = start(&[&relay.url], &[R1]);
    assert!(wait_until(|| relay.connections() == 1).await);
    drop(a);
    assert!(wait_until(|| relay.connections() == 0).await);
}

#[tokio::test]
async fn reconnect_now_skips_the_backoff_wait() {
    // 先占一个端口再关掉：连接池连不上，进入很长的退避等待
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let cfg = RelayConfig {
        backoff: (Duration::from_secs(30), Duration::from_secs(60)),
        ..fast()
    };
    let (a, _a_in) = RelayPool::start(vec![format!("ws://{addr}")], vec![R1], cfg);
    sleep(Duration::from_millis(300)).await;
    // 网络「恢复」：中继在同一地址上线，不叫醒的话要等 30 秒
    let relay = FakeRelay::start_on(addr, Behavior::default()).await;
    a.reconnect_now();
    assert!(
        wait_until(|| relay.subscribers(&R1) == 1).await,
        "叫醒后立刻重连"
    );
}
