//! 备份与恢复（需求 2.3 验收 8、架构 8.3）。

use super::tests::{DEFAULT_PARAMS, new_core};
use super::*;
use crate::backup::BackupKind;
use crate::protocol::frame::short_id;
use crate::sync::plan::ready_to_activate;

const PASS: &str = "心之钢123";

fn heartsteel_count(c: &Core, room: &RoomId) -> usize {
    c.room_events(room)
        .unwrap()
        .iter()
        .filter(|e| e.event.kind == kind::HEARTSTEEL)
        .count()
}

fn my_stacks(c: &Core, room: &RoomId) -> u32 {
    let me = c.identity().member_id;
    c.list_members(room)
        .unwrap()
        .iter()
        .find(|m| m.member_id == me)
        .map_or(0, |m| m.stacks)
}

/// 房主 A 建房并叠一层（3 条聊天 + 1 个叠层事件）。
fn stacked_owner() -> (Core, RoomId, String) {
    let mut a = new_core();
    a.set_profile("阿钢", Some(Avatar::Preset { icon: 1, color: 1 }), 500)
        .unwrap();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    for t in [10_000, 11_000, 12_000] {
        a.send_text(&room, "冲", t).unwrap();
    }
    assert_eq!(heartsteel_count(&a, &room), 1);
    (a, room, created.code)
}

#[test]
fn identity_only_restore_does_not_restack_old_messages() {
    let (a, room, code) = stacked_owner();
    let file = a.export_backup(BackupKind::IdentityOnly, PASS).unwrap();

    // 换一台新电脑：恢复身份，再用房间码进房，从同伴那里同步回本人的旧消息
    let mut a2 = new_core();
    let view = a2.import_backup(&file, PASS, false, 50_000).unwrap();
    assert_eq!(view.member_id, a.identity().member_id);
    assert_eq!(view.nickname, "阿钢");
    assert_eq!(view.avatar, Some(Avatar::Preset { icon: 1, color: 1 }));
    assert!(a2.list_rooms().unwrap().is_empty(), "只备份身份时不带房间");
    a2.join_room(&code, 50_000).unwrap();

    // 只同步回一部分：同伴有我第 3 条聊天，本机还没有，不能激活
    let partial: Vec<SignedEvent> = a
        .room_events(&room)
        .unwrap()
        .into_iter()
        .filter(|e| !(e.event.stream == Stream::Chat && e.event.seq == 3))
        .collect();
    a2.ingest_events(&room, &partial, false, 60_000).unwrap();
    let me = short_id(&a.identity().member_id);
    let peer = vec![a.have_vector(&room).unwrap()];
    assert!(!ready_to_activate(
        true,
        &a2.holdings(&room).unwrap(),
        &peer,
        me
    ));

    a2.ingest_events(&room, &a.room_events(&room).unwrap(), false, 60_000)
        .unwrap();
    assert!(ready_to_activate(
        true,
        &a2.holdings(&room).unwrap(),
        &peer,
        me
    ));
    assert_eq!(
        heartsteel_count(&a2, &room),
        1,
        "同步回来的旧消息不产生新叠层"
    );
    assert_eq!(my_stacks(&a2, &room), 1, "层数由重放恢复");

    a2.activate(&room, 70_000).unwrap();
    let next = a2.send_text(&room, "我回来了", 80_000).unwrap();
    assert_eq!(next.events[0].event.seq, 4, "新消息的序号接在旧消息后面");
}

#[test]
fn full_restore_brings_rooms_and_history_back_in_joining() {
    let (a, room, _) = stacked_owner();
    let file = a.export_backup(BackupKind::Full, PASS).unwrap();
    let mut a2 = new_core();
    a2.import_backup(&file, PASS, false, 50_000).unwrap();
    let rooms = a2.list_rooms().unwrap();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].status, RoomStatus::Joining, "和同伴同步后才能发言");
    assert_eq!(rooms[0].title.as_deref(), Some("开黑"));
    let chats = a2
        .list_messages(&room, None, 50)
        .unwrap()
        .iter()
        .filter(|i| matches!(i, TimelineItem::Chat(_)))
        .count();
    assert_eq!(chats, 3);
    assert_eq!(my_stacks(&a2, &room), 1);
    assert_eq!(heartsteel_count(&a2, &room), 1);
    assert_eq!(a2.room_code(&room).unwrap(), a.room_code(&room).unwrap());
}

#[test]
fn importing_over_existing_rooms_needs_confirmation() {
    let (a, _, _) = stacked_owner();
    let file = a.export_backup(BackupKind::IdentityOnly, PASS).unwrap();
    let (mut b, b_room, _) = stacked_owner();
    let b_id = b.identity().member_id;
    assert!(matches!(
        b.import_backup(&file, PASS, false, 50_000),
        Err(AppError::IdentityExists)
    ));
    assert_eq!(b.identity().member_id, b_id, "没确认覆盖时什么都不改");
    assert_eq!(b.list_rooms().unwrap().len(), 1);

    b.import_backup(&file, PASS, true, 50_000).unwrap();
    assert_eq!(b.identity().member_id, a.identity().member_id);
    assert!(
        b.list_rooms().unwrap().is_empty(),
        "覆盖时清空本机原来的房间"
    );
    assert!(b.room_events(&b_room).unwrap().is_empty());
}

#[test]
fn wrong_passphrase_is_rejected() {
    let (a, _, _) = stacked_owner();
    let file = a.export_backup(BackupKind::IdentityOnly, PASS).unwrap();
    let mut a2 = new_core();
    let before = a2.identity().member_id;
    assert!(matches!(
        a2.import_backup(&file, "不是这个口令", true, 50_000),
        Err(AppError::BackupInvalid)
    ));
    assert_eq!(a2.identity().member_id, before);
}
