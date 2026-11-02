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
fn create_room_makes_me_owner_with_default_rule() {
    let mut a = new_core();
    a.set_nickname("阿钢", 500).unwrap();
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
    let events = a.set_nickname("新名字", 5000).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.kind, kind::PROFILE);
    assert!(events[0].event.ts_ms > created.events.last().unwrap().event.ts_ms);
    assert_eq!(a.identity().nickname, "新名字");
    assert_eq!(
        a.list_members(&room).unwrap()[0].nickname.as_deref(),
        Some("新名字")
    );
}
