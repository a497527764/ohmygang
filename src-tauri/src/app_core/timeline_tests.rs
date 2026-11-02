//! 消息列表与未读测试。

use super::ingest_tests::{chat, founding, remote_owner, state};
use super::tests::{DEFAULT_PARAMS, new_core};
use super::*;
use crate::chat::{NoticeKind, TimelineItem};
use crate::protocol::body::RenameBody;

fn describe(items: &[TimelineItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| match i {
            TimelineItem::Chat(m) => format!("聊天 {}", m.ts_ms),
            TimelineItem::Notice(n) => format!("{:?} {}", n.kind, n.ts_ms),
        })
        .collect()
}

#[test]
fn late_old_message_still_counts_as_unread() {
    let o = remote_owner(30);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    b.ingest_events(&o.room, &founding(&o), false, 2000)
        .unwrap();
    b.mark_read(&o.room).unwrap();
    assert_eq!(b.unread(&o.room).unwrap(), 0);
    // 作者时间很早（5 秒），但刚刚才收到
    b.ingest_events(&o.room, &[chat(&o.id, o.room, 1, 5000)], false, 900_000)
        .unwrap();
    assert_eq!(b.unread(&o.room).unwrap(), 1);
    assert_eq!(b.list_rooms().unwrap()[0].unread, 1);
}

#[test]
fn own_messages_are_not_unread_and_mark_read_clears() {
    let mut a = new_core();
    let room = a
        .create_room("开黑", DEFAULT_PARAMS, 1000)
        .unwrap()
        .summary
        .room_id;
    a.send_text(&room, "我自己说的", 2000).unwrap();
    assert_eq!(a.unread(&room).unwrap(), 0);

    let friend = crate::crypto::Identity::from_secret(&[31; 32]);
    let from_friend = vec![
        state(&friend, room, 1, 3000, kind::JOIN, vec![]),
        chat(&friend, room, 1, 3001),
        chat(&friend, room, 2, 3002),
    ];
    a.ingest_events(&room, &from_friend, true, 3003).unwrap();
    assert_eq!(a.unread(&room).unwrap(), 2);
    a.mark_read(&room).unwrap();
    assert_eq!(a.unread(&room).unwrap(), 0);
}

#[test]
fn pages_mix_chat_and_notices_in_order() {
    let o = remote_owner(32);
    let mallory = crate::crypto::Identity::from_secret(&[33; 32]);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    let mut evs = founding(&o);
    evs.extend([
        chat(&o.id, o.room, 1, 2000),
        chat(&o.id, o.room, 2, 3000),
        chat(&o.id, o.room, 3, 4000),
    ]);
    evs.push(state(
        &o.id,
        o.room,
        4,
        3500,
        kind::RENAME,
        body::encode(&RenameBody {
            title: "新群名".into(),
        }),
    ));
    // 非房主改名：入库但不显示
    evs.push(state(
        &mallory,
        o.room,
        1,
        3600,
        kind::RENAME,
        body::encode(&RenameBody {
            title: "乱改".into(),
        }),
    ));
    b.ingest_events(&o.room, &evs, false, 10_000).unwrap();

    let page1 = b.list_messages(&o.room, None, 3).unwrap();
    assert_eq!(
        describe(&page1),
        vec!["聊天 3000", "Rename 3500", "聊天 4000"]
    );
    assert!(
        matches!(&page1[1], TimelineItem::Notice(n) if n.kind == NoticeKind::Rename && n.title.as_deref() == Some("新群名"))
    );
    let page2 = b
        .list_messages(&o.room, Some(page1[0].cursor()), 3)
        .unwrap();
    assert_eq!(describe(&page2), vec!["Join 1001", "聊天 2000"]);
    assert!(
        b.list_messages(&o.room, Some(page2[0].cursor()), 3)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn messages_after_dissolve_are_hidden_and_not_unread() {
    let o = remote_owner(34);
    let mut b = new_core();
    b.join_room(&o.code, 500).unwrap();
    let mut evs = founding(&o);
    evs.push(state(&o.id, o.room, 4, 50_000, kind::DISSOLVE, vec![]));
    evs.push(chat(&o.id, o.room, 1, 40_000));
    evs.push(chat(&o.id, o.room, 2, 60_000));
    b.ingest_events(&o.room, &evs, false, 70_000).unwrap();
    let items = b.list_messages(&o.room, None, 10).unwrap();
    assert_eq!(
        describe(&items),
        vec!["Join 1001", "聊天 40000", "Dissolve 50000"]
    );
    assert_eq!(b.unread(&o.room).unwrap(), 1);
}
