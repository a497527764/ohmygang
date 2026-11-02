//! 节点经中继互连的集成测试：连本地假中继，局域网用 127.0.0.1 上的真实 UDP，定时器缩短。

use std::net::SocketAddr;
use std::time::Duration;

use super::tests::{
    TestNode, burst, create, eventually, fast_config, free_ports, join, remove_db, stacks,
    start_with, status_is, temp_db,
};
use super::*;
use crate::app_core::{RoomStatus, SettingsPatch};
use crate::chat::TimelineItem;
use crate::sync::path::PathKind;
use crate::transport::fake_relay::{Behavior, FakeRelay};
use crate::transport::relay::RelayConfig;

/// `lan` 为 None 时不走局域网（只连中继）。
fn relay_config(me: SocketAddr, lan: Option<&[SocketAddr]>, relay: &str) -> NodeConfig {
    NodeConfig {
        relay: RelayConfig {
            echo_timeout: Duration::from_secs(2),
            backoff: (Duration::from_millis(50), Duration::from_millis(200)),
            cooldown: Duration::from_millis(300),
            connect_timeout: Duration::from_secs(1),
            ..RelayConfig::default()
        },
        relay_urls: Some(vec![relay.to_string()]),
        relay_have_period: Duration::from_millis(300),
        relay_backfill_per_sec: 1000,
        ..fast_config(me, lan.unwrap_or(&[me]))
    }
}

async fn chats(node: &TestNode, room: RoomId) -> usize {
    node.handle
        .call(move |c, _| {
            c.list_messages(&room, None, 100)
                .unwrap()
                .iter()
                .filter(|i| matches!(i, TimelineItem::Chat(_)))
                .count()
        })
        .await
        .unwrap()
}

async fn path_of(node: &TestNode, room: RoomId, member: MemberId) -> PathKind {
    node.handle
        .inspect(move |n| {
            n.member_paths(&room)
                .into_iter()
                .find(|(m, _)| *m == member)
                .map_or(PathKind::Unknown, |(_, p)| p)
        })
        .await
        .unwrap()
}

async fn member_id(node: &TestNode) -> MemberId {
    node.handle
        .call(|c, _| c.identity().member_id)
        .await
        .unwrap()
}

