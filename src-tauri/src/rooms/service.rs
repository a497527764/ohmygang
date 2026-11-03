//! 建房、进房和房间缓存（架构第 3、8 节）。
//!
//! 实现流程：
//! 1. 建房：生成 16 字节 room_secret，和本人公钥的 owner_tag 组成房间码，派生出 room_id；房间码用本机主密钥加密后存库。
//!    在调用方的事务里写入 RoomCreate（规则 epoch 0）、Join，有昵称时再写 Profile，房间直接是 Active。
//! 2. 进房：解析房间码（校验位不对立即报错），派生 room_id，记一条 Joining 状态的房间。
//!    Joining 转 Active 需要先和同伴同步状态事件，在 M1-3 实现。
//! 3. 缓存：状态事件变化后，用 M1-1 的 derive_room_state 重新推导群名、房主、成员、昵称、规则和解散时间，写回缓存表；
//!    推导出解散时间时，房间转为 Dissolved。缓存都能从事件重建，权威始终是推导结果。

use crate::chat;
use crate::crypto::{self, Identity, MemberId, RoomId};
use crate::error::{AppError, AppResult};
use crate::keystore::{PURPOSE_ROOM_CODE, decrypt_local, encrypt_local};
use crate::protocol::body::{self, Avatar, ProfileBody, RoomCreateBody, Rule, TITLE_MAX_CHARS};
use crate::protocol::event::{SignedEvent, Stream, kind};
use crate::protocol::room_code::{RoomCode, owner_tag_of};
use crate::rooms::state::{RoomState, derive_room_state};
use crate::storage::Connection;
use crate::storage::events;
use crate::storage::rooms::{self as room_store, RoomRow, RoomStatus};

/// 建房时由用户填写的规则参数；rule_epoch 由核心分配（架构 5.1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleParams {
    pub window_ms: u32,
    pub threshold: u32,
    pub cooldown_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomSummary {
    pub room_id: RoomId,
    pub title: Option<String>,
    pub status: RoomStatus,
    pub is_owner: bool,
    pub unread: u32,
    pub last_active: u64,
    pub muted: bool,
    pub pinned: bool,
    /// 最后一条消息（「乙：……」，自己发的不带名字）。
    pub preview: Option<String>,
    /// 未读里有人 @ 我。
    pub mentioned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberView {
    pub member_id: MemberId,
    pub fingerprint: String,
    pub nickname: Option<String>,
    pub stacks: u32,
    pub is_owner: bool,
    pub in_room: bool,
    pub avatar: Option<Avatar>,
}

fn code_bytes(code: &RoomCode) -> Vec<u8> {
    [code.room_secret.as_slice(), &code.owner_tag].concat()
}

fn code_from_bytes(b: &[u8]) -> AppResult<RoomCode> {
    if b.len() != 26 {
        return Err(AppError::Decrypt);
    }
    let mut code = RoomCode {
        room_secret: [0; 16],
        owner_tag: [0; 10],
    };
    code.room_secret.copy_from_slice(&b[..16]);
    code.owner_tag.copy_from_slice(&b[16..]);
    Ok(code)
}

pub fn get(conn: &Connection, room: &RoomId) -> AppResult<RoomRow> {
    room_store::get_room(conn, room)?.ok_or(AppError::RoomNotFound)
}

/// 返回 (room_id, 房间码, 新写的事件)。
pub fn create_room(
    conn: &Connection,
    me: &Identity,
    master: &[u8; 32],
    profile: &ProfileBody,
    title: &str,
    params: RuleParams,
    now_ms: u64,
) -> AppResult<(RoomId, String, Vec<SignedEvent>)> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > TITLE_MAX_CHARS {
        return Err(AppError::InvalidTitle);
    }
    let rule = Rule {
        rule_epoch: 0,
        window_ms: params.window_ms,
        threshold: params.threshold,
        cooldown_ms: params.cooldown_ms,
    };
    if !rule.is_valid() {
        return Err(AppError::InvalidRule);
    }

    let code = RoomCode {
        room_secret: crypto::random_bytes()?,
        owner_tag: owner_tag_of(&me.member_id()),
    };
    let (_, room) = code.keys();
    let cipher = encrypt_local(master, PURPOSE_ROOM_CODE, &code_bytes(&code))?;
    room_store::insert_room(
        conn,
        &room,
        &cipher,
        &code.owner_tag,
        RoomStatus::Active,
        now_ms,
    )?;

    let create = body::encode(&RoomCreateBody {
        title: title.to_string(),
        rule,
    });
    let mut written = vec![
        events::append_own(
            conn,
            me,
            &room,
            Stream::State,
            kind::ROOM_CREATE,
            create,
            now_ms,
        )?,
        events::append_own(conn, me, &room, Stream::State, kind::JOIN, vec![], now_ms)?,
    ];
    if profile.has_content() {
        written.push(events::append_own(
            conn,
            me,
            &room,
            Stream::State,
            kind::PROFILE,
            body::encode(profile),
            now_ms,
        )?);
    }
    apply_state(conn, &room)?;
    Ok((room, code.format(), written))
}

