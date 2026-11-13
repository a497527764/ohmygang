//! 多节点集成测试：经本地假中继互连，定时器缩短到几十毫秒。局域网已去掉，中继是唯一路径。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::app_core::{CoreEvent, RoomStatus};
use crate::keystore::MemoryKeyStore;
use crate::protocol::live::Status;
use crate::storage;
use crate::transport::budget::Priority;
use crate::transport::fake_relay::{Behavior, FakeRelay};
use crate::transport::relay::{RelayConfig, RelayPool, RelayState};

/// 测试用的快速配置：定时器缩短，指向给定中继。
pub(super) fn fast_config(relay: &str) -> NodeConfig {
    NodeConfig {
        relay: RelayConfig {
            echo_timeout: Duration::from_secs(2),
            backoff: (Duration::from_millis(50), Duration::from_millis(200)),
            cooldown: Duration::from_millis(300),
            connect_timeout: Duration::from_secs(1),
            ..RelayConfig::default()
        },
        relay_urls: vec![relay.to_string()],
        relay_have_period: Duration::from_millis(200),
        relay_presence_period: Duration::from_millis(200),
        live_have_delay: (Duration::from_millis(10), Duration::from_millis(50)),
        new_peer_have_delay: (Duration::from_millis(0), Duration::from_millis(20)),
        want_timeout: Duration::from_millis(300),
        want_max_tries: 3,
        relay_backfill_per_sec: 1000,
        tick: Duration::from_millis(20),
        resend_after: [
            Duration::from_millis(300),
            Duration::from_millis(600),
            Duration::from_millis(1200),
        ],
    }
}

pub(super) struct TestNode {
    pub(super) handle: NodeHandle,
    pub(super) events: Arc<Mutex<Vec<NodeEvent>>>,
}

impl TestNode {
    /// 收到过的、来自 `from` 的实时帧。
    pub(super) fn lives_from(&self, from: MemberId) -> Vec<(LiveBody, Via)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                NodeEvent::Live {
                    from: f, body, via, ..
                } if *f == from => Some((body.clone(), *via)),
                _ => None,
            })
            .collect()
    }
}

pub(super) fn temp_db() -> PathBuf {
    let tag = crate::crypto::random_bytes::<8>()
        .map(u64::from_le_bytes)
        .unwrap();
    std::env::temp_dir().join(format!("gang2-node-{tag}.db"))
}

