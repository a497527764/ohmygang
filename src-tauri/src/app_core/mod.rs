//! 本机核心：接口文档第 1 节的 `core` 模块（Rust 标准库已有 `core`，为避免重名叫 `app_core`）。
//!
//! 实现流程：
//! 1. 启动：从凭据管理器取本机主密钥，读出或生成本机身份。
//! 2. 每个对外操作开一个数据库事务，调用各业务模块，全部成功才提交；
//!    返回本次新写的事件（M1-3 交给 sync 发出去）和给界面的通知。
//! 3. 入站（`ingest_events`，接口文档第 1 节）：无状态校验不过的丢弃；入库（重复跳过、分叉提示）；
//!    状态事件有变化就重新推导房间缓存；建房、改规则、解散会影响所有人的层数，重算全房，否则只重算发了叠层的作者；
//!    最后为新的聊天和系统提示、层数变化、实时叠层生成给界面的通知。整批在一个事务里。
//! 4. 它是唯一知道所有模块的地方，业务模块之间不互相回调。

use std::collections::BTreeSet;

use crate::chat;
use crate::crypto::{EventId, Identity, MemberId, RoomId, RoomKey};
use crate::error::AppResult;
use crate::heartsteel::service::{self as hs, Replayed};
use crate::identity::{self, IdentityView};
use crate::keystore::KeyStore;
use crate::protocol::body::{self, ProfileBody, Rule};
use crate::protocol::event::{SignedEvent, Stream, kind, verify_stateless};
use crate::protocol::frame::{HaveEntry, WantRange, short_id};
use crate::rooms::service as rooms;
use crate::settings;
use crate::storage::events::{self, PutResult, StoredEvent};
use crate::storage::{Connection, rooms as room_store};
use crate::sync::plan::Holdings;

pub use crate::chat::{ChatMessage, TimelineCursor, TimelineItem};
pub use crate::error::AppError;
pub use crate::rooms::service::{MemberView, RoomSummary, RuleParams};
pub use crate::settings::{LocalSettings, SettingsPatch};
pub use crate::storage::events::Delivery;
pub use crate::storage::rooms::RoomStatus;

/// 发言的结果：消息本身、本次新写的事件（M1-3 交给 sync 发出去）、给界面的通知。
pub struct SentMessage {
    pub message: ChatMessage,
    pub events: Vec<SignedEvent>,
    pub notes: Vec<CoreEvent>,
}

/// 给界面的通知（接口文档各模块的 Events）。M1-5 把它们转发成 Tauri 事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreEvent {
    TimelineItemAdded {
        room_id: RoomId,
        item: TimelineItem,
        is_live: bool,
    },
    StacksChanged {
        room_id: RoomId,
        changes: Vec<(MemberId, u32)>,
    },
    HeartsteelLive {
        room_id: RoomId,
        member_id: MemberId,
        new_stacks: u32,
        event_id: EventId,
    },
    RoomUpdated {
        room_id: RoomId,
    },
    ForkDetected {
        room_id: RoomId,
        member_id: MemberId,
    },
}

pub struct CreatedRoom {
    pub summary: RoomSummary,
    pub code: String,
    pub events: Vec<SignedEvent>,
}

pub struct Core {
    conn: Connection,
    me: Identity,
    view: IdentityView,
    master: [u8; 32],
}

impl Core {
    pub fn open(conn: Connection, keystore: &dyn KeyStore) -> AppResult<Self> {
        let master = keystore.master_key()?;
        let (me, view) = identity::bootstrap(&conn, &master)?;
        Ok(Self {
            conn,
            me,
            view,
            master,
        })
    }

    pub fn identity(&self) -> &IdentityView {
        &self.view
    }

    /// 改全局昵称，并在每个 Active 房间写一条 Profile 事件（需求 3.1）。
    pub fn set_nickname(&mut self, nickname: &str, now_ms: u64) -> AppResult<Vec<SignedEvent>> {
        let tx = self.conn.transaction()?;
        identity::set_nickname(&tx, nickname)?;
        let mut written = Vec::new();
        for row in room_store::list_rooms(&tx)? {
            if row.status != RoomStatus::Active {
                continue;
            }
            let profile = body::encode(&ProfileBody {
                nickname: nickname.to_string(),
            });
            written.push(events::append_own(
                &tx,
                &self.me,
                &row.room_id,
                Stream::State,
                kind::PROFILE,
                profile,
                now_ms,
            )?);
            rooms::apply_state(&tx, &row.room_id)?;
        }
        tx.commit()?;
        self.view.nickname = nickname.to_string();
        Ok(written)
    }

