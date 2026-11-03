//! 存储层测试：事件入库去重与分叉、序号与时间、时间线分页、房间相关表、重新打开文件。

use super::events::{self, Delivery, PutResult};
use super::rooms::{self, RoomStatus};
use super::*;
use crate::crypto::Identity;
use crate::heartsteel::Rules;
use crate::protocol::body::Rule;
use crate::protocol::event::{SignedEvent, Stream, build_event, kind, verify_stateless};

const ROOM: [u8; 16] = [1u8; 16];

fn me() -> Identity {
    Identity::from_secret(&[7u8; 32])
}

fn chat(who: &Identity, seq: u64, ts: u64, body: &[u8]) -> SignedEvent {
    build_event(who, ROOM, Stream::Chat, seq, ts, kind::CHAT, body.to_vec())
}

fn count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn put_event_dedups() {
    let c = open_in_memory().unwrap();
    let e = chat(&me(), 1, 1000, b"a");
    assert!(matches!(
        events::put_event(&c, &e, 1000, None).unwrap(),
        PutResult::Inserted(_)
    ));
    assert_eq!(
        events::put_event(&c, &e, 1000, None).unwrap(),
        PutResult::Duplicate
    );
    assert_eq!(count(&c), 1);
}

#[test]
fn fork_marks_both_rows() {
    let c = open_in_memory().unwrap();
    let a = chat(&me(), 1, 1000, b"a");
    let b = chat(&me(), 1, 1001, b"b");
    events::put_event(&c, &a, 1000, None).unwrap();
    assert!(matches!(
        events::put_event(&c, &b, 1001, None).unwrap(),
        PutResult::Fork(_)
    ));
    let forks: i64 = c
        .query_row("SELECT COUNT(*) FROM events WHERE is_fork = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(forks, 2);
}

#[test]
fn stored_event_roundtrips_and_still_verifies() {
    let c = open_in_memory().unwrap();
    let e = chat(&me(), 1, 1000, b"hello");
    events::put_event(&c, &e, 1000, Some(Delivery::Pending)).unwrap();
    let back = events::query_by_author(&c, &ROOM, &me().member_id(), Stream::Chat).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].event, e);
    assert_eq!(back[0].delivery, Some(Delivery::Pending));
    assert_eq!(verify_stateless(&back[0].event, &ROOM), Ok(()));
}

#[test]
fn next_seq_per_stream_and_last_ts_across_streams() {
    let c = open_in_memory().unwrap();
    let m = me();
    let id = m.member_id();
    assert_eq!(events::next_seq(&c, &ROOM, &id, Stream::Chat).unwrap(), 1);
    assert_eq!(events::last_ts(&c, &ROOM, &id).unwrap(), None);
    events::put_event(&c, &chat(&m, 1, 1000, b"a"), 1000, None).unwrap();
    events::put_event(&c, &chat(&m, 2, 2000, b"b"), 2000, None).unwrap();
    let join = build_event(&m, ROOM, Stream::State, 1, 3000, kind::JOIN, vec![]);
    events::put_event(&c, &join, 3000, None).unwrap();
    assert_eq!(events::next_seq(&c, &ROOM, &id, Stream::Chat).unwrap(), 3);
    assert_eq!(events::next_seq(&c, &ROOM, &id, Stream::State).unwrap(), 2);
    assert_eq!(events::last_ts(&c, &ROOM, &id).unwrap(), Some(3000));
}

#[test]
fn timeline_pages_without_gaps_or_repeats() {
    let c = open_in_memory().unwrap();
    let m = me();
    for i in 1..=7u64 {
        events::put_event(&c, &chat(&m, i, i * 1000, &[i as u8]), i * 1000, None).unwrap();
    }
    let page1 = events::timeline(&c, &ROOM, None, Default::default(), 3).unwrap();
    assert_eq!(
        page1.iter().map(|e| e.display_ts).collect::<Vec<_>>(),
        vec![5000, 6000, 7000]
    );
    let first = &page1[0];
    let page2 = events::timeline(
        &c,
        &ROOM,
        Some((first.display_ts, first.event.event_id)),
        Default::default(),
        3,
    )
    .unwrap();
    assert_eq!(
        page2.iter().map(|e| e.display_ts).collect::<Vec<_>>(),
        vec![2000, 3000, 4000]
    );
    let first = &page2[0];
    let page3 = events::timeline(
        &c,
        &ROOM,
        Some((first.display_ts, first.event.event_id)),
        Default::default(),
        3,
    )
    .unwrap();
    assert_eq!(
        page3.iter().map(|e| e.display_ts).collect::<Vec<_>>(),
        vec![1000]
    );
}

