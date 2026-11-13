//! 存储层测试：事件入库去重与分叉、序号与时间、队伍相关表、设置、从 M1 的库升级、重新打开文件。

use super::events::{self, Delivery, PutResult};
use super::rooms::{self, RoomStatus};
use super::*;
use crate::crypto::Identity;
use crate::protocol::event::{SignedEvent, build_event, kind, verify_stateless};
use crate::settings2::Settings;

const ROOM: [u8; 16] = [1u8; 16];

fn me() -> Identity {
    Identity::from_secret(&[7u8; 32])
}

fn ev(who: &Identity, seq: u64, ts: u64, body: &[u8]) -> SignedEvent {
    build_event(who, ROOM, seq, ts, kind::RENAME, body.to_vec())
}

fn count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap()
}

fn temp_db(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "gang2-{tag}-{}.db",
        crate::crypto::random_bytes::<8>()
            .map(u64::from_le_bytes)
            .unwrap()
    ))
}

fn remove_db(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn put_event_dedups() {
    let c = open_in_memory().unwrap();
    let e = ev(&me(), 1, 1000, b"a");
    assert!(matches!(
        events::put_event(&c, &e, None).unwrap(),
        PutResult::Inserted(_)
    ));
    assert_eq!(
        events::put_event(&c, &e, None).unwrap(),
        PutResult::Duplicate
    );
    assert_eq!(count(&c), 1);
}

#[test]
fn fork_marks_both_rows() {
    let c = open_in_memory().unwrap();
    let a = ev(&me(), 1, 1000, b"a");
    let b = ev(&me(), 1, 1001, b"b");
    events::put_event(&c, &a, None).unwrap();
    assert!(matches!(
        events::put_event(&c, &b, None).unwrap(),
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
    let e = ev(&me(), 1, 1000, b"hello");
    events::put_event(&c, &e, Some(Delivery::Pending)).unwrap();
    let back = events::query_by_author(&c, &ROOM, &me().member_id()).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].event, e);
    assert_eq!(back[0].delivery, Some(Delivery::Pending));
    assert_eq!(verify_stateless(&back[0].event, &ROOM), Ok(()));
}

#[test]
fn next_seq_and_last_ts() {
    let c = open_in_memory().unwrap();
    let m = me();
    let id = m.member_id();
    assert_eq!(events::next_seq(&c, &ROOM, &id).unwrap(), 1);
    assert_eq!(events::last_ts(&c, &ROOM, &id).unwrap(), None);
    events::put_event(&c, &ev(&m, 1, 1000, b"a"), None).unwrap();
    events::put_event(&c, &ev(&m, 2, 3000, b"b"), None).unwrap();
    assert_eq!(events::next_seq(&c, &ROOM, &id).unwrap(), 3);
    assert_eq!(events::last_ts(&c, &ROOM, &id).unwrap(), Some(3000));
    // 本人写事件：序号接着往下，时间严格递增
    let e = events::append_own(&c, &m, &ROOM, kind::JOIN, vec![], 2000).unwrap();
    assert_eq!((e.event.seq, e.event.ts_ms), (3, 3001));
    assert_eq!(events::pending_own(&c, &ROOM, &id).unwrap().len(), 1);
    assert_eq!(
        events::mark_sent(&c, &ROOM, &id, 3).unwrap(),
        vec![e.event_id]
    );
    assert!(events::pending_own(&c, &ROOM, &id).unwrap().is_empty());
}

#[test]
fn rooms_and_members() {
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

    let id = me().member_id();
    rooms::set_members(
        &c,
        &ROOM,
        &[id].into_iter().collect(),
        &[(id, "我".to_string())].into_iter().collect(),
        &[(id, vec![1u8, 2, 3])].into_iter().collect(),
    )
    .unwrap();
    let ms = rooms::members(&c, &ROOM).unwrap();
    assert_eq!(ms.len(), 1);
    assert_eq!(
        (ms[0].in_room, ms[0].nickname.as_deref()),
        (true, Some("我"))
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
    // 退出后不在队，人还在列表里
    rooms::set_members(
        &c,
        &ROOM,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let ms = rooms::members(&c, &ROOM).unwrap();
    assert_eq!(
        (ms[0].in_room, ms[0].nickname.as_deref()),
        (false, Some("我"))
    );

    events::put_event(&c, &ev(&me(), 1, 1000, b"a"), None).unwrap();
    rooms::delete_room(&c, &ROOM).unwrap();
    assert!(rooms::get_room(&c, &ROOM).unwrap().is_none());
    assert!(rooms::members(&c, &ROOM).unwrap().is_empty());
    assert_eq!(count(&c), 0);
}

#[test]
fn fresh_database_reads_default_settings() {
    let c = open_in_memory().unwrap();
    assert_eq!(settings::load(&c).unwrap(), Settings::default());
}

#[test]
fn saved_settings_read_back() {
    let c = open_in_memory().unwrap();
    let s = Settings {
        volume: 25,
        theme: "dark-carbon".into(),
        ..Settings::default()
    };
    settings::save(&c, &s).unwrap();
    assert_eq!(settings::load(&c).unwrap(), s);
    let s2 = Settings { muted: true, ..s };
    settings::save(&c, &s2).unwrap();
    assert_eq!(settings::load(&c).unwrap(), s2);
}

#[test]
fn broken_settings_json_reads_as_defaults() {
    let c = open_in_memory().unwrap();
    c.execute("INSERT INTO settings (id, json) VALUES (1, 'not json')", [])
        .unwrap();
    assert_eq!(settings::load(&c).unwrap(), Settings::default());
}

#[test]
fn m1_database_upgrades_and_keeps_settings() {
    let path = temp_db("m1");
    let s = Settings {
        ping_size_px: 150,
        ..Settings::default()
    };
    {
        // 2.0-M1 的库：只有第 1 版的设置表
        let c = Connection::open(&path).unwrap();
        c.execute_batch(schema::MIGRATIONS[0]).unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        settings::save(&c, &s).unwrap();
    }
    {
        let c = open(&path).unwrap();
        let version: u32 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, schema::MIGRATIONS.len());
        assert_eq!(settings::load(&c).unwrap(), s);
        events::put_event(&c, &ev(&me(), 1, 1000, b"a"), None).unwrap();
    }
    {
        let c = open(&path).unwrap();
        assert_eq!(count(&c), 1);
    }
    remove_db(&path);
}

#[test]
fn v5_rooms_with_32_byte_owner_tag_upgrade_to_10() {
    let path = temp_db("v6");
    let old_room: [u8; 16] = [9u8; 16];
    {
        // 第 5 版的库：rooms.owner_tag 是早期版本的 32 字节完整摘要
        let c = Connection::open(&path).unwrap();
        for sql in schema::MIGRATIONS.iter().take(5) {
            c.execute_batch(sql).unwrap();
        }
        c.pragma_update(None, "user_version", 5).unwrap();
        c.execute(
            "INSERT INTO rooms (room_id, code_cipher, owner_tag, status, last_active)
             VALUES (?1, ?2, ?3, 1, 100)",
            rusqlite::params![&old_room[..], vec![0u8; 52], vec![7u8; 32]],
        )
        .unwrap();
    }
    {
        // 升级后旧 tag 截成 10 字节，get_room 不再报
        //「Cannot read 10 byte value out of 32 byte blob」
        let c = open(&path).unwrap();
        let r = rooms::get_room(&c, &old_room).unwrap().unwrap();
        assert_eq!(r.owner_tag, [7u8; 10]);
        assert_eq!(r.status, RoomStatus::Active);
        // 新写入的 10 字节 tag 不受影响
        let new_room: [u8; 16] = [3u8; 16];
        rooms::insert_room(&c, &new_room, b"c", &[4u8; 10], RoomStatus::Joining, 7).unwrap();
        assert_eq!(
            rooms::get_room(&c, &new_room).unwrap().unwrap().owner_tag,
            [4u8; 10]
        );
    }
    remove_db(&path);
}
