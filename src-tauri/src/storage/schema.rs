//! 表结构（架构 9.1）。每个元素是一次升级，下标 + 1 就是升级后的版本号，只能在末尾追加，不能改已有的。
//! 第 2 版（M1-4）加入 `settings`；`sync_state` 还用不到，留给 M2 以新版本加入。

pub const MIGRATIONS: &[&str] = &[V1, V2];

const V1: &str = "
CREATE TABLE identity (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    member_id BLOB NOT NULL,
    nickname TEXT NOT NULL DEFAULT '',
    secret_cipher BLOB NOT NULL
);

CREATE TABLE rooms (
    room_id BLOB PRIMARY KEY,
    code_cipher BLOB NOT NULL,
    owner_tag BLOB NOT NULL,
    status INTEGER NOT NULL,
    title TEXT,
    owner_id BLOB,
    dissolve_ts INTEGER,
    last_active INTEGER NOT NULL DEFAULT 0,
    muted INTEGER NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE room_rules (
    room_id BLOB NOT NULL,
    rule_epoch INTEGER NOT NULL,
    window_ms INTEGER NOT NULL,
    threshold INTEGER NOT NULL,
    cooldown_ms INTEGER NOT NULL,
    PRIMARY KEY (room_id, rule_epoch)
);

CREATE TABLE members (
    room_id BLOB NOT NULL,
    member_id BLOB NOT NULL,
    nickname TEXT,
    in_room INTEGER NOT NULL DEFAULT 0,
    stacks INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (room_id, member_id)
);

CREATE TABLE events (
    local_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id BLOB NOT NULL UNIQUE,
    room_id BLOB NOT NULL,
    author_id BLOB NOT NULL,
    format INTEGER NOT NULL,
    stream INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    ts_ms INTEGER NOT NULL,
    kind INTEGER NOT NULL,
    body BLOB NOT NULL,
    sig BLOB NOT NULL,
    display_ts INTEGER NOT NULL,
    delivery INTEGER,
    is_fork INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX events_by_author ON events (room_id, author_id, stream, seq);
CREATE INDEX events_by_display ON events (room_id, display_ts, event_id);
CREATE INDEX events_by_local ON events (room_id, local_seq);

CREATE TABLE read_state (
    room_id BLOB PRIMARY KEY,
    read_local_seq INTEGER NOT NULL DEFAULT 0
);
";

const V2: &str = "
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";