    pub fn create_room(
        &mut self,
        title: &str,
        params: RuleParams,
        now_ms: u64,
    ) -> AppResult<CreatedRoom> {
        let tx = self.conn.transaction()?;
        let (room, code, written) = rooms::create_room(
            &tx,
            &self.me,
            &self.master,
            &self.view.nickname,
            title,
            params,
            now_ms,
        )?;
        let summary = rooms::summary(&tx, &self.view.member_id, &rooms::get(&tx, &room)?)?;
        tx.commit()?;
        Ok(CreatedRoom {
            summary,
            code,
            events: written,
        })
    }

    pub fn join_room(&mut self, code: &str, now_ms: u64) -> AppResult<RoomSummary> {
        let tx = self.conn.transaction()?;
        let room = rooms::join_room(&tx, &self.master, code, now_ms)?;
        let summary = rooms::summary(&tx, &self.view.member_id, &rooms::get(&tx, &room)?)?;
        tx.commit()?;
        Ok(summary)
    }

    pub fn list_rooms(&self) -> AppResult<Vec<RoomSummary>> {
        room_store::list_rooms(&self.conn)?
            .iter()
            .map(|row| rooms::summary(&self.conn, &self.view.member_id, row))
            .collect()
    }

    pub fn room_code(&self, room: &RoomId) -> AppResult<String> {
        rooms::room_code(&self.conn, &self.master, room)
    }

    pub fn room_rule(&self, room: &RoomId) -> AppResult<Option<Rule>> {
        rooms::current_rule(&self.conn, room)
    }

    pub fn list_members(&self, room: &RoomId) -> AppResult<Vec<MemberView>> {
        rooms::list_members(&self.conn, room)
    }

    pub fn leaderboard(&self, room: &RoomId) -> AppResult<Vec<(MemberId, u32)>> {
        rooms::leaderboard(&self.conn, room)
    }

    /// 本人发言。和叠层判定在同一个事务里；本人实时叠层同样产生 HeartsteelLive，用于播放音效。
    pub fn send_text(&mut self, room: &RoomId, text: &str, now_ms: u64) -> AppResult<SentMessage> {
        let tx = self.conn.transaction()?;
        let (message, written, stacked) = chat::send_text(&tx, &self.me, room, text, now_ms)?;
        tx.commit()?;
        let mut notes = Vec::new();
        if let Some(s) = stacked {
            let me = self.view.member_id;
            if s.replayed.changed {
                notes.push(CoreEvent::StacksChanged {
                    room_id: *room,
                    changes: vec![(me, s.replayed.stacks)],
                });
            }
            if s.replayed
                .valid
                .iter()
                .any(|h| h.event_id == s.event_id && hs::is_live_sound(true, h, now_ms))
            {
                notes.push(CoreEvent::HeartsteelLive {
                    room_id: *room,
                    member_id: me,
                    new_stacks: s.replayed.stacks,
                    event_id: s.event_id,
                });
            }
        }
        Ok(SentMessage {
            message,
            events: written,
            notes,
        })
    }

    /// 聊天窗口的一页（正序）。`before` 为 None 时取最新一页；翻页时传入上一页第一条的 `cursor()`。
    pub fn list_messages(
        &self,
        room: &RoomId,
        before: Option<TimelineCursor>,
        limit: u32,
    ) -> AppResult<Vec<TimelineItem>> {
        chat::list_messages(&self.conn, room, before, limit)
    }

    pub fn unread(&self, room: &RoomId) -> AppResult<u32> {
        chat::unread(&self.conn, room, &self.view.member_id)
    }

    pub fn mark_read(&mut self, room: &RoomId) -> AppResult<()> {
        chat::mark_read(&self.conn, room)
    }

    /// 房间的本机偏好（免打扰、置顶），`None` 表示不改。只影响本机。
    pub fn set_room_prefs(
        &mut self,
        room: &RoomId,
        muted: Option<bool>,
        pinned: Option<bool>,
    ) -> AppResult<RoomSummary> {
        if !room_store::set_prefs(&self.conn, room, muted, pinned)? {
            return Err(AppError::RoomNotFound);
        }
        rooms::summary(
            &self.conn,
            &self.view.member_id,
            &rooms::get(&self.conn, room)?,
        )
    }

    pub fn settings(&self) -> AppResult<LocalSettings> {
        settings::load(&self.conn)
    }

    pub fn update_settings(&mut self, patch: SettingsPatch) -> AppResult<LocalSettings> {
        settings::update(&self.conn, patch)
    }

    // ---- 以下供同步（节点）使用 ----

    pub fn room_key(&self, room: &RoomId) -> AppResult<RoomKey> {
        rooms::room_key(&self.conn, &self.master, room)
    }

    pub fn room_status(&self, room: &RoomId) -> AppResult<RoomStatus> {
        Ok(rooms::get(&self.conn, room)?.status)
    }

