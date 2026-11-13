//! `gang2.db` 的表结构（2.0 架构第 8 节）。每个元素是一次升级，下标 + 1 就是升级后的版本号，只能在末尾追加，不能改已有的。
//! 第 1 版（2.0-M1）只有设置；第 2 版（2.0-M2）加入身份、队伍、成员和状态事件；第 3 版（2.0-M3）给成员加被叠次数；
//! 第 4 版（3.0-M4）加历史记录（文字、语音、信号、表情）；第 5 版给 rooms 加云端 6 位短码；
//! 第 6 版把旧版队伍的 owner_tag 截成 10 字节（早期版本存的是 32 字节完整摘要，3.x 读取会报错）。
//! 和 1.x 的 `lolgang.db` 相比去掉了聊天要的表和列：已读游标、聊天补齐分界、清空记录、隐藏、心之钢规则、事件的 stream 和显示时间。

pub const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5, V6];

const V1: &str =
    "CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL);";

const V2: &str = "
CREATE TABLE identity (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    member_id BLOB NOT NULL,
    nickname TEXT NOT NULL DEFAULT '',
    secret_cipher BLOB NOT NULL,
    avatar BLOB
);

CREATE TABLE rooms (
    room_id BLOB PRIMARY KEY,
    code_cipher BLOB NOT NULL,
    owner_tag BLOB NOT NULL,
    status INTEGER NOT NULL,
    title TEXT,
    owner_id BLOB,
    dissolve_ts INTEGER,
    last_active INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE members (
    room_id BLOB NOT NULL,
    member_id BLOB NOT NULL,
    nickname TEXT,
    avatar BLOB,
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
    seq INTEGER NOT NULL,
    ts_ms INTEGER NOT NULL,
    kind INTEGER NOT NULL,
    body BLOB NOT NULL,
    sig BLOB NOT NULL,
    delivery INTEGER,
    is_fork INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX events_by_author ON events (room_id, author_id, seq);
CREATE INDEX events_by_local ON events (room_id, local_seq);
";

const V3: &str = "ALTER TABLE members ADD COLUMN taken INTEGER NOT NULL DEFAULT 0;";

const V4: &str = "
CREATE TABLE history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    room_id BLOB NOT NULL,
    member_id BLOB NOT NULL,
    ts_ms INTEGER NOT NULL,
    kind INTEGER NOT NULL,
    text TEXT NOT NULL DEFAULT '',
    voice BLOB,
    voice_ms INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX history_by_room ON history (room_id, id);
";

const V5: &str = "ALTER TABLE rooms ADD COLUMN short_code TEXT;";

/// 早期版本 owner_tag 存的是 32 字节完整 SHA-256，3.x 起只用前 10 字节（`owner_tag_of`），
/// 读旧行会触发「Cannot read 10 byte value out of 32 byte blob」。截前 10 字节正好等价
/// 于现在派生的 owner_tag，队伍码里存的就是同一段，语义不变。
const V6: &str =
    "UPDATE rooms SET owner_tag = substr(owner_tag, 1, 10) WHERE length(owner_tag) > 10;";
