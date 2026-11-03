//! Core 为同步提供的接口：持有向量、按区间取事件、Joining 转 Active、房间密钥、解散事件。

use super::ingest_tests::{founding, remote_owner, state};
use super::tests::{DEFAULT_PARAMS, new_core};
use super::*;
use crate::protocol::frame::{ShortId, short_id};
use crate::sync::plan::{ready_to_activate, state_gaps};

#[test]
fn floors_mark_where_the_last_n_chats_begin() {
    let mut a = new_core();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    let mut b = new_core();
    b.join_room(&created.code, 2000).unwrap();
    b.ingest_events(&room, &a.room_events(&room).unwrap(), false, 3000)
        .unwrap();
    let joined = b.activate(&room, 4000).unwrap();
    a.ingest_events(&room, &joined, false, 4000).unwrap();
    // A 的聊天在 10、20、30、40、50 秒，B 的在 25、35、60 秒（各自从 1 编号）
    for t in [10_000, 20_000, 30_000, 40_000, 50_000] {
        a.send_text(&room, "甲", t).unwrap();
    }
    for t in [25_000, 35_000, 60_000] {
        let s = b.send_text(&room, "乙", t).unwrap();
        a.ingest_events(&room, &s.events, false, 100_000).unwrap();
    }
    let (sa, sb) = (
        short_id(&a.identity().member_id),
        short_id(&b.identity().member_id),
    );
    let floors = |n| {
        let mut v: Vec<(ShortId, u64)> = a
            .floors_for_last(&room, n)
            .unwrap()
            .into_iter()
            .map(|f| (f.author, f.floor))
            .collect();
        v.sort();
        v
    };
    // 最近 4 条：B3(60) A5(50) A4(40) B2(35)
    let mut want = vec![(sa, 4), (sb, 2)];
    want.sort();
    assert_eq!(floors(4), want);
    assert!(floors(0).is_empty());
    let mut all = vec![(sa, 1), (sb, 1)];
    all.sort();
    assert_eq!(floors(100), all);

    // 分界落盘后能读回，后写的覆盖先写的
    a.set_chat_floors(&room, &[(sa, 3), (sb, 7)]).unwrap();
    a.set_chat_floors(&room, &[(sb, 2)]).unwrap();
    let got = a.chat_floors(&room).unwrap();
    assert_eq!((got.get(&sa), got.get(&sb)), (Some(&3), Some(&2)));
}

#[test]
fn joining_member_catches_up_then_activates() {
    let mut a = new_core();
    a.set_profile("房主", None, 1).unwrap();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    let mut b = new_core();
    b.set_profile("新人", None, 2).unwrap();
    b.join_room(&created.code, 2000).unwrap();
    assert!(!b.owner_known(&room).unwrap());

    // B 只看 A 的 Have 算缺口，按区间向 A 要，A 取出事件交给 B
    let a_vector = a.have_vector(&room).unwrap();
    let gaps = state_gaps(&b.holdings(&room).unwrap(), &a_vector);
    assert!(!gaps.is_empty());
    let served = a.events_for(&room, &gaps).unwrap();
    assert_eq!(served.len(), 3, "建房、进房、昵称");
    b.ingest_events(&room, &served, false, 3000).unwrap();

    assert!(b.owner_known(&room).unwrap());
    assert!(ready_to_activate(
        true,
        &b.holdings(&room).unwrap(),
        &[a_vector],
        short_id(&b.identity().member_id)
    ));
    let written = b.activate(&room, 4000).unwrap();
    assert_eq!(
        written.iter().map(|e| e.event.kind).collect::<Vec<_>>(),
        vec![kind::JOIN, kind::PROFILE]
    );
    assert_eq!(b.list_rooms().unwrap()[0].status, RoomStatus::Active);
    assert_eq!(b.unread(&room).unwrap(), 0, "首次进房同步的历史不计入未读");
    assert!(
        b.activate(&room, 5000).unwrap().is_empty(),
        "已经是 Active 时不再重复写"
    );

    a.ingest_events(&room, &written, true, 4100).unwrap();
    let in_room: Vec<_> = a
        .list_members(&room)
        .unwrap()
        .into_iter()
        .filter(|m| m.in_room)
        .collect();
    assert_eq!(in_room.len(), 2);
}

#[test]
fn have_vector_reports_contiguous_holdings() {
    let mut a = new_core();
    let room = a
        .create_room("开黑", DEFAULT_PARAMS, 1000)
        .unwrap()
        .summary
        .room_id;
    for t in [2000, 3000] {
        a.send_text(&room, "冲", t).unwrap();
    }
    let me = short_id(&a.identity().member_id);
    let v = a.have_vector(&room).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].author, v[0].state_max, v[0].chat_max), (me, 2, 2));
}

#[test]
fn room_key_matches_code_and_sync_rooms_skip_left() {
    let mut a = new_core();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    let code = crate::protocol::room_code::RoomCode::parse(&created.code).unwrap();
    assert_eq!(a.room_key(&room).unwrap(), code.keys().0);
    assert_eq!(a.sync_rooms().unwrap(), vec![(room, RoomStatus::Active)]);
    room_store::set_status(&a.conn, &room, RoomStatus::Left).unwrap();
    assert!(a.sync_rooms().unwrap().is_empty());
}

#[test]
fn dissolve_event_is_the_owners_earliest() {
    let o = remote_owner(40);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    assert!(b.dissolve_event(&o.room).unwrap().is_none());
    let mut evs = founding(&o);
    evs.push(state(&o.id, o.room, 4, 50_000, kind::DISSOLVE, vec![]));
    evs.push(state(&o.id, o.room, 5, 60_000, kind::DISSOLVE, vec![]));
    b.ingest_events(&o.room, &evs, false, 70_000).unwrap();
    assert_eq!(
        b.dissolve_event(&o.room).unwrap().map(|e| e.event.seq),
        Some(4)
    );
}

#[test]
fn only_the_owner_can_dissolve() {
    let mut a = new_core();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    let mut b = new_core();
    b.join_room(&created.code, 2000).unwrap();
    room_store::set_status(&b.conn, &room, RoomStatus::Active).unwrap();
    assert!(matches!(
        b.dissolve_room(&room, 3000),
        Err(AppError::NotOwner)
    ));
    let written = a.dissolve_room(&room, 4000).unwrap();
    assert_eq!(written[0].event.kind, kind::DISSOLVE);
    assert_eq!(a.room_status(&room).unwrap(), RoomStatus::Dissolved);
    assert!(matches!(
        a.send_text(&room, "还能说话吗", 5000),
        Err(AppError::RoomNotActive)
    ));
}
