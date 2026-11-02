//! 发言与叠层测试，包括两个实例交换事件后层数一致、恢复不重复。

use super::ingest_tests::stacks_of;
use super::tests::{DEFAULT_PARAMS, new_core};
use super::*;
use crate::keystore::{MemoryKeyStore, PURPOSE_IDENTITY, encrypt_local};
use crate::protocol::body::CHAT_TEXT_MAX_BYTES;
use crate::storage::{self, identity as id_store};

const MASTER: [u8; 32] = [1u8; 32];

fn has_live(notes: &[CoreEvent]) -> bool {
    notes
        .iter()
        .any(|n| matches!(n, CoreEvent::HeartsteelLive { .. }))
}

/// 建好房间的 A，返回 (A, room_id, 房间码)。
fn owner_with_room() -> (Core, RoomId, String) {
    let mut a = new_core();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    (a, created.summary.room_id, created.code)
}

/// 用同一私钥再开一个本机实例，模拟换机恢复后的同一个人。
fn same_person_new_device(a: &Core) -> Core {
    let conn = storage::open_in_memory().unwrap();
    let row = id_store::IdentityRow {
        member_id: a.me.member_id(),
        nickname: String::new(),
        secret_cipher: encrypt_local(&MASTER, PURPOSE_IDENTITY, &a.me.secret()).unwrap(),
    };
    id_store::insert(&conn, &row).unwrap();
    Core::open(conn, &MemoryKeyStore(MASTER)).unwrap()
}

#[test]
fn three_quick_messages_stack_once() {
    let (mut a, room, _) = owner_with_room();
    let me = a.identity().member_id;
    assert_eq!(a.send_text(&room, "一", 10_000).unwrap().events.len(), 1);
    assert_eq!(a.send_text(&room, "二", 11_000).unwrap().events.len(), 1);
    let third = a.send_text(&room, "三", 12_000).unwrap();
    assert_eq!(third.events.len(), 2, "聊天 + 叠层事件");
    assert_eq!(third.events[1].event.kind, kind::HEARTSTEEL);
    assert!(third.events[1].event.ts_ms > third.events[0].event.ts_ms);
    assert!(third.notes.iter().any(
        |n| matches!(n, CoreEvent::StacksChanged { changes, .. } if changes == &vec![(me, 1)])
    ));
    assert!(has_live(&third.notes));
    assert_eq!(third.message.text, "三");
    assert_eq!(third.message.delivery, Some(Delivery::Pending));
    assert_eq!(a.leaderboard(&room).unwrap(), vec![(me, 1)]);
}

#[test]
fn leaderboard_is_sorted_by_stacks_then_member_id() {
    let (mut a, room, code) = owner_with_room();
    let mut others = Vec::new();
    for _ in 0..2 {
        let mut x = new_core();
        x.join_room(&code, 500).unwrap();
        x.ingest_events(&room, &a.room_events(&room).unwrap(), false, 1500)
            .unwrap();
        let written = x.activate(&room, 2000).unwrap();
        a.ingest_events(&room, &written, false, 2000).unwrap();
        others.push(x);
    }
    for t in [10_000, 11_000, 12_000] {
        a.send_text(&room, "冲", t).unwrap();
        others[1].send_text(&room, "冲", t).unwrap();
    }
    let c_events = others[1].room_events(&room).unwrap();
    a.ingest_events(&room, &c_events, false, 20_000).unwrap();

    let me = a.identity().member_id;
    let b = others[0].identity().member_id;
    let c = others[1].identity().member_id;
    let mut top = [me, c];
    top.sort();
    assert_eq!(
        a.leaderboard(&room).unwrap(),
        vec![(top[0], 1), (top[1], 1), (b, 0)],
        "层数降序，同层按成员 ID 升序"
    );
}

