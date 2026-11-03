//! V1.1 的聊天功能：引用、@、提醒判定、房间预览、搜索、图片、清空聊天记录。

use super::tests::{DEFAULT_PARAMS, new_core};
use super::*;
use crate::protocol::body::{IMAGE_CHUNK_BYTES, IMAGE_MAX_BYTES};
use crate::protocol::event::kind;
use crate::protocol::frame::short_id;

/// A 建房，B 进房并激活，双方都拿到对方的事件。
fn pair() -> (Core, Core, RoomId) {
    let mut a = new_core();
    a.set_profile("甲", None, 500).unwrap();
    let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
    let room = created.summary.room_id;
    let mut b = new_core();
    b.set_profile("乙", None, 600).unwrap();
    b.join_room(&created.code, 2000).unwrap();
    b.ingest_events(&room, &a.room_events(&room).unwrap(), false, 3000)
        .unwrap();
    let joined = b.activate(&room, 4000).unwrap();
    a.ingest_events(&room, &joined, false, 4000).unwrap();
    (a, b, room)
}

fn chat_of(item: &TimelineItem) -> &ChatMessage {
    match item {
        TimelineItem::Chat(m) => m,
        TimelineItem::Notice(_) => panic!("应当是聊天"),
    }
}

/// 收到这批事件后给界面的新消息通知：(条目, 是否提醒)。
fn added(notes: &[CoreEvent]) -> Vec<(TimelineItem, bool)> {
    notes
        .iter()
        .filter_map(|n| match n {
            CoreEvent::TimelineItemAdded { item, alert, .. } => Some((item.clone(), *alert)),
            _ => None,
        })
        .collect()
}

fn webp(len: usize) -> Vec<u8> {
    let mut b = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
    b.extend((0..len.saturating_sub(16)).map(|i| (i % 251) as u8));
    b
}

#[test]
fn reply_and_mentions_reach_the_other_side() {
    let (mut a, mut b, room) = pair();
    let first = a.send_text(&room, "今晚打几把？", 10_000).unwrap();
    b.ingest_events(&room, &first.events, true, 10_100).unwrap();

    let me_a = a.identity().member_id;
    let draft = Draft {
        text: "@甲 三把",
        reply_to: Some(first.message.event_id),
        mentions: &[me_a, me_a],
    };
    let sent = b.send_chat(&room, &draft, 11_000).unwrap();
    assert_eq!(
        sent.message.mentions,
        vec![short_id(&me_a)],
        "重复的 @ 去掉"
    );
    let reply = sent.message.reply.clone().unwrap();
    assert_eq!(reply.excerpt, "今晚打几把？");
    assert_eq!(reply.author_id, Some(me_a));

    let notes = a.ingest_events(&room, &sent.events, true, 11_100).unwrap();
    let got = added(&notes);
    assert_eq!(got.len(), 1);
    let m = chat_of(&got[0].0);
    assert_eq!(m.reply.as_ref().unwrap().author_id, Some(me_a));
    assert_eq!(m.mentions, vec![short_id(&me_a)]);
    assert!(got[0].1, "实时收到别人的消息要提醒");
}

#[test]
fn reply_must_be_a_message_in_this_room() {
    let (mut a, _b, room) = pair();
    let other = a.create_room("别的群", DEFAULT_PARAMS, 5000).unwrap();
    let elsewhere = a.send_text(&other.summary.room_id, "这边", 6000).unwrap();
    let draft = Draft {
        text: "回复",
        reply_to: Some(elsewhere.message.event_id),
        mentions: &[],
    };
    assert!(matches!(
        a.send_chat(&room, &draft, 7000),
        Err(AppError::ReplyNotFound)
    ));
}

#[test]
fn excerpt_is_cut_on_char_boundary_and_long_bodies_are_rejected() {
    let (mut a, _b, room) = pair();
    let long = "心".repeat(100);
    let first = a.send_text(&room, &long, 10_000).unwrap();
    let draft = Draft {
        text: "嗯",
        reply_to: Some(first.message.event_id),
        mentions: &[],
    };
    let r = a
        .send_chat(&room, &draft, 11_000)
        .unwrap()
        .message
        .reply
        .unwrap();
    assert_eq!(r.excerpt, "心".repeat(20), "60 字节正好 20 个汉字");

    // 文字本身在 900 字节以内，但加上引用和 20 个 @ 之后超出 1024 字节
    let many: Vec<MemberId> = (0..20u8).map(|i| [i; 32]).collect();
    let text = "a".repeat(900);
    let draft = Draft {
        text: &text,
        reply_to: Some(first.message.event_id),
        mentions: &many,
    };
    assert!(matches!(
        a.send_chat(&room, &draft, 12_000),
        Err(AppError::TextTooLong)
    ));
}

