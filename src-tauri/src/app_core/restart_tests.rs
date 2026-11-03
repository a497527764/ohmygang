//! 重启保留：用真实的数据库文件，关掉再打开，身份、房间、消息和层数都还在。

use super::tests::DEFAULT_PARAMS;
use super::*;
use crate::keystore::MemoryKeyStore;
use crate::storage;

#[test]
fn everything_survives_reopen() {
    let tag = crate::crypto::random_bytes::<8>()
        .map(u64::from_le_bytes)
        .unwrap();
    let path = std::env::temp_dir().join(format!("lolgang-restart-{tag}.db"));
    let keys = MemoryKeyStore([7u8; 32]);

    let (me, room, code, before_msgs, before_board) = {
        let mut a = Core::open(storage::open(&path).unwrap(), &keys).unwrap();
        a.set_profile("阿钢", None, 500).unwrap();
        let created = a.create_room("开黑", DEFAULT_PARAMS, 1000).unwrap();
        let room = created.summary.room_id;
        for t in [10_000, 11_000, 12_000, 13_000] {
            a.send_text(&room, "冲", t).unwrap();
        }
        let msgs = a.list_messages(&room, None, 50).unwrap();
        let board = a.leaderboard(&room).unwrap();
        (a.identity().member_id, room, created.code, msgs, board)
    };
    assert_eq!(before_board, vec![(me, 1)]);

    {
        let a = Core::open(storage::open(&path).unwrap(), &keys).unwrap();
        assert_eq!(a.identity().member_id, me);
        assert_eq!(a.identity().nickname, "阿钢");
        let rooms = a.list_rooms().unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(
            (rooms[0].title.as_deref(), rooms[0].status),
            (Some("开黑"), RoomStatus::Active)
        );
        assert_eq!(a.room_code(&room).unwrap(), code);
        assert_eq!(a.list_messages(&room, None, 50).unwrap(), before_msgs);
        assert_eq!(a.leaderboard(&room).unwrap(), before_board);
        // 层数缓存和重放结果一致
        let replayed = hs::recompute_author(&a.conn, &room, &me).unwrap();
        assert_eq!((replayed.stacks, replayed.changed), (1, false));
    }

    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}
