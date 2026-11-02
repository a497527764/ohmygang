//! 双节点（三节点）集成测试：在 127.0.0.1 上用真实 UDP 互连，定时器缩短到几十毫秒。

use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::app_core::{CoreEvent, RoomStatus, RuleParams};
use crate::keystore::MemoryKeyStore;
use crate::storage;

pub(super) const PARAMS: RuleParams = RuleParams {
    window_ms: 5000,
    threshold: 3,
    cooldown_ms: 30_000,
};

/// 先占几个空闲端口再释放，让每个节点事先知道彼此的地址。
pub(super) fn free_ports(n: usize) -> Vec<SocketAddr> {
    let socks: Vec<UdpSocket> = (0..n)
        .map(|_| UdpSocket::bind("127.0.0.1:0").unwrap())
        .collect();
    socks.iter().map(|s| s.local_addr().unwrap()).collect()
}

pub(super) fn fast_config(me: SocketAddr, all: &[SocketAddr]) -> NodeConfig {
    NodeConfig {
        lan: LanConfig {
            bind: me,
            fixed_targets: Some(all.iter().copied().filter(|a| *a != me).collect()),
            ..LanConfig::default()
        },
        have_period: Duration::from_millis(200),
        live_have_delay: (Duration::from_millis(10), Duration::from_millis(50)),
        new_peer_have_delay: (Duration::from_millis(0), Duration::from_millis(20)),
        want_timeout: Duration::from_millis(300),
        want_max_tries: 3,
        backfill_per_sec: 1000,
        tick: Duration::from_millis(20),
        relay_urls: Some(Vec::new()),
        ..NodeConfig::default()
    }
}

pub(super) struct TestNode {
    pub(super) handle: NodeHandle,
    pub(super) events: Arc<Mutex<Vec<NodeEvent>>>,
}

pub(super) fn temp_db() -> PathBuf {
    let tag = crate::crypto::random_bytes::<8>()
        .map(u64::from_le_bytes)
        .unwrap();
    std::env::temp_dir().join(format!("lolgang-node-{tag}.db"))
}

