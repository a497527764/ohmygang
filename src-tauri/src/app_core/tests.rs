//! Core 的单机测试：建房、进房、昵称、成员查询。后续任务在这里继续加入站、发言、未读的测试。

use super::*;
use crate::keystore::MemoryKeyStore;
use crate::protocol::body::Rule;
use crate::protocol::event::kind;
use crate::protocol::room_code::{RoomCode, owner_tag_of};
use crate::storage;

pub(super) fn new_core() -> Core {
    Core::open(
        storage::open_in_memory().unwrap(),
        &MemoryKeyStore([1u8; 32]),
    )
    .unwrap()
}

pub(super) const DEFAULT_PARAMS: RuleParams = RuleParams {
    window_ms: 5000,
    threshold: 3,
    cooldown_ms: 30_000,
};

#[test]
fn room_prefs_change_only_given_fields_and_pinned_rooms_come_first() {
    let mut a = new_core();
    let r1 = a
        .create_room("一", DEFAULT_PARAMS, 1000)
        .unwrap()
        .summary
        .room_id;
    let r2 = a
        .create_room("二", DEFAULT_PARAMS, 2000)
        .unwrap()
        .summary
        .room_id;
    let order = |c: &Core| {
        c.list_rooms()
            .unwrap()
            .iter()
            .map(|r| r.room_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&a), vec![r2, r1], "默认按最近活跃");

    let s = a.set_room_prefs(&r1, None, Some(true)).unwrap();
    assert!(s.pinned && !s.muted);
    assert_eq!(order(&a), vec![r1, r2], "置顶的在前");

    let s = a.set_room_prefs(&r1, Some(true), None).unwrap();
    assert!(s.pinned && s.muted, "只改给定项");
    assert!(matches!(
        a.set_room_prefs(&[9u8; 16], Some(true), None),
        Err(AppError::RoomNotFound)
    ));
}

#[test]
fn create_room_makes_me_owner_with_default_rule() {
    let mut a = new_core();
    a.set_profile("阿钢", None, 500).unwrap();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;

    assert_eq!(created.summary.title.as_deref(), Some("开黑"));
    assert_eq!(created.summary.status, RoomStatus::Active);
    assert!(created.summary.is_owner);
    assert_eq!(
        created
            .events
            .iter()
            .map(|e| e.event.kind)
            .collect::<Vec<_>>(),
        vec![kind::ROOM_CREATE, kind::JOIN, kind::PROFILE]
    );
    // 时间严格递增
    assert!(
        created
            .events
            .windows(2)
            .all(|w| w[0].event.ts_ms < w[1].event.ts_ms)
    );

    let code = RoomCode::parse(&created.code).unwrap();
    assert_eq!(code.owner_tag, owner_tag_of(&a.identity().member_id));
    assert_eq!(code.keys().1, room);
    assert_eq!(a.room_code(&room).unwrap(), created.code);

    let members = a.list_members(&room).unwrap();
    assert_eq!(members.len(), 1);
    assert!(members[0].is_owner && members[0].in_room);
    assert_eq!(members[0].nickname.as_deref(), Some("阿钢"));
    assert_eq!(a.room_rule(&room).unwrap(), Some(Rule::DEFAULT));
}

#[test]
fn create_room_validates_title_and_rule() {
    let mut a = new_core();
    assert!(matches!(
        a.create_room("  ", DEFAULT_PARAMS, 1),
        Err(AppError::InvalidTitle)
    ));
    assert!(matches!(
        a.create_room(&"名".repeat(65), DEFAULT_PARAMS, 1),
        Err(AppError::InvalidTitle)
    ));
    let bad = RuleParams {
        threshold: 1,
        ..DEFAULT_PARAMS
    };
    assert!(matches!(
        a.create_room("开黑", bad, 1),
        Err(AppError::InvalidRule)
    ));
    assert!(a.list_rooms().unwrap().is_empty());
}

#[test]
fn join_room_parses_code_and_starts_joining() {
    let mut a = new_core();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let mut b = new_core();
    let joined = b.join_room(&created.code, 2000).unwrap();
    assert_eq!(joined.room_id, created.summary.room_id);
    assert_eq!(joined.status, RoomStatus::Joining);
    // 重复进房不重复建记录
    b.join_room(&created.code.to_lowercase(), 3000).unwrap();
    assert_eq!(b.list_rooms().unwrap().len(), 1);

    let mut typo: Vec<char> = created.code.chars().collect();
    typo[10] = if typo[10] == 'A' { 'B' } else { 'A' };
    let typo: String = typo.into_iter().collect();
    assert!(matches!(
        b.join_room(&typo, 4000),
        Err(AppError::InvalidRoomCode(_))
    ));
}

#[test]
fn nickname_change_emits_profile_in_active_rooms() {
    let mut a = new_core();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    let events = a.set_profile("新名字", None, 5000).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.kind, kind::PROFILE);
    assert!(events[0].event.ts_ms > created.events.last().unwrap().event.ts_ms);
    assert_eq!(a.identity().nickname, "新名字");
    assert_eq!(
        a.list_rooms().unwrap()[0].last_active,
        1000,
        "改资料不改变房间列表的时间"
    );
    assert_eq!(
        a.list_members(&room).unwrap()[0].nickname.as_deref(),
        Some("新名字")
    );
}

#[test]
fn avatar_reaches_other_members_and_new_rooms() {
    let mut a = new_core();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    let preset = Avatar::Preset { icon: 7, color: 2 };
    let events = a.set_profile("阿钢", Some(preset.clone()), 5000).unwrap();
    assert_eq!(a.identity().avatar, Some(preset.clone()));

    // 另一个人拿到这些事件后，成员列表里能看到头像
    let mut b = new_core();
    b.join_room(&created.code, 6000).unwrap();
    let mut all = created.events.clone();
    all.extend(events);
    b.ingest_events(&room, &all, false, 7000).unwrap();
    let me = a.identity().member_id;
    let seen = b.list_members(&room).unwrap();
    let m = seen.iter().find(|m| m.member_id == me).unwrap();
    assert_eq!(m.avatar, Some(preset.clone()));

    // 之后新建的房间，建房时的 Profile 就带着头像
    let second = a.create_room("第二个群", DEFAULT_PARAMS, 8000).unwrap();
    let mut c = new_core();
    c.join_room(&second.code, 9000).unwrap();
    c.ingest_events(&second.summary.room_id, &second.events, false, 9500)
        .unwrap();
    assert_eq!(
        c.list_members(&second.summary.room_id).unwrap()[0].avatar,
        Some(preset)
    );

    // 换回默认头像
    a.set_profile("阿钢", None, 10_000).unwrap();
    assert_eq!(a.list_members(&room).unwrap()[0].avatar, None);
}

#[test]
fn invalid_avatar_is_rejected_without_writing() {
    let mut a = new_core();
    a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let big = Avatar::Image(vec![0xff; 2000]);
    assert!(matches!(
        a.set_profile("阿钢", Some(big), 5000),
        Err(AppError::InvalidAvatar)
    ));
    assert_eq!(a.identity().nickname, "");
}
