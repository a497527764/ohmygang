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
    assert_eq!((r1.accepted(), r2.accepted()), (1, 1), "一个包发给所有中继");
}

#[tokio::test]
async fn missing_echo_reconnects() {
    let relay = FakeRelay::start(Behavior {
        silent: true,
        ..Behavior::default()
    })
    .await;
    let (a, _a_in) = start(&[&relay.url], &[R1]);
    assert!(wait_until(|| relay.subscribers(&R1) == 1).await);

    a.publish(R1, vec![1; 10], Priority::Live);
    assert!(
        wait_until(|| relay.total_connections() >= 2).await,
        "没有回显应该重连"
    );
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