/// 已经在这个房间里就不重复建记录；之前退出过的（Left）重新进入 Joining（架构 8.2）。
pub fn join_room(
    conn: &Connection,
    master: &[u8; 32],
    code_str: &str,
    now_ms: u64,
) -> AppResult<RoomId> {
    let code = RoomCode::parse(code_str)?;
    let (_, room) = code.keys();
    let cipher = encrypt_local(master, PURPOSE_ROOM_CODE, &code_bytes(&code))?;
    if !room_store::insert_room(
        conn,
        &room,
        &cipher,
        &code.owner_tag,
        RoomStatus::Joining,
        now_ms,
    )? && get(conn, &room)?.status == RoomStatus::Left
    {
        room_store::set_status(conn, &room, RoomStatus::Joining)?;
    }
    Ok(room)
}

pub fn room_code(conn: &Connection, master: &[u8; 32], room: &RoomId) -> AppResult<String> {
    let row = get(conn, room)?;
    Ok(code_from_bytes(&decrypt_local(master, PURPOSE_ROOM_CODE, &row.code_cipher)?)?.format())
}

/// 从全部状态事件重新推导房间状态，并写回缓存表。
pub fn apply_state(conn: &Connection, room: &RoomId) -> AppResult<RoomState> {
    let row = get(conn, room)?;
    let state_events: Vec<SignedEvent> = events::query_room_stream(conn, room, Stream::State)?
        .into_iter()
        .map(|s| s.event)
        .collect();
    let st = derive_room_state(&row.owner_tag, &state_events);
    room_store::update_cache(
        conn,
        room,
        st.title.as_deref(),
        st.owner.as_ref(),
        st.dissolve_ts,
    )?;
    room_store::replace_rules(conn, room, &st.rules)?;
    let avatars = st
        .avatars
        .iter()
        .map(|(m, a)| (*m, body::encode(a)))
        .collect();
    room_store::set_members(conn, room, &st.members, &st.nicknames, &avatars)?;
    if st.dissolve_ts.is_some() && matches!(row.status, RoomStatus::Joining | RoomStatus::Active) {
        room_store::set_status(conn, room, RoomStatus::Dissolved)?;
    }
    Ok(st)
}

pub fn summary(conn: &Connection, me: &MemberId, row: &RoomRow) -> AppResult<RoomSummary> {
    let cursor = room_store::read_cursor(conn, &row.room_id)?;
    Ok(RoomSummary {
        room_id: row.room_id,
        title: row.title.clone(),
        status: row.status,
        is_owner: owner_tag_of(me) == row.owner_tag,
        unread: events::count_unread(conn, &row.room_id, me, cursor, chat::visible(row, true))?,
        last_active: row.last_active,
        muted: row.muted,
        pinned: row.pinned,
        preview: chat::preview(conn, row, me)?,
        mentioned: chat::mentioned(conn, row, me)?,
    })
}

/// 房间里出现过的所有人（含已退出的），按层数降序。
pub fn list_members(conn: &Connection, room: &RoomId) -> AppResult<Vec<MemberView>> {
    let row = get(conn, room)?;
    Ok(room_store::members(conn, room)?
        .into_iter()
        .map(|m| MemberView {
            fingerprint: crypto::fingerprint(&m.member_id),
            is_owner: owner_tag_of(&m.member_id) == row.owner_tag,
            member_id: m.member_id,
            nickname: m.nickname,
            stacks: m.stacks,
            in_room: m.in_room,
            avatar: m.avatar.and_then(|b| body::decode::<Avatar>(&b).ok()),
        })
        .collect())
}

/// 排行榜只列当前成员（需求 3.2），按层数降序。
pub fn leaderboard(conn: &Connection, room: &RoomId) -> AppResult<Vec<(MemberId, u32)>> {
    Ok(room_store::members(conn, room)?
        .into_iter()
        .filter(|m| m.in_room)
        .map(|m| (m.member_id, m.stacks))
        .collect())
}

/// 当前规则：epoch 最大的一版。
pub fn current_rule(conn: &Connection, room: &RoomId) -> AppResult<Option<Rule>> {
    let rules = room_store::get_rules(conn, room)?;
    Ok(rules.keys().max().and_then(|e| rules.get(e)).copied())
}

/// 这个房间的包加密密钥（由房间码派生）。
pub fn room_key(
    conn: &Connection,
    master: &[u8; 32],
    room: &RoomId,
) -> AppResult<crate::crypto::RoomKey> {
    let row = get(conn, room)?;
    let code = code_from_bytes(&decrypt_local(master, PURPOSE_ROOM_CODE, &row.code_cipher)?)?;
    Ok(code.keys().0)
}
