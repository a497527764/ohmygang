//! 入站处理测试：用一个不在本机的身份手工构造「别人」的事件，交给本机 Core。

use super::tests::new_core;
use super::*;
use crate::chat::TimelineItem;
use crate::protocol::body::{
    ChatBody, ConfigNoticeBody, HeartsteelBody, ProfileBody, RoomCreateBody, Trigger,
};
use crate::protocol::event::build_event;
use crate::protocol::room_code::{RoomCode, owner_tag_of};

pub(super) struct Remote {
    pub id: Identity,
    pub room: RoomId,
    pub code: String,
}

/// 一个远端房主：房间码由它的公钥摘要组成。
pub(super) fn remote_owner(n: u8) -> Remote {
    let id = Identity::from_secret(&[n; 32]);
    let code = RoomCode {
        room_secret: [n; 16],
        owner_tag: owner_tag_of(&id.member_id()),
    };
    Remote {
        room: code.keys().1,
        code: code.format(),
        id,
    }
}

pub(super) fn state(
    who: &Identity,
    room: RoomId,
    seq: u64,
    ts: u64,
    k: u16,
    b: Vec<u8>,
) -> SignedEvent {
    build_event(who, room, Stream::State, seq, ts, k, b)
}

pub(super) fn chat(who: &Identity, room: RoomId, seq: u64, ts: u64) -> SignedEvent {
    let text = format!("第 {seq} 条");
    build_event(
        who,
        room,
        Stream::Chat,
        seq,
        ts,
        kind::CHAT,
        body::encode(&ChatBody { text }),
    )
}

pub(super) fn heartsteel(
    who: &Identity,
    room: RoomId,
    seq: u64,
    ts: u64,
    triggers: &[&SignedEvent],
) -> SignedEvent {
    let triggers = triggers
        .iter()
        .map(|e| Trigger {
            event_id: e.event_id,
            ts_ms: e.event.ts_ms,
        })
        .collect();
    state(
        who,
        room,
        seq,
        ts,
        kind::HEARTSTEEL,
        body::encode(&HeartsteelBody {
            rule_epoch: 0,
            triggers,
        }),
    )
}

/// 房主的建房、进房、昵称三条状态事件，时间从 1000 毫秒开始。
pub(super) fn founding(o: &Remote) -> Vec<SignedEvent> {
    let create = body::encode(&RoomCreateBody {
        title: "远端房".into(),
        rule: Rule::DEFAULT,
    });
    vec![
        state(&o.id, o.room, 1, 1000, kind::ROOM_CREATE, create),
        state(&o.id, o.room, 2, 1001, kind::JOIN, vec![]),
        state(
            &o.id,
            o.room,
            3,
            1002,
            kind::PROFILE,
            body::encode(&ProfileBody {
                nickname: "房主".into(),
            }),
        ),
    ]
}

/// 房主在 10、11、12 秒连发 3 条，并叠一层（第 4 条状态事件）。
pub(super) fn one_stack(o: &Remote) -> (Vec<SignedEvent>, SignedEvent) {
    let chats = vec![
        chat(&o.id, o.room, 1, 10_000),
        chat(&o.id, o.room, 2, 11_000),
        chat(&o.id, o.room, 3, 12_000),
    ];
    let hs = heartsteel(&o.id, o.room, 4, 12_001, &chats.iter().collect::<Vec<_>>());
    (chats, hs)
}

pub(super) fn stacks_of(core: &Core, room: &RoomId, who: &MemberId) -> u32 {
    core.list_members(room)
        .unwrap()
        .iter()
        .find(|m| &m.member_id == who)
        .map_or(0, |m| m.stacks)
}

fn has_live(notes: &[CoreEvent]) -> bool {
    notes
        .iter()
        .any(|n| matches!(n, CoreEvent::HeartsteelLive { .. }))
}

#[test]
fn state_events_in_any_order_give_same_room_state() {
    let o = remote_owner(10);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    let mut evs = founding(&o);
    evs.reverse();
    let notes = b.ingest_events(&o.room, &evs, true, 2000).unwrap();
    assert!(
        notes
            .iter()
            .any(|n| matches!(n, CoreEvent::RoomUpdated { .. }))
    );
    let members = b.list_members(&o.room).unwrap();
    assert_eq!(members.len(), 1);
    assert!(members[0].is_owner && members[0].in_room);
    assert_eq!(members[0].nickname.as_deref(), Some("房主"));
    assert_eq!(b.room_rule(&o.room).unwrap(), Some(Rule::DEFAULT));
    assert_eq!(b.list_rooms().unwrap()[0].title.as_deref(), Some("远端房"));
}

#[test]
fn stacks_match_regardless_of_arrival_order() {
    let o = remote_owner(11);
    let owner = o.id.member_id();
    let (chats, hs) = one_stack(&o);

    // 一次收齐，顺序打乱
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    let mut all = founding(&o);
    all.push(hs.clone());
    all.extend(chats.iter().cloned());
    all.reverse();
    let notes = b.ingest_events(&o.room, &all, false, 20_000).unwrap();
    assert!(notes.iter().any(
        |n| matches!(n, CoreEvent::StacksChanged { changes, .. } if changes == &vec![(owner, 1)])
    ));
    assert_eq!(stacks_of(&b, &o.room, &owner), 1);

    // 分批到达：叠层事件先到，规则（建房事件）后到
    let mut c = new_core();
    c.join_room(&o.code, 500).unwrap();
    c.ingest_events(&o.room, std::slice::from_ref(&hs), false, 20_000)
        .unwrap();
    assert_eq!(stacks_of(&c, &o.room, &owner), 0, "规则还没到，暂不计入");
    c.ingest_events(&o.room, &founding(&o), false, 20_000)
        .unwrap();
    assert_eq!(stacks_of(&c, &o.room, &owner), 1, "规则到达后重算计入");
}