#[test]
fn peer_have_confirms_my_events_up_to_its_contiguous_seq() {
    let (mut a, room, _) = owner_with_room();
    let me = a.identity().member_id;
    let sent: Vec<_> = ["一", "二", "三"]
        .iter()
        .enumerate()
        .map(|(i, t)| a.send_text(&room, t, 10_000 + i as u64 * 10_000).unwrap())
        .collect();
    let pending = |c: &Core| {
        c.list_messages(&room, None, 50)
            .unwrap()
            .iter()
            .filter_map(|i| match i {
                TimelineItem::Chat(m) if m.delivery == Some(Delivery::Pending) => {
                    Some(m.text.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(pending(&a).len(), 3);

    // 同伴连续持有我的前 2 条聊天、全部状态事件
    let peer = vec![HaveEntry {
        author: short_id(&me),
        state_max: 99,
        chat_max: 2,
    }];
    let confirmed = a.mark_delivered(&room, &peer).unwrap();
    assert!(confirmed.contains(&sent[0].message.event_id));
    assert!(confirmed.contains(&sent[1].message.event_id));
    assert!(!confirmed.contains(&sent[2].message.event_id));
    assert_eq!(pending(&a), vec!["三".to_string()]);
    assert!(
        a.pending_events(&room)
            .unwrap()
            .iter()
            .all(|e| e.event.stream == Stream::Chat && e.event.seq == 3),
        "状态事件也被确认了，只剩第 3 条聊天"
    );
    assert!(
        a.mark_delivered(&room, &peer).unwrap().is_empty(),
        "已确认的不再返回"
    );

    // 向量里没有我时什么都不变
    let other = vec![HaveEntry {
        author: [7u8; 8],
        state_max: 99,
        chat_max: 99,
    }];
    assert!(a.mark_delivered(&room, &other).unwrap().is_empty());
}

#[test]
fn cooldown_blocks_then_restack_after() {
    let (mut a, room, _) = owner_with_room();
    let me = a.identity().member_id;
    for t in [10_000, 11_000, 12_000, 13_000, 14_000] {
        a.send_text(&room, "冲", t).unwrap();
    }
    assert_eq!(stacks_of(&a, &room, &me), 1, "冷却内继续发不叠层");
    for t in [42_500, 43_000, 44_000] {
        a.send_text(&room, "冲", t).unwrap();
    }
    assert_eq!(stacks_of(&a, &room, &me), 2, "冷却结束后重新攒满");
}

#[test]
fn two_instances_agree_whatever_the_order() {
    let (mut a, room, code) = owner_with_room();
    let me = a.identity().member_id;
    for t in [
        10_000, 11_000, 12_000, 20_000, 45_000, 46_000, 47_000, 90_000,
    ] {
        a.send_text(&room, "冲", t).unwrap();
    }
    assert_eq!(stacks_of(&a, &room, &me), 2);
    let all = a.room_events(&room).unwrap();

    // 一次收齐、倒序
    let mut b = new_core();
    b.join_room(&code, 500).unwrap();
    let mut rev = all.clone();
    rev.reverse();
    b.ingest_events(&room, &rev, false, 100_000).unwrap();
    assert_eq!(stacks_of(&b, &room, &me), 2);
    assert_eq!(b.leaderboard(&room).unwrap(), a.leaderboard(&room).unwrap());

    // 一条一条、交错顺序到达
    let mut c = new_core();
    c.join_room(&code, 500).unwrap();
    let (odd, even): (Vec<_>, Vec<_>) = all
        .iter()
        .cloned()
        .enumerate()
        .partition(|(i, _)| i % 2 == 1);
    for (_, e) in odd.into_iter().chain(even) {
        c.ingest_events(&room, &[e], true, 100_000).unwrap();
    }
    assert_eq!(stacks_of(&c, &room, &me), 2);
}

#[test]
fn restored_old_messages_do_not_create_stacks() {
    let (mut a, room, code) = owner_with_room();
    let me = a.identity().member_id;
    for t in [10_000, 11_000, 12_000] {
        a.send_text(&room, "冲", t).unwrap();
    }
    // 新设备只同步回本人的状态事件和聊天，没拿到叠层事件
    let synced: Vec<SignedEvent> = a
        .room_events(&room)
        .unwrap()
        .into_iter()
        .filter(|e| e.event.kind != kind::HEARTSTEEL)
        .collect();
    let mut a2 = same_person_new_device(&a);
    assert_eq!(a2.identity().member_id, me);
    a2.join_room(&code, 50_000).unwrap();
    a2.ingest_events(&room, &synced, false, 50_000).unwrap();
    let hs_count = |c: &Core| {
        c.room_events(&room)
            .unwrap()
            .iter()
            .filter(|e| e.event.kind == kind::HEARTSTEEL)
            .count()
    };
    assert_eq!(hs_count(&a2), 0, "同步回来的旧消息不触发叠层");

    // 同伴同步完成后转为 Active（M1-3 的流程），再发一条：旧消息早已超出窗口，不会被补算
    room_store::set_status(&a2.conn, &room, RoomStatus::Active).unwrap();
    a2.send_text(&room, "回来了", 100_000).unwrap();
    assert_eq!(hs_count(&a2), 0);
    assert_eq!(stacks_of(&a2, &room, &me), 0);
}

#[test]
fn timestamps_stay_strictly_increasing_when_clock_goes_back() {
    let (mut a, room, _) = owner_with_room();
    let first = a.send_text(&room, "一", 50_000).unwrap();
    let second = a.send_text(&room, "二", 40_000).unwrap();
    assert_eq!(second.message.ts_ms, first.message.ts_ms + 1);
}

#[test]
fn send_checks_room_state_and_length() {
    let (mut a, room, code) = owner_with_room();
    let mut b = new_core();
    b.join_room(&code, 500).unwrap();
    assert!(matches!(
        b.send_text(&room, "我能说话吗", 1000),
        Err(AppError::RoomNotActive)
    ));
    assert!(matches!(
        a.send_text(&room, &"a".repeat(CHAT_TEXT_MAX_BYTES + 1), 2000),
        Err(AppError::TextTooLong)
    ));
    a.send_text(&room, &"a".repeat(CHAT_TEXT_MAX_BYTES), 3000)
        .unwrap();
}