pub(super) fn remove_db(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

async fn start(db: &Path, key: u8, me: SocketAddr, all: &[SocketAddr]) -> TestNode {
    start_with(db, key, fast_config(me, all)).await
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

pub(super) fn stacks(core: &mut Core, room: &RoomId) -> Vec<(MemberId, u32)> {
    core.leaderboard(room).unwrap()
}

pub(super) async fn create(node: &TestNode) -> (RoomId, String) {
    node.handle
        .call(|core, out| {
            let c = core.create_room("开黑", PARAMS, now_ms()).unwrap();
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

pub(super) async fn burst(node: &TestNode, room: RoomId, n: usize) {
    for i in 0..n {
        node.handle
            .call(move |core, out| {
                let s = core.send_text(&room, &format!("冲 {i}"), now_ms()).unwrap();
                out.broadcast(room, s.events);
                out.notes.extend(s.notes.into_iter().map(NodeEvent::Core));
            })
            .await
            .unwrap();
    }
}

pub(super) fn status_is(
    room: RoomId,
    want: RoomStatus,
) -> impl Fn(&mut Core) -> bool + Send + Sync + Clone + 'static {
    move |core: &mut Core| core.room_status(&room).ok() == Some(want)
}

fn live_count(node: &TestNode) -> usize {
    node.events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, NodeEvent::Core(CoreEvent::HeartsteelLive { .. })))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_join_chat_and_agree_on_stacks() {
    let addrs = free_ports(2);
    let (da, db) = (temp_db(), temp_db());
    let a = start(&da, 1, addrs[0], &addrs).await;
    let b = start(&db, 2, addrs[1], &addrs).await;

    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(
        eventually(&b, status_is(room, RoomStatus::Active)).await,
        "B 应从 Joining 变成 Active"
    );
    assert!(
        eventually(&a, move |c| c
            .list_members(&room)
            .unwrap()
            .iter()
            .filter(|m| m.in_room)
            .count()
            == 2)
        .await,
        "A 应看到 B 加入"
    );

    burst(&a, room, 3).await;
    assert!(
        eventually(&b, move |c| c
            .list_messages(&room, None, 50)
            .unwrap()
            .iter()
            .filter(|i| matches!(i, crate::chat::TimelineItem::Chat(_)))
            .count()
            == 3)
        .await,
        "B 应实时收到 3 条聊天"
    );
    let a_board = a.handle.call(move |c, _| stacks(c, &room)).await.unwrap();
    assert_eq!(a_board.iter().map(|(_, s)| *s).max(), Some(1));
    assert!(
        eventually(&b, move |c| stacks(c, &room) == a_board).await,
        "B 算出的层数应与 A 一致"
    );
    assert!(live_count(&b) >= 1, "B 实时收到叠层应播放音效");

    burst(&b, room, 3).await;
    let b_board = b.handle.call(move |c, _| stacks(c, &room)).await.unwrap();
    assert_eq!(b_board.iter().filter(|(_, s)| *s == 1).count(), 2);
    assert!(
        eventually(&a, move |c| stacks(c, &room) == b_board).await,
        "两边排行榜应完全一致"
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn late_joiner_catches_up_state_without_old_sounds() {
    let addrs = free_ports(3);
    let dbs = [temp_db(), temp_db(), temp_db()];
    let a = start(&dbs[0], 1, addrs[0], &addrs).await;
    let b = start(&dbs[1], 2, addrs[1], &addrs).await;
    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(eventually(&b, status_is(room, RoomStatus::Active)).await);
    burst(&a, room, 3).await;
    burst(&b, room, 3).await;
    let both_stacked =
        move |c: &mut Core| stacks(c, &room).iter().filter(|(_, s)| *s == 1).count() == 2;
    assert!(eventually(&a, both_stacked).await, "A 应看到两人各 1 层");
    assert!(eventually(&b, both_stacked).await, "B 应看到两人各 1 层");

    let c = start(&dbs[2], 3, addrs[2], &addrs).await;
    join(&c, &code).await;
    assert!(
        eventually(&c, status_is(room, RoomStatus::Active)).await,
        "C 只靠补齐也能激活"
    );
    let board = a.handle.call(move |c, _| stacks(c, &room)).await.unwrap();
    assert!(
        eventually(&c, move |core| stacks(core, &room)
            .into_iter()
            .filter(|(_, s)| *s > 0)
            .collect::<Vec<_>>()
            == board
                .iter()
                .copied()
                .filter(|(_, s)| *s > 0)
                .collect::<Vec<_>>())
        .await,
        "C 看到的层数应与 A、B 一致"
    );
    assert_eq!(live_count(&c), 0, "补齐回来的历史叠层不播放音效");

    drop((a, b, c));
    for d in &dbs {
        remove_db(d);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn restarted_node_reconnects_and_stays_consistent() {
    let addrs = free_ports(2);
    let (da, db) = (temp_db(), temp_db());
    let a = start(&da, 1, addrs[0], &addrs).await;
    let b = start(&db, 2, addrs[1], &addrs).await;
    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(eventually(&b, status_is(room, RoomStatus::Active)).await);
    burst(&a, room, 3).await;
    let board = a.handle.call(move |c, _| stacks(c, &room)).await.unwrap();
    assert!(eventually(&b, move |c| stacks(c, &room) == board).await);

    drop(b);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let b = start(&db, 2, addrs[1], &addrs).await;
    assert_eq!(
        b.handle
            .call(move |c, _| c.room_status(&room).unwrap())
            .await,
        Some(RoomStatus::Active)
    );
    burst(&b, room, 3).await;
    let board = b.handle.call(move |c, _| stacks(c, &room)).await.unwrap();
    assert!(
        eventually(&a, move |c| stacks(c, &room) == board).await,
        "重启后的 B 叠层，A 也能收到"
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn member_offline_during_dissolve_learns_it_later() {
    let addrs = free_ports(2);
    let (da, db) = (temp_db(), temp_db());
    let a = start(&da, 1, addrs[0], &addrs).await;
    let b = start(&db, 2, addrs[1], &addrs).await;
    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(eventually(&b, status_is(room, RoomStatus::Active)).await);

    drop(b);
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.handle
        .call(move |core, out| {
            let written = core.dissolve_room(&room, now_ms()).unwrap();
            out.broadcast(room, written);
        })
        .await
        .unwrap();
    assert_eq!(
        a.handle
            .call(move |c, _| c.room_status(&room).unwrap())
            .await,
        Some(RoomStatus::Dissolved)
    );

    let b = start(&db, 2, addrs[1], &addrs).await;
    assert!(
        eventually(&b, status_is(room, RoomStatus::Dissolved)).await,
        "B 上线后应从 A 的回应得知解散"
    );

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}

#[tokio::test(flavor = "multi_thread")]
async fn busy_lan_port_falls_back_to_a_random_port() {
    let taken = UdpSocket::bind("127.0.0.1:0").unwrap();
    let busy = taken.local_addr().unwrap();
    let free = free_ports(1)[0];
    let (d1, d2) = (temp_db(), temp_db());
    let degraded = start(&d1, 1, busy, &[busy]).await;
    let normal = start(&d2, 2, free, &[free]).await;

    let s = degraded.handle.inspect(|n| n.lan_status()).await.unwrap();
    assert!(s.degraded, "端口被占时照常启动，但报告降级");
    assert_ne!(s.port, busy.port());
    let s = normal.handle.inspect(|n| n.lan_status()).await.unwrap();
    assert_eq!((s.port, s.degraded), (free.port(), false));

    drop((degraded, normal, taken));
    remove_db(&d1);
    remove_db(&d2);
}

#[tokio::test(flavor = "multi_thread")]
async fn unconfirmed_message_is_resent_and_then_marked_sent() {
    let addrs = free_ports(2);
    let (da, db) = (temp_db(), temp_db());
    let quick_resend = NodeConfig {
        resend_after: [
            Duration::from_millis(300),
            Duration::from_millis(600),
            Duration::from_millis(1200),
        ],
        ..fast_config(addrs[0], &addrs)
    };
    let a = start_with(&da, 1, quick_resend).await;
    let b = start(&db, 2, addrs[1], &addrs).await;
    let (room, code) = create(&a).await;
    join(&b, &code).await;
    assert!(eventually(&b, status_is(room, RoomStatus::Active)).await);

    // B 掉线期间 A 发言，第一次广播 B 收不到
    drop(b);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let id = a
        .handle
        .call(move |core, out| {
            let s = core.send_text(&room, "你在吗", now_ms()).unwrap();
            out.broadcast(room, s.events);
            s.message.event_id
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let b = start(&db, 2, addrs[1], &addrs).await;

    let has_it = move |c: &mut Core| {
        c.list_messages(&room, None, 50)
            .unwrap()
            .iter()
            .filter(|i| matches!(i, crate::chat::TimelineItem::Chat(m) if m.event_id == id))
            .count()
            == 1
    };
    assert!(eventually(&b, has_it).await, "重发后 B 收到");
    let sent = move |c: &mut Core| {
        c.list_messages(&room, None, 50).unwrap().iter().any(|i| {
            matches!(i, crate::chat::TimelineItem::Chat(m) if m.event_id == id && m.delivery == Some(crate::storage::events::Delivery::Sent))
        })
    };
    assert!(
        eventually(&a, sent).await,
        "B 的 Have 确认后 A 标记为已发送"
    );
    assert!(
        a.events.lock().unwrap().iter().any(|e| matches!(
            e,
            NodeEvent::Delivered { room_id, event_ids } if *room_id == room && event_ids.contains(&id)
        )),
        "通知界面已发送"
    );

    // 所有重发都过去后，B 仍然只有一条，界面也只收到一次
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(b.handle.call(move |c, _| has_it(c)).await, Some(true));
    let shown = b
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, NodeEvent::Core(CoreEvent::TimelineItemAdded { item: crate::chat::TimelineItem::Chat(m), .. }) if m.event_id == id))
        .count();
    assert_eq!(shown, 1, "重复投递不产生重复气泡");

    drop((a, b));
    remove_db(&da);
    remove_db(&db);
}