    /// 需要联网的房间：除了已退出的都要（架构 7.5）。
    pub fn sync_rooms(&self) -> AppResult<Vec<(RoomId, RoomStatus)>> {
        Ok(room_store::list_rooms(&self.conn)?
            .into_iter()
            .filter(|r| r.status != RoomStatus::Left)
            .map(|r| (r.room_id, r.status))
            .collect())
    }

    pub fn holdings(&self, room: &RoomId) -> AppResult<Holdings> {
        let mut h = Holdings::default();
        for (author, stream, seq) in events::holdings_rows(&self.conn, room)? {
            h.add(short_id(&author), stream, seq);
        }
        Ok(h)
    }

    pub fn have_vector(&self, room: &RoomId) -> AppResult<Vec<HaveEntry>> {
        Ok(self.holdings(room)?.vector())
    }

    /// 回应 Want：按短 ID 和序号区间取本机持有的事件。
    pub fn events_for(&self, room: &RoomId, ranges: &[WantRange]) -> AppResult<Vec<SignedEvent>> {
        let authors = events::room_authors(&self.conn, room)?;
        let mut out = Vec::new();
        for r in ranges {
            let stream = if r.stream == 0 {
                Stream::State
            } else {
                Stream::Chat
            };
            for a in authors.iter().filter(|a| short_id(a) == r.author) {
                let found = events::query_range(&self.conn, room, a, stream, r.from_seq, r.to_seq)?;
                out.extend(found.into_iter().map(|s| s.event));
            }
        }
        Ok(out)
    }

    /// 已拿到能对上 owner_tag 的建房事件。
    pub fn owner_known(&self, room: &RoomId) -> AppResult<bool> {
        Ok(rooms::get(&self.conn, room)?.owner_id.is_some())
    }

    /// Joining 转 Active（架构 7.4）：写入本人的 Join（还不是成员时）和 Profile，已读游标移到当前最大值。
    /// 不是 Joining 时什么都不做。返回新写的事件。
    pub fn activate(&mut self, room: &RoomId, now_ms: u64) -> AppResult<Vec<SignedEvent>> {
        let tx = self.conn.transaction()?;
        if rooms::get(&tx, room)?.status != RoomStatus::Joining {
            return Ok(Vec::new());
        }
        room_store::set_status(&tx, room, RoomStatus::Active)?;
        let me = self.view.member_id;
        let already_in = room_store::members(&tx, room)?
            .iter()
            .any(|m| m.member_id == me && m.in_room);
        let mut written = Vec::new();
        if !already_in {
            written.push(events::append_own(
                &tx,
                &self.me,
                room,
                Stream::State,
                kind::JOIN,
                vec![],
                now_ms,
            )?);
        }
        if !self.view.nickname.is_empty() {
            let profile = body::encode(&ProfileBody {
                nickname: self.view.nickname.clone(),
            });
            written.push(events::append_own(
                &tx,
                &self.me,
                room,
                Stream::State,
                kind::PROFILE,
                profile,
                now_ms,
            )?);
        }
        rooms::apply_state(&tx, room)?;
        chat::mark_read(&tx, room)?;
        tx.commit()?;
        Ok(written)
    }

    /// 房主解散房间（需求 3.2，不可撤销）。解散后的叠层无效，所以重算全房层数。返回新写的解散事件。
    pub fn dissolve_room(&mut self, room: &RoomId, now_ms: u64) -> AppResult<Vec<SignedEvent>> {
        let tx = self.conn.transaction()?;
        let row = rooms::get(&tx, room)?;
        if crate::protocol::room_code::owner_tag_of(&self.view.member_id) != row.owner_tag {
            return Err(AppError::NotOwner);
        }
        if row.status != RoomStatus::Active {
            return Err(AppError::RoomNotActive);
        }
        let e = events::append_own(
            &tx,
            &self.me,
            room,
            Stream::State,
            kind::DISSOLVE,
            vec![],
            now_ms,
        )?;
        rooms::apply_state(&tx, room)?;
        hs::recompute_room(&tx, room)?;
        tx.commit()?;
        Ok(vec![e])
    }

    /// 房主最早的一条解散事件，用于告诉还不知道解散的同伴（架构 7.5）。
    pub fn dissolve_event(&self, room: &RoomId) -> AppResult<Option<SignedEvent>> {
        let Some(owner) = rooms::get(&self.conn, room)?.owner_id else {
            return Ok(None);
        };
        Ok(
            events::query_kind_by_author(&self.conn, room, &owner, kind::DISSOLVE)?
                .into_iter()
                .find(|s| s.event.event.stream == Stream::State)
                .map(|s| s.event),
        )
    }