#[test]
fn muted_rooms_alert_only_when_mentioned_and_backfill_never_alerts() {
    let (mut a, mut b, room) = pair();
    a.set_room_prefs(&room, Some(true), None).unwrap();
    let plain = b.send_text(&room, "在吗", 10_000).unwrap();
    let got = added(&a.ingest_events(&room, &plain.events, true, 10_100).unwrap());
    assert!(!got[0].1, "免打扰不提醒");

    let me_a = a.identity().member_id;
    let draft = Draft {
        text: "@甲 上线",
        reply_to: None,
        mentions: &[me_a],
    };
    let at = b.send_chat(&room, &draft, 11_000).unwrap();
    let got = added(&a.ingest_events(&room, &at.events, true, 11_100).unwrap());
    assert!(got[0].1, "@ 我时免打扰也提醒");

    let late = b.send_text(&room, "补齐来的", 12_000).unwrap();
    a.set_room_prefs(&room, Some(false), None).unwrap();
    let got = added(&a.ingest_events(&room, &late.events, false, 50_000).unwrap());
    assert!(!got[0].1, "补齐来的不提醒");

    // 自己发的消息不会经入站提醒自己：B 收到自己的消息也不提醒
    let own = b.send_text(&room, "自己", 13_000).unwrap();
    let got = added(&b.ingest_events(&room, &own.events, true, 13_100).unwrap());
    assert!(got.is_empty() || !got[0].1);
}

#[test]
fn room_preview_and_mention_flag() {
    let (mut a, mut b, room) = pair();
    let summary = |c: &Core| {
        c.list_rooms()
            .unwrap()
            .into_iter()
            .find(|r| r.room_id == room)
            .unwrap()
    };
    assert_eq!(summary(&a).preview, None);
    a.send_text(&room, "我先说", 10_000).unwrap();
    assert_eq!(
        summary(&a).preview.as_deref(),
        Some("我先说"),
        "自己发的不带名字"
    );

    let me_a = a.identity().member_id;
    let draft = Draft {
        text: "@甲 来\n了",
        reply_to: None,
        mentions: &[me_a],
    };
    let at = b.send_chat(&room, &draft, 11_000).unwrap();
    a.ingest_events(&room, &at.events, true, 11_100).unwrap();
    let s = summary(&a);
    assert_eq!(s.preview.as_deref(), Some("乙：@甲 来 了"));
    assert_eq!(
        s.last_active, at.message.display_ts,
        "收到消息也更新房间的活跃时间"
    );
    assert!(s.mentioned);
    assert_eq!(s.unread, 1);
    a.mark_read(&room).unwrap();
    assert!(!summary(&a).mentioned, "看过之后 @ 标记消失");
}

#[test]
fn search_is_case_insensitive_newest_first_and_per_room() {
    let (mut a, _b, room) = pair();
    a.send_text(&room, "Rank 走起", 10_000).unwrap();
    a.send_text(&room, "不相关", 11_000).unwrap();
    a.send_text(&room, "再来一把 rank", 12_000).unwrap();
    let other = a.create_room("别的群", DEFAULT_PARAMS, 13_000).unwrap();
    a.send_text(&other.summary.room_id, "RANK", 14_000).unwrap();
    let found: Vec<String> = a
        .search_chat(&room, " RANK ")
        .unwrap()
        .iter()
        .map(|i| chat_of(i).text.clone())
        .collect();
    assert_eq!(found, vec!["再来一把 rank", "Rank 走起"]);
    assert!(a.search_chat(&room, "   ").unwrap().is_empty());
}

