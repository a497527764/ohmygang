//! 群管理：改群名、改规则、退出、加入中取消（需求 3.2、3.4、3.5）。

use super::ingest_tests::stacks_of;
use super::tests::{DEFAULT_PARAMS, new_core};
use super::*;

/// 房主 A 建房，B 进房并激活，双方都有对方的事件。
fn pair() -> (Core, Core, RoomId) {
    let mut a = new_core();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    let mut b = new_core();
    b.join_room(&created.code, 2000).unwrap();
    b.ingest_events(&room, &a.room_events(&room).unwrap(), false, 3000)
        .unwrap();
    let joined = b.activate(&room, 4000).unwrap();
    a.ingest_events(&room, &joined, false, 4000).unwrap();
    (a, b, room)
}

fn in_room(c: &Core, room: &RoomId, who: &MemberId) -> bool {
    c.list_members(room)
        .unwrap()
        .iter()
        .any(|m| m.member_id == *who && m.in_room)
}

#[test]
fn only_the_owner_renames_and_changes_rules() {
    let (mut a, mut b, room) = pair();
    assert!(matches!(
        b.rename_room(&room, "新名字", 5000),
        Err(AppError::NotOwner)
    ));
    assert!(matches!(
        b.set_room_rule(&room, DEFAULT_PARAMS, 5000),
        Err(AppError::NotOwner)
    ));
    assert!(matches!(
        b.dissolve_room(&room, 5000),
        Err(AppError::NotOwner)
    ));
    assert!(matches!(
        a.rename_room(&room, "  ", 5000),
        Err(AppError::InvalidTitle)
    ));
    let bad = RuleParams {
        window_ms: 500,
        ..DEFAULT_PARAMS
    };
    assert!(matches!(
        a.set_room_rule(&room, bad, 5000),
        Err(AppError::InvalidRule)
    ));

    let renamed = a.rename_room(&room, "周五开黑", 5000).unwrap();
    b.ingest_events(&room, &renamed, true, 5000).unwrap();
    let title = |c: &Core| c.list_rooms().unwrap()[0].title.clone();
    assert_eq!(title(&a).as_deref(), Some("周五开黑"));
    assert_eq!(title(&b).as_deref(), Some("周五开黑"), "B 收到改名");

    let rule = RuleParams {
        window_ms: 10_000,
        threshold: 2,
        cooldown_ms: 5000,
    };
    let changed = a.set_room_rule(&room, rule, 6000).unwrap();
    b.ingest_events(&room, &changed, true, 6000).unwrap();
    let r = b.room_rule(&room).unwrap().unwrap();
    assert_eq!(
        (r.rule_epoch, r.window_ms, r.threshold, r.cooldown_ms),
        (1, 10_000, 2, 5000),
        "新规则 epoch 加 1"
    );
}

#[test]
fn new_rule_applies_to_later_messages_only() {
    let (mut a, _b, room) = pair();
    let me = a.identity().member_id;
    for t in [10_000, 11_000, 12_000] {
        a.send_text(&room, "冲", t).unwrap();
    }
    assert_eq!(stacks_of(&a, &room, &me), 1);
    let easier = RuleParams {
        window_ms: 5000,
        threshold: 2,
        cooldown_ms: 1000,
    };
    a.set_room_rule(&room, easier, 60_000).unwrap();
    assert_eq!(stacks_of(&a, &room, &me), 1, "改规则不改以前的层数");
    for t in [70_000, 71_000] {
        a.send_text(&room, "冲", t).unwrap();
    }
    assert_eq!(stacks_of(&a, &room, &me), 2, "之后按新规则：2 条就叠层");
}

#[test]
fn leaving_keeps_stacks_and_rejoining_restores_membership() {
    let (mut a, mut b, room) = pair();
    let bid = b.identity().member_id;
    for t in [10_000, 11_000, 12_000] {
        let s = b.send_text(&room, "冲", t).unwrap();
        a.ingest_events(&room, &s.events, true, t).unwrap();
    }
    let left = b.leave_room(&room, 20_000).unwrap();
    assert_eq!(b.room_status(&room).unwrap(), RoomStatus::Left);
    assert!(matches!(
        b.send_text(&room, "还在吗", 21_000),
        Err(AppError::RoomNotActive)
    ));
    a.ingest_events(&room, &left, true, 20_000).unwrap();
    assert!(!in_room(&a, &room, &bid), "A 看到 B 退出");
    assert!(
        !a.leaderboard(&room).unwrap().iter().any(|(m, _)| *m == bid),
        "排行榜只列当前成员"
    );
    assert_eq!(stacks_of(&a, &room, &bid), 1, "层数保留");

    let code = a.room_code(&room).unwrap();
    b.join_room(&code, 30_000).unwrap();
    assert_eq!(b.room_status(&room).unwrap(), RoomStatus::Joining);
    let back = b.activate(&room, 31_000).unwrap();
    assert!(
        back.iter().any(|e| e.event.kind == kind::JOIN),
        "重新进房写 Join"
    );
    a.ingest_events(&room, &back, true, 31_000).unwrap();
    assert!(in_room(&a, &room, &bid));
    assert!(
        a.leaderboard(&room)
            .unwrap()
            .iter()
            .any(|(m, s)| *m == bid && *s == 1),
        "回来后层数还在"
    );
}

#[test]
fn cancelling_a_fresh_join_deletes_the_room() {
    let (mut a, _b, room) = pair();
    let code = a.room_code(&room).unwrap();
    let mut c = new_core();
    c.join_room(&code, 1000).unwrap();
    c.ingest_events(&room, &a.room_events(&room).unwrap(), false, 2000)
        .unwrap();
    c.cancel_join(&room).unwrap();
    assert!(c.list_rooms().unwrap().is_empty());
    assert!(c.room_events(&room).unwrap().is_empty(), "本机记录删干净");
    assert!(matches!(
        a.cancel_join(&room),
        Err(AppError::RoomNotJoining)
    ));
}

#[test]
fn cancelling_a_rejoin_goes_back_to_left() {
    let (a, mut b, room) = pair();
    let code = a.room_code(&room).unwrap();
    b.send_text(&room, "我来过", 10_000).unwrap();
    b.leave_room(&room, 20_000).unwrap();
    b.join_room(&code, 30_000).unwrap();
    b.cancel_join(&room).unwrap();
    assert_eq!(b.room_status(&room).unwrap(), RoomStatus::Left);
    let kept = b
        .list_messages(&room, None, 50)
        .unwrap()
        .iter()
        .any(|i| matches!(i, TimelineItem::Chat(m) if m.text == "我来过"));
    assert!(kept, "以前的聊天记录保留");
}
