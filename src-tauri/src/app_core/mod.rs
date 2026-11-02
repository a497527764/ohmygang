//! 本机核心：接口文档第 1 节的 `core` 模块（Rust 标准库已有 `core`，为避免重名叫 `app_core`）。
//!
//! 实现流程：
//! 1. 启动：从凭据管理器取本机主密钥，读出或生成本机身份。
//! 2. 每个对外操作开一个数据库事务，调用各业务模块，全部成功才提交；
//!    返回本次新写的事件（M1-3 交给 sync 发出去）和给界面的通知。
//! 3. 它是唯一知道所有模块的地方，业务模块之间不互相回调。

use crate::crypto::{Identity, MemberId, RoomId};
use crate::error::AppResult;
use crate::identity::{self, IdentityView};
use crate::keystore::KeyStore;
use crate::protocol::body::{self, ProfileBody, Rule};
use crate::protocol::event::{SignedEvent, Stream, kind};
use crate::rooms::service as rooms;
use crate::storage::events;
use crate::storage::{Connection, rooms as room_store};

pub use crate::error::AppError;
pub use crate::rooms::service::{MemberView, RoomSummary, RuleParams};
pub use crate::storage::rooms::RoomStatus;

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