#[test]
fn image_is_split_reassembled_and_does_not_stack() {
    let (mut a, mut b, room) = pair();
    let data = webp(IMAGE_CHUNK_BYTES * 2 + 100);
    let sent = a.send_image(&room, &data, (640, 480), 10_000).unwrap();
    assert_eq!(sent.events.len(), 4, "3 片 + 1 条图片消息");
    assert!(
        sent.events[..3]
            .iter()
            .all(|e| e.event.kind == kind::IMAGE_CHUNK)
    );
    assert_eq!(sent.events[3].event.kind, kind::IMAGE);
    assert!(sent.notes.is_empty(), "图片不参与叠层判定");
    let id = sent.message.event_id;
    assert_eq!(a.image_data(&room, &id).unwrap(), Some(data.clone()));

    // 连发三张图也不叠层
    a.send_image(&room, &webp(100), (1, 1), 10_100).unwrap();
    a.send_image(&room, &webp(100), (1, 1), 10_200).unwrap();
    let me = a.identity().member_id;
    let board = a.leaderboard(&room).unwrap();
    assert_eq!(board.iter().find(|(m, _)| *m == me), Some(&(me, 0)));

    // 图片消息先到、各片后到：先显示未收齐，收齐时发 ImageReady
    let head = sent.events[3].clone();
    let got = added(&b.ingest_events(&room, &[head], true, 10_300).unwrap());
    let img = chat_of(&got[0].0).image.unwrap();
    assert_eq!((img.width, img.height, img.ready), (640, 480, false));
    assert_eq!(b.image_data(&room, &id).unwrap(), None);
    let notes = b
        .ingest_events(&room, &sent.events[..2], false, 10_400)
        .unwrap();
    assert!(
        !notes
            .iter()
            .any(|n| matches!(n, CoreEvent::ImageReady { .. }))
    );
    let notes = b
        .ingest_events(&room, &sent.events[2..3], false, 10_500)
        .unwrap();
    assert!(
        notes
            .iter()
            .any(|n| matches!(n, CoreEvent::ImageReady { event_id, .. } if *event_id == id))
    );
    assert_eq!(b.image_data(&room, &id).unwrap(), Some(data));
    let listed = b.list_messages(&room, None, 50).unwrap();
    assert!(
        listed
            .iter()
            .any(|i| matches!(i, TimelineItem::Chat(m) if m.image.is_some_and(|v| v.ready)))
    );
    // 图片的各片不算消息：未读只算图片那一条
    assert_eq!(b.unread(&room).unwrap(), 1);
    assert_eq!(
        b.list_rooms().unwrap()[0].preview.as_deref(),
        Some("甲：[图片]")
    );
}

#[test]
fn bad_images_are_rejected() {
    let (mut a, _b, room) = pair();
    assert!(matches!(
        a.send_image(&room, b"GIF89a....", (1, 1), 10_000),
        Err(AppError::InvalidImage)
    ));
    assert!(matches!(
        a.send_image(&room, &webp(IMAGE_MAX_BYTES + 1), (1, 1), 10_000),
        Err(AppError::ImageTooLarge)
    ));
}

#[test]
fn floors_count_messages_and_include_image_chunks() {
    let (mut a, _b, room) = pair();
    a.send_text(&room, "一", 10_000).unwrap();
    let img = a
        .send_image(&room, &webp(IMAGE_CHUNK_BYTES * 3), (1, 1), 20_000)
        .unwrap();
    a.send_text(&room, "二", 30_000).unwrap();
    let me = short_id(&a.identity().member_id);
    let floor = |n| {
        a.floors_for_last(&room, n)
            .unwrap()
            .into_iter()
            .find(|f| f.author == me)
            .map(|f| f.floor)
    };
    // 最近 2 条消息是图片和「二」：分界挪到图片的第一片
    assert_eq!(floor(2), Some(img.events[0].event.seq));
    assert_eq!(floor(1), Some(img.events.last().unwrap().event.seq + 1));
    assert_eq!(floor(3), Some(1));
}

#[test]
fn clearing_history_hides_old_messages_only_on_this_machine() {
    let (mut a, mut b, room) = pair();
    let before = b.send_text(&room, "清空前", 1_000_000).unwrap();
    a.ingest_events(&room, &before.events, true, 1_000_100)
        .unwrap();
    // B 的这条 A 暂时没收到
    let missed = b.send_text(&room, "没收到的旧消息", 1_005_000).unwrap();
    for t in [1_010_000, 1_011_000, 1_012_000] {
        a.send_text(&room, "冲", t).unwrap();
    }
    let me = a.identity().member_id;
    let stacks = |c: &Core| {
        c.leaderboard(&room)
            .unwrap()
            .into_iter()
            .find(|(m, _)| *m == me)
    };
    let stacked = stacks(&a);
    assert_eq!(stacked, Some((me, 1)));

    let s = a.clear_history(&room, 1_200_000).unwrap();
    assert_eq!((s.unread, s.preview.clone()), (0, None));
    assert!(a.list_messages(&room, None, 50).unwrap().is_empty());
    assert!(a.search_chat(&room, "清空前").unwrap().is_empty());
    assert_eq!(stacks(&a), stacked, "层数不受影响");

    // 之后的新消息照常显示：B 的电脑慢一分钟，按 B 的时间是清空之前，也要显示
    let after = b.send_text(&room, "清空后", 1_150_000).unwrap();
    let got = added(
        &a.ingest_events(&room, &after.events, true, 1_210_000)
            .unwrap(),
    );
    assert_eq!(got.len(), 1);
    // 之后才补齐来的旧消息不显示
    assert!(
        added(
            &a.ingest_events(&room, &missed.events, false, 1_220_000)
                .unwrap()
        )
        .is_empty()
    );
    let shown: Vec<String> = a
        .list_messages(&room, None, 50)
        .unwrap()
        .iter()
        .map(|i| chat_of(i).text.clone())
        .collect();
    assert_eq!(shown, vec!["清空后"]);
    // 清空只是本机的事：B 那边照常
    assert_eq!(b.search_chat(&room, "清空前").unwrap().len(), 1);
}
