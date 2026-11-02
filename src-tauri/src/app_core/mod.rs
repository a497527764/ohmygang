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

use crate::chat::{self, TimelineItem};
use crate::crypto::{EventId, Identity, MemberId, RoomId};
use crate::error::AppResult;
use crate::heartsteel::service::{self as hs, Replayed};
use crate::identity::{self, IdentityView};
use crate::keystore::KeyStore;
use crate::protocol::body::{self, ProfileBody, Rule};
use crate::protocol::event::{SignedEvent, Stream, kind, verify_stateless};
use crate::rooms::service as rooms;
use crate::storage::events::{self, PutResult, StoredEvent};
use crate::storage::{Connection, rooms as room_store};

pub use crate::chat::ChatMessage;
pub use crate::error::AppError;
pub use crate::rooms::service::{MemberView, RoomSummary, RuleParams};
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