    /// 处理收到的一批事件。`is_live` 表示来自实时帧（会影响显示时间和是否播放音效）。
    pub fn ingest_events(
        &mut self,
        room: &RoomId,
        incoming: &[SignedEvent],
        is_live: bool,
        now_ms: u64,
    ) -> AppResult<Vec<CoreEvent>> {
        let tx = self.conn.transaction()?;
        rooms::get(&tx, room)?;
        let mut notes = Vec::new();
        let mut state_changed = false;
        let mut room_wide = false;
        let mut hs_authors = BTreeSet::new();
        let mut new_hs = Vec::new();
        let mut new_items = Vec::new();

        for e in incoming {
            if verify_stateless(e, room).is_err() {
                continue;
            }
            let ev = &e.event;
            let shown = chat::display_ts(ev.ts_ms, now_ms, is_live);
            let local_seq = match events::put_event(&tx, e, shown, None)? {
                PutResult::Duplicate => continue,
                PutResult::Fork(l) => {
                    notes.push(CoreEvent::ForkDetected {
                        room_id: *room,
                        member_id: ev.author_id,
                    });
                    l
                }
                PutResult::Inserted(l) => l,
            };
            if ev.stream == Stream::State {
                if ev.kind == kind::HEARTSTEEL {
                    hs_authors.insert(ev.author_id);
                    new_hs.push((ev.author_id, e.event_id));
                } else {
                    state_changed = true;
                    room_wide |= matches!(
                        ev.kind,
                        kind::ROOM_CREATE | kind::CONFIG_NOTICE | kind::DISSOLVE
                    );
                }
            }
            new_items.push(StoredEvent {
                event: e.clone(),
                local_seq,
                display_ts: shown,
                delivery: None,
                is_fork: false,
            });
        }

        if state_changed {
            rooms::apply_state(&tx, room)?;
            notes.push(CoreEvent::RoomUpdated { room_id: *room });
        }

        let replayed: Vec<(MemberId, Replayed)> = if room_wide {
            hs::recompute_room(&tx, room)?
        } else {
            hs_authors
                .iter()
                .map(|a| Ok((*a, hs::recompute_author(&tx, room, a)?)))
                .collect::<AppResult<_>>()?
        };
        let changes: Vec<(MemberId, u32)> = replayed
            .iter()
            .filter(|(_, r)| r.changed)
            .map(|(a, r)| (*a, r.stacks))
            .collect();
        if !changes.is_empty() {
            notes.push(CoreEvent::StacksChanged {
                room_id: *room,
                changes,
            });
        }
        for (author, id) in &new_hs {
            let Some((_, r)) = replayed.iter().find(|(a, _)| a == author) else {
                continue;
            };
            if r.valid
                .iter()
                .any(|h| h.event_id == *id && hs::is_live_sound(is_live, h, now_ms))
            {
                notes.push(CoreEvent::HeartsteelLive {
                    room_id: *room,
                    member_id: *author,
                    new_stacks: r.stacks,
                    event_id: *id,
                });
            }
        }

        let row = rooms::get(&tx, room)?;
        for s in &new_items {
            if let Some(item) = chat::to_item(s, &row.owner_tag, row.dissolve_ts) {
                notes.push(CoreEvent::TimelineItemAdded {
                    room_id: *room,
                    item,
                    is_live,
                });
            }
        }
        tx.commit()?;
        Ok(notes)
    }

    /// 房间里本机持有的全部事件（按入库顺序）。M1-3 的补齐从这里取数据；测试里用它模拟把事件交给另一个人。
    /// 同伴的 Have 覆盖到的本人事件标记为已发送（架构 7.1），返回改了的事件 ID。
    pub fn mark_delivered(&mut self, room: &RoomId, peer: &[HaveEntry]) -> AppResult<Vec<EventId>> {
        let me = self.view.member_id;
        let Some(e) = peer.iter().find(|e| e.author == short_id(&me)) else {
            return Ok(Vec::new());
        };
        let tx = self.conn.transaction()?;
        let mut ids = events::mark_sent(&tx, room, &me, Stream::State, e.state_max)?;
        ids.extend(events::mark_sent(&tx, room, &me, Stream::Chat, e.chat_max)?);
        tx.commit()?;
        Ok(ids)
    }

    /// 本人还没被确认的事件（节点启动时排进重发计划）。
    pub fn pending_events(&self, room: &RoomId) -> AppResult<Vec<SignedEvent>> {
        Ok(events::pending_own(&self.conn, room, &self.view.member_id)?
            .into_iter()
            .map(|s| s.event)
            .collect())
    }

    pub fn room_events(&self, room: &RoomId) -> AppResult<Vec<SignedEvent>> {
        Ok(events::query_room(&self.conn, room)?
            .into_iter()
            .map(|s| s.event)
            .collect())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ingest_tests;

#[cfg(test)]
mod send_tests;

#[cfg(test)]
mod timeline_tests;

#[cfg(test)]
mod restart_tests;

#[cfg(test)]
mod sync_tests;
