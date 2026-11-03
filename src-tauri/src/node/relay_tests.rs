//! 节点经中继互连的集成测试：连本地假中继，局域网用 127.0.0.1 上的真实 UDP，定时器缩短。

use std::net::SocketAddr;
use std::time::Duration;

use super::tests::{
    TestNode, create, eventually, fast_config, free_ports, in_room, join, member_id, ping,
    remove_db, start_with, status_is, temp_db, wait_for,
};
use super::*;
use crate::app_core::RoomStatus;
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
        relay_urls: vec![relay.to_string()],
        relay_have_period: Duration::from_millis(300),
        relay_backfill_per_sec: 1000,
        ..fast_config(me, lan.unwrap_or(&[me]))
    }
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
async fn relay_only_pair_forms_a_team_and_pings() {
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
    let (a_id, b_id) = (member_id(&a).await, member_id(&b).await);
    assert!(path_becomes(&b, room, a_id, PathKind::Relay).await);
    assert!(eventually(&a, in_room(room, 2)).await, "甲应看到乙进队");
    assert!(path_becomes(&a, room, b_id, PathKind::Relay).await);

    a.handle.send_live(ping("allin"));
    assert!(
        wait_for(|| b
            .lives_from(a_id)
            .iter()
            .any(|(body, via)| *body == ping("allin") && *via == Via::Relay))
        .await,
        "信号经中继到达"
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn lan_pair_and_relay_only_member_all_get_pings() {
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
    let (a_id, b_id, c_id) = (
        member_id(&a).await,
        member_id(&b).await,
        member_id(&c).await,
    );
    assert!(
        path_becomes(&a, room, b_id, PathKind::Lan).await,
        "甲经局域网看到乙"
    );
    assert!(
        path_becomes(&a, room, c_id, PathKind::Relay).await,
        "甲经中继看到丙"
    );
    for n in [&a, &b, &c] {
        assert!(eventually(n, in_room(room, 3)).await, "三人互相知道已进队");
    }

    a.handle.send_live(ping("danger"));
    assert!(
        wait_for(|| b.lives_from(a_id).iter().any(|(_, via)| *via == Via::Lan)).await,
        "乙经局域网收到"
    );
    assert!(
        wait_for(|| c.lives_from(a_id).iter().any(|(_, via)| *via == Via::Relay)).await,
        "丙经中继收到"
    );
    c.handle.send_live(ping("omw"));
    assert!(wait_for(|| a.lives_from(c_id).len() == 1 && b.lives_from(c_id).len() == 1).await);

    drop((a, b, c));
    for d in &dbs {
        remove_db(d);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn lan_pair_keeps_pings_off_the_relay() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let addrs = free_ports(2);
    let (da, db) = (temp_db(), temp_db());
    let quiet = |me| NodeConfig {
        relay_have_period: Duration::from_secs(60),
        relay_presence_period: Duration::from_secs(60),
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
    for id in ["danger", "push", "omw"] {
        a.handle.send_live(ping(id));
    }
    assert!(wait_for(|| b.lives_from(a_id).len() == 3).await);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(relay.accepted(), before, "全员在局域网时信号不发到中继");

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

    a.handle.set_relays(Vec::new()).await;
    assert_eq!(a.handle.inspect(|n| n.relay_status()).await, Some(None));
    assert!(
        path_becomes(&a, room, c_id, PathKind::Unknown).await,
        "丙只在中继，变为离线"
    );
    assert_eq!(
        path_of(&a, room, b_id).await,
        PathKind::Lan,
        "局域网队员不受影响"
    );
    assert!(wait_for(|| relay.connections() == 2).await, "甲不再连中继");

    drop((a, b, c));
    for d in &dbs {
        remove_db(d);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cursor_frames_reach_the_relay_only_a_few_times_a_second() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let addrs = free_ports(2);
    let (da, db) = (temp_db(), temp_db());
    let a = start_with(&da, 1, relay_config(addrs[0], None, &relay.url)).await;
    let b = start_with(&db, 2, relay_config(addrs[1], None, &relay.url)).await;
    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(eventually(&a, in_room(room, 2)).await);
    let a_id = member_id(&a).await;
    let b_id = member_id(&b).await;
    assert!(path_becomes(&a, room, b_id, PathKind::Relay).await);

    // 一秒发 30 帧，中继上只该有三四帧，最后一帧一定到
    for i in 0..30u16 {
        a.handle.send_live(LiveBody::Cursor {
            display: 1,
            x: i,
            y: i,
        });
        tokio::time::sleep(Duration::from_millis(33)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let cursors: Vec<u16> = b
        .lives_from(a_id)
        .iter()
        .filter_map(|(body, _)| match body {
            LiveBody::Cursor { x, .. } => Some(*x),
            _ => None,
        })
        .collect();
    assert!(
        (2..=6).contains(&cursors.len()),
        "经中继收到 {} 帧",
        cursors.len()
    );
    assert_eq!(cursors.last(), Some(&29), "最新的一帧不丢");

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}