/// 每 20 毫秒查一次，5 秒内条件成立就返回 true。
async fn wait_for(cond: impl Fn() -> bool) -> bool {
    for _ in 0..250 {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// 每 20 毫秒查一次路径，5 秒内变成 `want` 就返回 true。
async fn path_becomes(node: &TestNode, room: RoomId, member: MemberId, want: PathKind) -> bool {
    for _ in 0..250 {
        if path_of(node, room, member).await == want {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_only_pair_joins_chats_and_agrees_on_stacks() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let addrs = free_ports(2);
    let (da, db) = (temp_db(), temp_db());
    let a = start_with(&da, 1, relay_config(addrs[0], None, &relay.url)).await;
    let b = start_with(&db, 2, relay_config(addrs[1], None, &relay.url)).await;

    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(
        eventually(&b, status_is(room, RoomStatus::Active)).await,
        "只经中继也能激活"
    );
    let a_id = member_id(&a).await;
    assert!(path_becomes(&b, room, a_id, PathKind::Relay).await);

    burst(&a, room, 3).await;
    let board = a.handle.call(move |c, _| stacks(c, &room)).await.unwrap();
    assert_eq!(board.iter().map(|(_, s)| *s).max(), Some(1));
    assert!(
        eventually(&b, move |c| stacks(c, &room) == board).await,
        "B 的层数应与 A 一致"
    );
    assert_eq!(chats(&b, room).await, 3);

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn lan_pair_and_relay_only_member_agree() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let addrs = free_ports(3);
    let lan = &addrs[0..2];
    let dbs = [temp_db(), temp_db(), temp_db()];
    let a = start_with(&dbs[0], 1, relay_config(addrs[0], Some(lan), &relay.url)).await;
    let b = start_with(&dbs[1], 2, relay_config(addrs[1], Some(lan), &relay.url)).await;
    let c = start_with(&dbs[2], 3, relay_config(addrs[2], None, &relay.url)).await;

    let (room, code) = create(&a).await;
    join(&b, &code).await;
    join(&c, &code).await;
    for n in [&b, &c] {
        assert!(eventually(n, status_is(room, RoomStatus::Active)).await);
    }
    let (b_id, c_id) = (member_id(&b).await, member_id(&c).await);
    assert!(
        path_becomes(&a, room, b_id, PathKind::Lan).await,
        "A 经局域网看到 B"
    );
    assert!(
        path_becomes(&a, room, c_id, PathKind::Relay).await,
        "A 经中继看到 C"
    );

    // A、B 之间的聊天也经中继到达 C
    burst(&a, room, 3).await;
    burst(&b, room, 3).await;
    let two_stacked =
        move |c: &mut Core| stacks(c, &room).iter().filter(|(_, s)| *s == 1).count() == 2;
    assert!(eventually(&a, two_stacked).await, "A 应看到 A、B 各 1 层");
    let board = a.handle.call(move |c, _| stacks(c, &room)).await.unwrap();
    let both = board.clone();
    assert!(eventually(&b, move |c| stacks(c, &room) == both).await);
    let both = board.clone();
    assert!(
        eventually(&c, move |c| stacks(c, &room) == both).await,
        "C 的层数应与 A、B 一致"
    );
    assert_eq!(chats(&c, room).await, 6);

    burst(&c, room, 3).await;
    let board = c.handle.call(move |c, _| stacks(c, &room)).await.unwrap();
    assert_eq!(board.iter().filter(|(_, s)| *s == 1).count(), 3);
    for n in [&a, &b] {
        let want = board.clone();
        assert!(
            eventually(n, move |c| stacks(c, &room) == want).await,
            "C 的叠层经中继到达"
        );
    }

    drop((a, b, c));
    for d in &dbs {
        remove_db(d);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn lan_pair_keeps_chat_off_the_relay() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let addrs = free_ports(2);
    let (da, db) = (temp_db(), temp_db());
    let quiet = |me| NodeConfig {
        relay_have_period: Duration::from_secs(60),
        ..relay_config(me, Some(&addrs), &relay.url)
    };
    let a = start_with(&da, 1, quiet(addrs[0])).await;
    let b = start_with(&db, 2, quiet(addrs[1])).await;

    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(eventually(&b, status_is(room, RoomStatus::Active)).await);
    let (a_id, b_id) = (member_id(&a).await, member_id(&b).await);
    assert!(path_becomes(&a, room, b_id, PathKind::Lan).await);
    assert!(path_becomes(&b, room, a_id, PathKind::Lan).await);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let before = relay.accepted();
    burst(&a, room, 3).await;
    assert!(eventually(&b, move |c| stacks(c, &room).iter().any(|(_, s)| *s == 1)).await);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        relay.accepted(),
        before,
        "全员在局域网时聊天和叠层不发到中继"
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_internet_off_drops_relay_only_members() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let addrs = free_ports(3);
    let lan = &addrs[0..2];
    let dbs = [temp_db(), temp_db(), temp_db()];
    let a = start_with(&dbs[0], 1, relay_config(addrs[0], Some(lan), &relay.url)).await;
    let b = start_with(&dbs[1], 2, relay_config(addrs[1], Some(lan), &relay.url)).await;
    let c = start_with(&dbs[2], 3, relay_config(addrs[2], None, &relay.url)).await;

    let (room, code) = create(&a).await;
    join(&b, &code).await;
    join(&c, &code).await;
    let (b_id, c_id) = (member_id(&b).await, member_id(&c).await);
    assert!(path_becomes(&a, room, c_id, PathKind::Relay).await);
    assert!(wait_for(|| relay.connections() == 3).await);

    a.handle
        .call(|core, out| {
            core.update_settings(SettingsPatch {
                internet_allowed: Some(false),
                relays: None,
            })
            .unwrap();
            out.settings_changed = true;
        })
        .await
        .unwrap();
    assert_eq!(a.handle.inspect(|n| n.relay_status()).await, Some(None));
    assert!(
        path_becomes(&a, room, c_id, PathKind::Unknown).await,
        "C 只在中继，变为离线"
    );
    assert_eq!(
        path_of(&a, room, b_id).await,
        PathKind::Lan,
        "局域网成员不受影响"
    );
    assert!(wait_for(|| relay.connections() == 2).await, "A 不再连中继");

    drop((a, b, c));
    for d in &dbs {
        remove_db(d);
    }
}