#[test]
fn non_owner_cannot_change_rule_or_dissolve() {
    let o = remote_owner(12);
    let mallory = Identity::from_secret(&[99; 32]);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    b.ingest_events(&o.room, &founding(&o), true, 2000).unwrap();
    let cheat = Rule {
        rule_epoch: 5,
        threshold: 2,
        ..Rule::DEFAULT
    };
    let evs = vec![
        state(&mallory, o.room, 1, 3000, kind::JOIN, vec![]),
        state(
            &mallory,
            o.room,
            2,
            3001,
            kind::CONFIG_NOTICE,
            body::encode(&ConfigNoticeBody { rule: cheat }),
        ),
        state(&mallory, o.room, 3, 3002, kind::DISSOLVE, vec![]),
    ];
    b.ingest_events(&o.room, &evs, true, 4000).unwrap();
    assert_eq!(b.room_rule(&o.room).unwrap(), Some(Rule::DEFAULT));
    assert_ne!(b.list_rooms().unwrap()[0].status, RoomStatus::Dissolved);
    assert_eq!(b.list_members(&o.room).unwrap().len(), 2);
}

#[test]
fn owner_dissolve_closes_room() {
    let o = remote_owner(13);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    let mut evs = founding(&o);
    evs.push(state(&o.id, o.room, 4, 50_000, kind::DISSOLVE, vec![]));
    b.ingest_events(&o.room, &evs, true, 50_100).unwrap();
    assert_eq!(b.list_rooms().unwrap()[0].status, RoomStatus::Dissolved);
}

#[test]
fn heartsteel_sound_only_for_live_and_recent() {
    let o = remote_owner(14);
    let (chats, hs) = one_stack(&o);
    let mut batch = founding(&o);
    batch.extend(chats);
    batch.push(hs);

    // 补齐收到：只更新层数，不响
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    let notes = b.ingest_events(&o.room, &batch, false, 12_500).unwrap();
    assert!(
        notes
            .iter()
            .any(|n| matches!(n, CoreEvent::StacksChanged { .. }))
    );
    assert!(!has_live(&notes));

    // 实时收到，且在触发后 60 秒内：响
    let mut c = new_core();
    c.join_room(&o.code, 500).unwrap();
    assert!(has_live(
        &c.ingest_events(&o.room, &batch, true, 12_500).unwrap()
    ));

    // 实时收到，但触发已是 60 秒以前：不响
    let mut d = new_core();
    d.join_room(&o.code, 500).unwrap();
    assert!(!has_live(
        &d.ingest_events(&o.room, &batch, true, 12_000 + 60_001)
            .unwrap()
    ));
}

#[test]
fn display_ts_handles_clock_skew() {
    let o = remote_owner(15);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    b.ingest_events(&o.room, &founding(&o), false, 2000)
        .unwrap();
    let shown = |notes: Vec<CoreEvent>| {
        notes
            .into_iter()
            .find_map(|n| match n {
                CoreEvent::TimelineItemAdded {
                    item: TimelineItem::Chat(m),
                    ..
                } => Some(m.display_ts),
                _ => None,
            })
            .expect("应有一条聊天")
    };
    let now = 1_000_000;
    // 实时收到、对方时钟快 10 分钟：按本机时间显示
    let fast = chat(&o.id, o.room, 1, now + 600_000);
    assert_eq!(
        shown(b.ingest_events(&o.room, &[fast], true, now).unwrap()),
        now
    );
    // 补齐收到、时间在未来：按本机时间显示
    let future = chat(&o.id, o.room, 2, now + 600_001);
    assert_eq!(
        shown(b.ingest_events(&o.room, &[future], false, now).unwrap()),
        now
    );
    // 补齐收到的旧消息：按作者时间显示
    let old = chat(&o.id, o.room, 3, 5000);
    assert_eq!(
        shown(b.ingest_events(&o.room, &[old], false, now).unwrap()),
        5000
    );
}

#[test]
fn tampered_and_duplicate_events_are_ignored_forks_reported() {
    let o = remote_owner(16);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    let mut bad = chat(&o.id, o.room, 1, 5000);
    bad.event.body.push(b'!');
    assert!(
        b.ingest_events(&o.room, &[bad], true, 5000)
            .unwrap()
            .is_empty()
    );
    assert!(b.room_events(&o.room).unwrap().is_empty());

    let good = chat(&o.id, o.room, 1, 5000);
    b.ingest_events(&o.room, std::slice::from_ref(&good), true, 5000)
        .unwrap();
    assert!(
        b.ingest_events(&o.room, &[good], true, 5000)
            .unwrap()
            .is_empty(),
        "重复事件不产生通知"
    );

    let fork = chat(&o.id, o.room, 1, 5001);
    let notes = b.ingest_events(&o.room, &[fork], true, 5001).unwrap();
    let owner = o.id.member_id();
    assert!(
        notes
            .iter()
            .any(|n| matches!(n, CoreEvent::ForkDetected { member_id, .. } if *member_id == owner))
    );
}

#[test]
fn unknown_room_is_an_error() {
    let o = remote_owner(17);
    let mut b = new_core();
    assert!(matches!(
        b.ingest_events(&o.room, &founding(&o), true, 1),
        Err(AppError::RoomNotFound)
    ));
}
