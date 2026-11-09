//! 历史记录的运行时（3.0，计划 3.6）：信号、表情、文字、语音发生时记下来，聊天框打开时读出来。
//!
//! 实现流程：
//! 1. 一个专门的线程拥有自己的数据库连接（同一个 `gang2.db`，WAL 模式下和别的连接互不阻塞），
//!    别处只往通道里发消息，所以节点任务、透明窗口线程、语音线程记一条都不用等数据库。
//! 2. 记在「现在所在的队伍」下面：队伍有变化时（`team::refresh_names`）告诉这里现在是哪个队伍，
//!    别的队伍的记录当场删掉（`storage::history::keep_only`）——退出、被踢、解散（包括不在线时被解散、上线后才同步到）都是这一条。
//! 3. 加一条：存库、修剪（每个队伍 2000 条、语音总共 100 MB），再推给聊天框（"history:added"），开着的聊天框直接加上这一条。
//! 4. 语音存成一串 Opus 小帧：每个前面两字节长度（小端），回放时拆开交给语音线程。

use std::sync::mpsc::{self, Receiver, Sender};

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::crypto::{MemberId, RoomId};
use crate::protocol::live::LiveBody;
pub use crate::storage::history::Kind;
use crate::storage::history::{self as store, Row};

/// 每个队伍最多留多少条。
pub const MAX_ROWS: usize = 2000;
/// 所有队伍的语音加起来最多这么多字节。
pub const MAX_VOICE_BYTES: u64 = 100 * 1024 * 1024;
/// 一个 Opus 小帧多长（毫秒）。
const FRAME_MS: u32 = 20;
/// 聊天框的窗口名（`panel.rs` 建的输入框窗口）。
const CHAT_WINDOW: &str = "text";

/// 要记的一条。`member` 为 None 表示是我自己。
pub struct NewEntry {
    pub member: Option<MemberId>,
    pub ts_ms: u64,
    pub kind: Kind,
    /// 文字的内容、信号 id、表情引用。
    pub text: String,
    /// 语音的 Opus 小帧（每个 20 毫秒）。
    pub voice: Option<Vec<Vec<u8>>>,
}

/// 聊天框里的一条。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDto {
    pub id: i64,
    /// 公钥的十六进制。
    pub member_id: String,
    pub me: bool,
    /// 名字的颜色，和信号标签一样。
    pub color: &'static str,
    pub ts: u64,
    /// text、voice、ping、emote。
    pub kind: &'static str,
    pub text: String,
    pub voice_ms: u32,
}

enum Msg {
    Room(Option<RoomId>),
    Add(NewEntry),
    List(Sender<Vec<EntryDto>>),
    Voice(i64, Sender<Option<Vec<Vec<u8>>>>),
}

#[derive(Clone)]
pub struct History {
    tx: Sender<Msg>,
}

impl History {
    pub fn start(app: &AppHandle, conn: Connection, me: MemberId) -> Self {
        let (tx, rx) = mpsc::channel();
        let runner = Runner {
            app: app.clone(),
            conn,
            me,
            room: None,
        };
        std::thread::Builder::new()
            .name("history".into())
            .spawn(move || runner.run(rx))
            .expect("能开线程");
        Self { tx }
    }

    /// 现在所在的队伍（加入中或已加入）；None 表示不在队伍里。别的队伍的记录会被删掉。
    pub fn set_room(&self, room: Option<RoomId>) {
        let _ = self.tx.send(Msg::Room(room));
    }

    pub fn add(&self, e: NewEntry) {
        let _ = self.tx.send(Msg::Add(e));
    }

    /// 自己放的信号或表情（别的实时帧不记）。
    pub fn own_live(&self, body: &LiveBody) {
        if let Some((kind, text)) = live_entry(body) {
            self.add(NewEntry {
                member: None,
                ts_ms: crate::node::now_ms(),
                kind,
                text,
                voice: None,
            });
        }
    }

    /// 队友放的信号或表情。
    pub fn remote_live(&self, from: MemberId, body: &LiveBody) {
        if let Some((kind, text)) = live_entry(body) {
            self.add(NewEntry {
                member: Some(from),
                ts_ms: crate::node::now_ms(),
                kind,
                text,
                voice: None,
            });
        }
    }

    /// 现在所在队伍的全部记录，从旧到新。
    pub fn list(&self) -> Vec<EntryDto> {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Msg::List(tx));
        rx.recv().unwrap_or_default()
    }

    /// 一条语音的 Opus 小帧。
    pub fn voice(&self, id: i64) -> Option<Vec<Vec<u8>>> {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Msg::Voice(id, tx));
        rx.recv().ok().flatten()
    }
}

