//! 心之钢的有状态部分（架构 5.3、5.4、5.6）：从库里取事件，调用 M1-1 的纯函数，并维护层数缓存。
//!
//! 实现流程：
//! 1. 取某作者的全部叠层事件（状态流里 kind = Heartsteel），解码失败的视为无效。
//! 2. `recompute_author`：用房间全部规则版本和解散时间重放，层数写回 members 缓存，和入库在同一事务。
//! 3. `recompute_room`：规则或解散变化后，对房间里每个发过叠层（或已有层数缓存）的人重算。
//! 4. `evaluate_on_send`：只被发言调用。取本人在该房间的全部聊天、最后一条有效叠层、全部规则和当前规则，
//!    调用 evaluate_emit。同步回来的本人旧消息不走这里，所以恢复备份不会补算出叠层。
//! 5. `is_live_sound`：实时收到、有效、且最后一条触发消息离现在不超过 60 秒，才播放音效。

use std::collections::BTreeSet;

use super::{ChatPoint, HsEvent, evaluate_emit, replay_author};
use crate::crypto::{MemberId, RoomId};
use crate::error::AppResult;
use crate::heartsteel::Rules;
use crate::protocol::body::{self, HeartsteelBody};
use crate::protocol::event::{Stream, kind};
use crate::storage::Connection;
use crate::storage::events;
use crate::storage::rooms as room_store;

pub const LIVE_WINDOW_MS: u64 = 60_000;

/// 某作者重放后的结果。
pub struct Replayed {
    pub stacks: u32,
    pub changed: bool,
    pub valid: Vec<HsEvent>,
}

fn load_author(conn: &Connection, room: &RoomId, author: &MemberId) -> AppResult<Vec<HsEvent>> {
    Ok(
        events::query_kind_by_author(conn, room, author, kind::HEARTSTEEL)?
            .into_iter()
            .filter(|s| s.event.event.stream == Stream::State)
            .filter_map(|s| {
                let b = body::decode::<HeartsteelBody>(&s.event.event.body).ok()?;
                Some(HsEvent {
                    seq: s.event.event.seq,
                    event_id: s.event.event_id,
                    body: b,
                })
            })
            .collect(),
    )
}

fn context(conn: &Connection, room: &RoomId) -> AppResult<(Rules, Option<u64>)> {
    let rules = room_store::get_rules(conn, room)?;
    let dissolve_ts = room_store::get_room(conn, room)?.and_then(|r| r.dissolve_ts);
    Ok((rules, dissolve_ts))
}

pub fn recompute_author(
    conn: &Connection,
    room: &RoomId,
    author: &MemberId,
) -> AppResult<Replayed> {
    let (rules, dissolve_ts) = context(conn, room)?;
    let all = load_author(conn, room, author)?;
    let valid: Vec<HsEvent> = replay_author(&all, &rules, dissolve_ts)
        .into_iter()
        .cloned()
        .collect();
    let stacks = valid.len() as u32;
    let old = room_store::stacks_of(conn, room, author)?;
    if old != stacks {
        room_store::set_stacks(conn, room, author, stacks)?;
    }
    Ok(Replayed {
        stacks,
        changed: old != stacks,
        valid,
    })
}

pub fn recompute_room(conn: &Connection, room: &RoomId) -> AppResult<Vec<(MemberId, Replayed)>> {
    let mut authors: BTreeSet<MemberId> = events::heartsteel_authors(conn, room)?
        .into_iter()
        .collect();
    authors.extend(
        room_store::members(conn, room)?
            .into_iter()
            .filter(|m| m.stacks > 0)
            .map(|m| m.member_id),
    );
    authors
        .into_iter()
        .map(|a| Ok((a, recompute_author(conn, room, &a)?)))
        .collect()
}

/// 本人刚写入一条聊天后调用；返回要写的叠层事件内容，没触发返回 None。
pub fn evaluate_on_send(
    conn: &Connection,
    me: &MemberId,
    room: &RoomId,
) -> AppResult<Option<HeartsteelBody>> {
    let (rules, dissolve_ts) = context(conn, room)?;
    let Some(current) = rules.keys().max().and_then(|e| rules.get(e)).copied() else {
        return Ok(None);
    };
    let chats: Vec<ChatPoint> = events::query_kind_by_author(conn, room, me, kind::CHAT)?
        .into_iter()
        .filter(|s| s.event.event.stream == Stream::Chat)
        .map(|s| ChatPoint {
            event_id: s.event.event_id,
            ts_ms: s.event.event.ts_ms,
        })
        .collect();
    let mine = load_author(conn, room, me)?;
    let valid = replay_author(&mine, &rules, dissolve_ts);
    Ok(
        evaluate_emit(&chats, valid.last().copied(), &rules, &current).map(|triggers| {
            HeartsteelBody {
                rule_epoch: current.rule_epoch,
                triggers,
            }
        }),
    )
}

pub fn is_live_sound(is_live: bool, h: &HsEvent, now_ms: u64) -> bool {
    is_live
        && h.body
            .triggers
            .last()
            .is_some_and(|t| now_ms.abs_diff(t.ts_ms) <= LIVE_WINDOW_MS)
}