#[test]
fn rooms_rules_members_and_read_cursor() {
    let c = open_in_memory().unwrap();
    assert!(rooms::insert_room(&c, &ROOM, b"cipher", &[2u8; 10], RoomStatus::Joining, 5).unwrap());
    assert!(!rooms::insert_room(&c, &ROOM, b"other", &[2u8; 10], RoomStatus::Active, 6).unwrap());
    let r = rooms::get_room(&c, &ROOM).unwrap().unwrap();
    assert_eq!(r.status, RoomStatus::Joining);
    assert_eq!(r.code_cipher, b"cipher".to_vec());
    rooms::set_status(&c, &ROOM, RoomStatus::Active).unwrap();
    rooms::update_cache(&c, &ROOM, Some("开黑"), Some(&me().member_id()), Some(9000)).unwrap();
    let r = rooms::get_room(&c, &ROOM).unwrap().unwrap();
    assert_eq!(
        (r.status, r.title.as_deref(), r.owner_id, r.dissolve_ts),
        (
            RoomStatus::Active,
            Some("开黑"),
            Some(me().member_id()),
            Some(9000)
        )
    );

    let mut rules = Rules::new();
    rules.insert(0, Rule::DEFAULT);
    rules.insert(
        2,
        Rule {
            rule_epoch: 2,
            threshold: 4,
            ..Rule::DEFAULT
        },
    );
    rooms::replace_rules(&c, &ROOM, &rules).unwrap();
    assert_eq!(rooms::get_rules(&c, &ROOM).unwrap(), rules);

    let id = me().member_id();
    rooms::set_members(
        &c,
        &ROOM,
        &[id].into_iter().collect(),
        &[(id, "我".to_string())].into_iter().collect(),
        &[(id, vec![1u8, 2, 3])].into_iter().collect(),
    )
    .unwrap();
    rooms::set_stacks(&c, &ROOM, &id, 3).unwrap();
    let ms = rooms::members(&c, &ROOM).unwrap();
    assert_eq!(ms.len(), 1);
    assert_eq!(
        (ms[0].in_room, ms[0].stacks, ms[0].nickname.as_deref()),
        (true, 3, Some("我"))
    );
    assert_eq!(ms[0].avatar, Some(vec![1, 2, 3]));
    // 新的 Profile 没带头像：头像清空
    rooms::set_members(
        &c,
        &ROOM,
        &[id].into_iter().collect(),
        &[(id, "我".to_string())].into_iter().collect(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(rooms::members(&c, &ROOM).unwrap()[0].avatar, None);
    // 退出后不在房，层数保留
    rooms::set_members(
        &c,
        &ROOM,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let ms = rooms::members(&c, &ROOM).unwrap();
    assert_eq!((ms[0].in_room, ms[0].stacks), (false, 3));

    assert_eq!(rooms::read_cursor(&c, &ROOM).unwrap(), 0);
    rooms::set_read_cursor(&c, &ROOM, 42).unwrap();
    assert_eq!(rooms::read_cursor(&c, &ROOM).unwrap(), 42);
}

#[test]
fn version_1_database_upgrades_without_losing_data() {
    let path = std::env::temp_dir().join(format!(
        "lolgang-v1-{}.db",
        crate::crypto::random_bytes::<8>()
            .map(u64::from_le_bytes)
            .unwrap()
    ));
    {
        // 按第 1 版建库并写入数据，模拟 M1-3 用户的数据库
        let c = Connection::open(&path).unwrap();
        c.execute_batch(schema::MIGRATIONS[0]).unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        events::put_event(&c, &chat(&me(), 1, 1000, b"a"), 1000, None).unwrap();
    }
    {
        let c = open(&path).unwrap();
        let version: u32 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, schema::MIGRATIONS.len());
        assert_eq!(count(&c), 1);
        settings::set(&c, "k", "v").unwrap();
        assert_eq!(settings::get(&c, "k").unwrap().as_deref(), Some("v"));
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn reopening_a_file_keeps_data() {
    let path = std::env::temp_dir().join(format!(
        "lolgang-test-{}.db",
        crate::crypto::random_bytes::<8>()
            .map(u64::from_le_bytes)
            .unwrap()
    ));
    {
        let c = open(&path).unwrap();
        events::put_event(&c, &chat(&me(), 1, 1000, b"a"), 1000, None).unwrap();
    }
    {
        let c = open(&path).unwrap();
        assert_eq!(count(&c), 1);
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}