pub(super) fn remove_db(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

async fn start(db: &Path, key: u8, relay: &str) -> TestNode {
    start_with(db, key, fast_config(relay)).await
}

pub(super) async fn start_with(db: &Path, key: u8, cfg: NodeConfig) -> TestNode {
    let core = Core::open(storage::open(db).unwrap(), &MemoryKeyStore([key; 32])).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    let sink: Sink = Arc::new(move |e| sink_events.lock().unwrap().push(e));
    let handle = Node::spawn(core, cfg, sink).await.unwrap();
    TestNode { handle, events }
}

/// 每 20 毫秒查一次，5 秒内条件成立就返回 true。
pub(super) async fn eventually<F>(node: &TestNode, check: F) -> bool
where
    F: Fn(&mut Core) -> bool + Send + Sync + Clone + 'static,
{
    for _ in 0..250 {
        let c = check.clone();
        if node.handle.call(move |core, _| c(core)).await == Some(true) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// 每 20 毫秒查一次，5 秒内条件成立就返回 true。
pub(super) async fn wait_for(cond: impl Fn() -> bool) -> bool {
    for _ in 0..250 {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

pub(super) async fn create(node: &TestNode) -> (RoomId, String) {
    node.handle
        .call(|core, out| {
            let c = core.create_room("开黑", now_ms()).unwrap();
            out.broadcast(c.summary.room_id, c.events);
            (c.summary.room_id, c.code)
        })
        .await
        .unwrap()
}

pub(super) async fn join(node: &TestNode, code: &str) {
    let code = code.to_string();
    node.handle
        .call(move |core, _| core.join_room(&code, now_ms()).unwrap())
        .await
        .unwrap();
}

pub(super) async fn member_id(node: &TestNode) -> MemberId {
    node.handle
        .call(|c, _| c.identity().member_id)
        .await
        .unwrap()
}

pub(super) fn status_is(
    room: RoomId,
    want: RoomStatus,
) -> impl Fn(&mut Core) -> bool + Send + Sync + Clone + 'static {
    move |core: &mut Core| core.room_status(&room).ok() == Some(want)
}

/// 这个节点认为在队里的人数（含自己）。
pub(super) fn in_room(
    room: RoomId,
    n: usize,
) -> impl Fn(&mut Core) -> bool + Send + Sync + Clone + 'static {
    move |c: &mut Core| c.members_in(&room).unwrap().len() == n
}

pub(super) fn ping(id: &str) -> LiveBody {
    LiveBody::Ping {
        display: 1,
        x: 1000,
        y: 2000,
        id: id.into(),
    }
}

fn presence(status: Status) -> LiveBody {
    LiveBody::Presence {
        status,
        laning: false,
        version: "2.0.0".into(),
        platform: "win".into(),
        emotes: Vec::new(),
        off: 0,
        avatar: None,
    }
}

/// 等一条裸中继连接就绪。
async fn wait_connected(relay: &RelayHandle) -> bool {
    wait_for(|| relay.status().iter().any(|r| r.state == RelayState::Ok)).await
}

/// 甲建队、乙进队，等两边都看到对方。
async fn pair(da: &Path, db: &Path, relay: &str) -> (TestNode, TestNode, RoomId) {
    let a = start(da, 1, relay).await;
    let b = start(db, 2, relay).await;
    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(
        eventually(&b, status_is(room, RoomStatus::Active)).await,
        "乙应从 Joining 变成 Active"
    );
    assert!(eventually(&a, in_room(room, 2)).await, "甲应看到乙进队");
    (a, b, room)
}

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_form_a_team_and_ping_each_other() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (da, db) = (temp_db(), temp_db());
    let (a, b, _room) = pair(&da, &db, &relay.url).await;
    let (a_id, b_id) = (member_id(&a).await, member_id(&b).await);
    // 乙要先知道甲是队员（甲的事件早就补齐了）
    a.handle.send_live(ping("danger"));
    assert!(
        wait_for(|| b
            .lives_from(a_id)
            .iter()
            .any(|(body, _)| *body == ping("danger")))
        .await,
        "乙收到甲的信号"
    );
    b.handle.send_live(ping("omw"));
    assert!(
        wait_for(|| a
            .lives_from(b_id)
            .iter()
            .any(|(body, via)| *body == ping("omw") && *via == Via::Relay))
        .await
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        b.lives_from(a_id)
            .iter()
            .filter(|(body, _)| *body == ping("danger"))
            .count(),
        1,
        "同一帧只交一次"
    );
    assert!(b.lives_from(b_id).is_empty(), "自己的广播不算");

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn presence_repeats_and_follows_changes() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (da, db) = (temp_db(), temp_db());
    let (a, b, _room) = pair(&da, &db, &relay.url).await;
    let a_id = member_id(&a).await;
    a.handle.set_presence(presence(Status::Online));
    assert!(
        wait_for(|| b
            .lives_from(a_id)
            .iter()
            .filter(|(body, _)| *body == presence(Status::Online))
            .count()
            >= 3)
        .await,
        "按周期反复发"
    );
    a.handle.set_presence(presence(Status::Paused));
    assert!(
        wait_for(|| b
            .lives_from(a_id)
            .iter()
            .any(|(body, _)| *body == presence(Status::Paused)))
        .await
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn live_frames_from_non_members_and_replays_are_dropped() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (da, db) = (temp_db(), temp_db());
    let (a, b, room) = pair(&da, &db, &relay.url).await;
    let key = a
        .handle
        .call(move |c, _| c.room_key(&room).unwrap())
        .await
        .unwrap();

    // 裸中继连接：模拟拿到了队伍码、却没进队（或录包重发）的人
    let (inject, _inbox) = RelayPool::start(
        vec![relay.url.clone()],
        vec![room],
        fast_config(&relay.url).relay,
    );
    assert!(wait_connected(&inject).await, "注入连接应就绪");

    // 拿到了队伍码、却没进队的人：签名没问题，但不是队员
    let outsider = crate::crypto::Identity::from_secret(&[42u8; 32]);
    let frame = Frame::Live(crate::protocol::live::sign_live(
        &outsider,
        &room,
        1,
        now_ms(),
        &ping("danger"),
    ));
    for p in seal_packets(&key, &room, &frame).unwrap() {
        inject.publish(room, p, Priority::Live);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        a.lives_from(outsider.member_id()).is_empty(),
        "不是队员的帧不要"
    );

    // 队员的帧被人录下来重发：同一个序号只交一次（让乙自己签一帧，再经裸连接发两遍）
    let b_id = member_id(&b).await;
    let wire = b
        .handle
        .call(move |c, _| c.sign_live(&room, 7, now_ms(), &ping("omw")))
        .await
        .unwrap();
    let frame = Frame::Live(wire);
    let packets = seal_packets(&key, &room, &frame).unwrap();
    for p in &packets {
        inject.publish(room, p.clone(), Priority::Live);
    }
    for p in &packets {
        inject.publish(room, p.clone(), Priority::Live); // 录下来重发
    }
    assert!(wait_for(|| a.lives_from(b_id).len() == 1).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(a.lives_from(b_id).len(), 1, "同一帧只交一次");

    drop((a, b, inject));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn late_joiner_catches_up_and_activates() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let dbs = [temp_db(), temp_db(), temp_db()];
    let a = start(&dbs[0], 1, &relay.url).await;
    let b = start(&dbs[1], 2, &relay.url).await;
    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(eventually(&a, in_room(room, 2)).await);

    let c = start(&dbs[2], 3, &relay.url).await;
    join(&c, &code).await;
    assert!(
        eventually(&c, status_is(room, RoomStatus::Active)).await,
        "丙只靠补齐也能激活"
    );
    assert!(eventually(&c, in_room(room, 3)).await);
    assert!(eventually(&a, in_room(room, 3)).await);
    assert!(eventually(&b, in_room(room, 3)).await);

    drop((a, b, c));
    for d in &dbs {
        remove_db(d);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn restarted_node_reconnects_and_keeps_pinging() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (da, db) = (temp_db(), temp_db());
    let (a, b, room) = pair(&da, &db, &relay.url).await;
    let b_id = member_id(&b).await;

    drop(b);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let b = start(&db, 2, &relay.url).await;
    assert_eq!(
        b.handle
            .call(move |c, _| c.room_status(&room).unwrap())
            .await,
        Some(RoomStatus::Active)
    );
    b.handle.send_live(ping("push"));
    assert!(
        wait_for(|| a
            .lives_from(b_id)
            .iter()
            .any(|(body, _)| *body == ping("push")))
        .await,
        "重启后序号仍然比以前的大，甲照收"
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn member_offline_during_dissolve_learns_it_later() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (da, db) = (temp_db(), temp_db());
    let (a, b, room) = pair(&da, &db, &relay.url).await;

    drop(b);
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.handle
        .call(move |core, out| {
            let written = core.dissolve_room(&room, now_ms()).unwrap();
            out.broadcast(room, written);
        })
        .await
        .unwrap();
    let b = start(&db, 2, &relay.url).await;
    assert!(
        eventually(&b, status_is(room, RoomStatus::Dissolved)).await,
        "乙上线后应从甲的回应得知解散"
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn unconfirmed_event_is_resent_and_then_marked_sent() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (da, db) = (temp_db(), temp_db());
    let a = start(&da, 1, &relay.url).await;
    let b = start(&db, 2, &relay.url).await;
    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(eventually(&a, in_room(room, 2)).await);

    // 乙掉线期间甲改队名，第一次广播乙收不到（fast_config 的重发计划会补上）
    drop(b);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let id = a
        .handle
        .call(move |core, out| {
            let written = core.rename_room(&room, "五黑", now_ms()).unwrap();
            let id = written[0].event_id;
            out.broadcast(room, written);
            id
        })
        .await
        .unwrap();
    let b = start(&db, 2, &relay.url).await;
    assert!(
        eventually(&b, move |c| c.summary(&room).unwrap().title.as_deref()
            == Some("五黑"))
        .await,
        "重发或补齐后乙收到"
    );
    assert!(
        eventually(&a, move |c| c.pending_events(&room).unwrap().is_empty()).await,
        "乙的 Have 确认后甲标记为已发送"
    );
    assert!(a.events.lock().unwrap().iter().any(|e| matches!(
        e,
        NodeEvent::Delivered { room_id, event_ids } if *room_id == room && event_ids.contains(&id)
    )));

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn bye_and_leave_go_out_before_the_team_goes_quiet() {
    let relay = FakeRelay::start(Behavior::default()).await;
    let (da, db) = (temp_db(), temp_db());
    let (a, b, room) = pair(&da, &db, &relay.url).await;
    let b_id = member_id(&b).await;

    b.handle
        .call(move |core, out| {
            out.live(room, LiveBody::Bye);
            let written = core.leave_room(&room, now_ms()).unwrap();
            out.broadcast(room, written);
        })
        .await
        .unwrap();
    assert!(
        wait_for(|| a
            .lives_from(b_id)
            .iter()
            .any(|(body, _)| *body == LiveBody::Bye))
        .await,
        "甲收到乙的 Bye"
    );
    assert!(eventually(&a, in_room(room, 1)).await, "甲收到乙的退出");
    assert_eq!(
        b.handle
            .inspect(move |n| n.member_paths(&room).is_empty())
            .await,
        Some(true),
        "退出后乙不再为这个队伍联网"
    );
    // 乙已经不是队员：它再发的帧甲不收（乙不再为这个队伍联网，也发不出去）
    let before = a.lives_from(b_id).len();
    b.handle.send_live(ping("danger"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(a.lives_from(b_id).len(), before);
    assert!(
        a.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, NodeEvent::Core(CoreEvent::RoomUpdated { .. })))
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}