fn live_entry(body: &LiveBody) -> Option<(Kind, String)> {
    match body {
        LiveBody::Ping { id, .. } => Some((Kind::Ping, id.clone())),
        LiveBody::Emote { emote, .. } => Some((Kind::Emote, emote.clone())),
        _ => None,
    }
}

struct Runner {
    app: AppHandle,
    conn: Connection,
    me: MemberId,
    /// 现在所在的队伍；还没听说过时是 None（这时不记）。
    room: Option<RoomId>,
}

impl Runner {
    fn run(mut self, rx: Receiver<Msg>) {
        while let Ok(m) = rx.recv() {
            match m {
                Msg::Room(room) => {
                    self.room = room;
                    if let Err(e) = store::keep_only(&self.conn, room.as_ref()) {
                        eprintln!("历史记录没删掉：{e}");
                    }
                }
                Msg::Add(e) => self.add(e),
                Msg::List(reply) => {
                    let rows = self
                        .room
                        .and_then(|r| store::list(&self.conn, &r).ok())
                        .unwrap_or_default();
                    let _ = reply.send(rows.iter().map(|r| self.dto(r)).collect());
                }
                Msg::Voice(id, reply) => {
                    let frames = store::voice(&self.conn, id)
                        .ok()
                        .flatten()
                        .map(|b| unpack(&b));
                    let _ = reply.send(frames);
                }
            }
        }
    }

    fn add(&mut self, e: NewEntry) {
        let Some(room) = self.room else { return };
        let member = e.member.unwrap_or(self.me);
        let voice = e
            .voice
            .map(|frames| (pack(&frames), frames.len() as u32 * FRAME_MS));
        let added = store::add(
            &self.conn,
            &room,
            &member,
            e.ts_ms,
            e.kind,
            &e.text,
            voice.as_ref().map(|(b, ms)| (b.as_slice(), *ms)),
        )
        .and_then(|id| store::trim(&self.conn, &room, MAX_ROWS, MAX_VOICE_BYTES).map(|()| id));
        match added {
            Ok(id) => {
                let dto = self.dto(&Row {
                    id,
                    member_id: member,
                    ts_ms: e.ts_ms,
                    kind: e.kind,
                    text: e.text,
                    voice_ms: voice.map_or(0, |(_, ms)| ms),
                });
                let _ = self.app.emit_to(CHAT_WINDOW, "history:added", dto);
            }
            Err(err) => eprintln!("历史记录没存上：{err}"),
        }
    }

    fn dto(&self, r: &Row) -> EntryDto {
        EntryDto {
            id: r.id,
            member_id: crate::team::hex(&r.member_id),
            me: r.member_id == self.me,
            color: crate::overlay::tag_color(&r.member_id),
            ts: r.ts_ms,
            kind: match r.kind {
                Kind::Text => "text",
                Kind::Voice => "voice",
                Kind::Ping => "ping",
                Kind::Emote => "emote",
            },
            text: r.text.clone(),
            voice_ms: r.voice_ms,
        }
    }
}

/// 一串小帧 → 每个前面两字节长度（小端）。
fn pack(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames.iter().map(|f| f.len() + 2).sum());
    for f in frames {
        let len = u16::try_from(f.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&f[..usize::from(len)]);
    }
    out
}

fn unpack(mut b: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while b.len() >= 2 {
        let len = usize::from(u16::from_le_bytes([b[0], b[1]]));
        let Some(frame) = b.get(2..2 + len) else {
            break;
        };
        out.push(frame.to_vec());
        b = &b[2 + len..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_frames_pack_and_unpack() {
        let frames = vec![vec![1, 2, 3], vec![], vec![9; 300]];
        assert_eq!(unpack(&pack(&frames)), frames);
        // 截断的数据：读到哪算哪，不出错
        let mut b = pack(&frames);
        b.truncate(b.len() - 10);
        assert_eq!(unpack(&b), frames[..2].to_vec());
    }

    #[test]
    fn only_pings_and_emotes_come_from_live_frames() {
        let ping = LiveBody::Ping {
            display: 1,
            x: 0,
            y: 0,
            id: "danger".into(),
        };
        assert_eq!(live_entry(&ping), Some((Kind::Ping, "danger".into())));
        let emote = LiveBody::Emote {
            display: 1,
            x: 0,
            y: 0,
            emote: "facepalm".into(),
            fx: 0,
        };
        assert_eq!(live_entry(&emote), Some((Kind::Emote, "facepalm".into())));
        assert_eq!(live_entry(&LiveBody::Bye), None);
    }
}
