//! 节点经中继互连的集成测试：连本地假中继，定时器缩短。局域网已去掉，中继是唯一路径。

use std::time::Duration;

use super::tests::{
    TestNode, create, eventually, fast_config, in_room, join, member_id, ping, remove_db,
    start_with, status_is, temp_db, wait_for,
};
use super::*;
use crate::app_core::RoomStatus;
use crate::sync::path::PathKind;
use crate::transport::fake_relay::{Behavior, FakeRelay};

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
    let (da, db) = (temp_db(), temp_db());
    let a = start_with(&da, 1, fast_config(&relay.url)).await;
    let b = start_with(&db, 2, fast_config(&relay.url)).await;

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
async fn three_nodes_sync_and_ping_across_relay() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let dbs = [temp_db(), temp_db(), temp_db()];
    let a = start_with(&dbs[0], 1, fast_config(&relay.url)).await;
    let b = start_with(&dbs[1], 2, fast_config(&relay.url)).await;
    let c = start_with(&dbs[2], 3, fast_config(&relay.url)).await;

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
    assert!(path_becomes(&a, room, b_id, PathKind::Relay).await);
    assert!(path_becomes(&a, room, c_id, PathKind::Relay).await);
    for n in [&a, &b, &c] {
        assert!(eventually(n, in_room(room, 3)).await, "三人互相知道已进队");
    }

    a.handle.send_live(ping("danger"));
    assert!(
        wait_for(|| b.lives_from(a_id).iter().any(|(_, via)| *via == Via::Relay)).await,
        "乙经中继收到"
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
async fn turning_internet_off_makes_all_peers_offline() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let dbs = [temp_db(), temp_db(), temp_db()];
    let a = start_with(&dbs[0], 1, fast_config(&relay.url)).await;
    let b = start_with(&dbs[1], 2, fast_config(&relay.url)).await;
    let c = start_with(&dbs[2], 3, fast_config(&relay.url)).await;

    let (room, code) = create(&a).await;
    join(&b, &code).await;
    join(&c, &code).await;
    let (b_id, c_id) = (member_id(&b).await, member_id(&c).await);
    assert!(path_becomes(&a, room, b_id, PathKind::Relay).await);
    assert!(path_becomes(&a, room, c_id, PathKind::Relay).await);
    assert!(wait_for(|| relay.connections() == 3).await);

    a.handle.set_relays(Vec::new()).await;
    assert_eq!(a.handle.inspect(|n| n.relay_status()).await, Some(None));
    assert!(
        path_becomes(&a, room, c_id, PathKind::Unknown).await,
        "关了互联网，丙变为离线"
    );
    assert!(
        path_becomes(&a, room, b_id, PathKind::Unknown).await,
        "关了互联网，乙也变为离线"
    );
    assert!(wait_for(|| relay.connections() == 2).await, "甲不再连中继");

    drop((a, b, c));
    for d in &dbs {
        remove_db(d);
    }
}
