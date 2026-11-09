//! 队伍面板、投票、文字的运行时（2.0 架构第 9 节）。窗口和投票流程移植自 lolPing `src/main/panelWindows.ts`、
//! `roomVotes.ts`、`textWindow.ts`（MIT，© 2026 Danrui Meng）。
//!
//! 实现流程：
//! 1. 一个专门的线程拥有投票和三个小窗口（队伍面板 roster、投票 vote、输入框 text），别处只往它的通道里发消息。
//!    每秒检查一次投票结果，每 3 秒重发一次自己的回答（实时帧可能丢）；结果出来后投票窗口再留 3 秒。
//!    窗口第一次用到时才建，之后藏起来复用。命令要的投票状态放在共享的 `Mutex` 里，命令不等这个线程。
//! 2. 队伍面板：按住面板键时在鼠标所在的显示器中间显示，松开收起。面板和投票窗口能点但不拿焦点，
//!    游戏或正在打字的程序不丢键盘。面板页面每次显示时自己取一次队伍信息（team_view）。
//! 3. 投票：发起时定下投票人（本人加上在线、没关投票的队友，`team::voters`），发一帧 Vote；收到队友的 Vote 同样开一个。
//!    回答发 Ballot 帧。规则在 `votes`。踢人通过时，被踢的人自己的程序退出队伍：队伍记录里没有「踢人」，靠被踢的人配合。
//! 4. 聊天框（3.0-M4，原来的输入框）：文字快捷键打开、再按一次收起，放在鼠标所在显示器的左下角（要拿焦点才能打字）；
//!    上面是历史记录（页面自己取 `history_list`），下面是输入框。记下按键时鼠标的位置；回车后清理、认指令（`team_text`），
//!    普通文字画在自己的透明窗口上、发给队伍、记进历史记录，落到队友同号显示器的同一比例处。发完不关，可以接着说。

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};
use tokio::sync::oneshot;

use crate::api::AppState;
use crate::crypto::{self, MemberId};
use crate::node::now_ms;
use crate::overlay::Msg;
use crate::overlay::displays::{self, Display};
use crate::protocol::live::{LiveBody, to_ratio};
use crate::team_text::{self, Input};
use crate::votes::{Open, Outcome, Tally, VoteKind};

const CHECK: Duration = Duration::from_secs(1);
const BALLOT_REPEAT: Duration = Duration::from_secs(3);
/// 结果出来后投票窗口还留多久。
const RESULT_SHOW: Duration = Duration::from_secs(3);
/// 窗口大小（CSS 像素）。
const ROSTER_SIZE: (f64, f64) = (720.0, 470.0);
const VOTE_SIZE: (f64, f64) = (260.0, 136.0);
const READY_SIZE: (f64, f64) = (400.0, 230.0);
/// 聊天框的大小，和离显示器左边、下边的距离（CSS 像素）。
const CHAT_SIZE: (f64, f64) = (420.0, 520.0);
const CHAT_MARGIN: (f64, f64) = (24.0, 140.0);

pub enum PanelMsg {
    /// 面板键按下或松开。
    Show(bool),
    /// 文字快捷键：打开输入框。
    TextRequested,
    TextSubmit(String, oneshot::Sender<&'static str>),
    TextCancel,
    VoteStart {
        kind: VoteKind,
        target: Option<MemberId>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Answer(bool),
    RemoteVote {
        from: MemberId,
        id: u32,
        kind: VoteKind,
        target: Option<MemberId>,
    },
    RemoteBallot {
        from: MemberId,
        id: u32,
        yes: bool,
    },
    /// 队员名字；不在名单里的投票人算走了。
    Names(Vec<(MemberId, String)>),
    /// 队员退出程序或离开队伍。
    Gone(MemberId),
    /// 进了队伍或不在队伍里了。
    InTeam(bool),
    /// 正在说话的队员（队伍面板里亮起）。
    Speaking(Vec<MemberId>),
}

/// 一个投票人在窗口上的样子。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoterDto {
    pub name: String,
    /// yes、no、none。
    pub answer: &'static str,
    pub me: bool,
}

/// 投票窗口和面板要画的投票。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteDto {
    pub kind: VoteKind,
    pub title: String,
    pub voters: Vec<VoterDto>,
    /// 结束时间（Unix 毫秒）和总时长，页面画倒计时用。
    pub ends_at_ms: u64,
    pub duration_ms: u64,
    pub can_answer: bool,
    /// 我的回答。
    pub mine: Option<bool>,
    /// 结果出来了：那一句结果。
    pub result: Option<String>,
}

#[derive(Clone)]
pub struct Panel {
    tx: Sender<PanelMsg>,
    vote: Arc<Mutex<Option<VoteDto>>>,
}

impl Panel {
    pub fn start(app: &AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();
        let vote = Arc::new(Mutex::new(None));
        let runner = Runner {
            app: app.clone(),
            shared: vote.clone(),
            tally: Tally::default(),
            me: None,
            names: HashMap::new(),
            in_team: false,
            result_until: None,
            last_ballot: Instant::now(),
            text_at: None,
            roster: None,
            speaking: Vec::new(),
            vote: None,
            text: None,
        };
        std::thread::Builder::new()
            .name("panel".into())
            .spawn(move || runner.run(rx))
            .expect("能开线程");
        Self { tx, vote }
    }

    pub fn send(&self, m: PanelMsg) {
        let _ = self.tx.send(m);
    }
}

struct Runner {
    app: AppHandle,
    shared: Arc<Mutex<Option<VoteDto>>>,
    tally: Tally,
    me: Option<MemberId>,
    names: HashMap<MemberId, String>,
    in_team: bool,
    /// 结果出来后投票窗口显示到这个时候。
    result_until: Option<Instant>,
    last_ballot: Instant,
    /// 按文字快捷键时鼠标在第几号显示器、0–65535 的比例位置。
    text_at: Option<(u8, u16, u16)>,
    roster: Option<WebviewWindow>,
    /// 正在说话的队员（公钥的十六进制），队伍面板里亮起。
    speaking: Vec<String>,
    vote: Option<WebviewWindow>,
    text: Option<WebviewWindow>,
}

impl Runner {
    fn run(mut self, rx: Receiver<PanelMsg>) {
        let mut next_check = Instant::now() + CHECK;
        loop {
            match rx.recv_timeout(next_check.saturating_duration_since(Instant::now())) {
                Ok(m) => self.handle(m),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if Instant::now() >= next_check {
                next_check = Instant::now() + CHECK;
                self.tick();
            }
        }
    }

    fn state(&self) -> Option<State<'_, AppState>> {
        self.app.try_state::<AppState>()
    }

    fn me(&mut self) -> Option<MemberId> {
        if self.me.is_none() {
            let node = self.state()?.node.clone();
            self.me =
                tauri::async_runtime::block_on(node.call(|core, _| core.identity().member_id));
        }
        self.me
    }

    fn settings(&self) -> Option<crate::settings2::Settings> {
        Some(
            self.state()?
                .settings
                .lock()
                .expect("设置锁没有中毒")
                .clone(),
        )
    }

    fn handle(&mut self, m: PanelMsg) {
        match m {
            PanelMsg::Show(on) => self.show_roster(on),
            PanelMsg::TextRequested => {
                if self.text_at.is_some() {
                    self.close_text();
                } else {
                    self.open_text();
                }
            }
            PanelMsg::TextSubmit(raw, reply) => {
                let _ = reply.send(self.submit_text(&raw));
            }
            PanelMsg::TextCancel => self.close_text(),
            PanelMsg::VoteStart {
                kind,
                target,
                reply,
            } => {
                let _ = reply.send(self.start_vote(kind, target));
            }
            PanelMsg::Answer(yes) => self.answer(yes),
            PanelMsg::RemoteVote {
                from,
                id,
                kind,
                target,
            } => self.remote_vote(from, id, kind, target),
            PanelMsg::RemoteBallot { from, id, yes } => {
                if self.tally.ballot(from, id, yes) {
                    self.settle();
                    self.push();
                }
            }
            PanelMsg::Names(list) => {
                self.names = list.into_iter().collect();
                let gone: Vec<MemberId> = self
                    .tally
                    .view()
                    .map(|o| {
                        o.voters
                            .iter()
                            .filter(|v| !self.names.contains_key(*v))
                            .copied()
                            .collect()
                    })
                    .unwrap_or_default();
                for v in &gone {
                    self.tally.left(v);
                }
                if !gone.is_empty() {
                    self.settle();
                    self.push();
                }
            }
            PanelMsg::Gone(m) => {
                self.tally.left(&m);
                self.settle();
                self.push();
            }
            PanelMsg::Speaking(list) => {
                self.speaking = list.iter().map(|m| crypto::hex(m)).collect();
                if let Some(w) = &self.roster {
                    let _ = w.emit_to(w.label(), "roster:speaking", &self.speaking);
                }
            }
            PanelMsg::InTeam(on) => {
                self.in_team = on;
                if !on {
                    self.tally.clear();
                    self.result_until = None;
                    self.publish(None);
                    self.text_at = None;
                    for w in [&self.roster, &self.vote, &self.text].into_iter().flatten() {
                        let _ = w.hide();
                    }
                }
            }
        }
    }

    fn tick(&mut self) {
        self.settle();
        if let Some(o) = self.tally.view()
            && self.last_ballot.elapsed() >= BALLOT_REPEAT
            && let Some(me) = self.me
            && let Some(yes) = o.ballot_of(&me)
        {
            let id = o.id;
            self.last_ballot = Instant::now();
            self.send_live(LiveBody::Ballot { id, yes });
        }
        if self.result_until.is_some_and(|t| Instant::now() >= t) {
            self.result_until = None;
            self.publish(None);
            if let Some(w) = &self.vote {
                let _ = w.hide();
            }
        }
        // 开着的投票每秒刷新一次（倒计时之外的变化都会马上推，这里只是兜底）。
        if self.tally.view().is_some() {
            self.push();
        }
    }

    fn send_live(&self, body: LiveBody) {
        if let Some(state) = self.state() {
            state.node.send_live(body);
        }
    }

    fn name(&self, m: &MemberId) -> String {
        if Some(*m) == self.me {
            return "我".into();
        }
        self.names.get(m).cloned().unwrap_or_else(|| "队友".into())
    }

    // ---------- 投票 ----------

    fn start_vote(&mut self, kind: VoteKind, target: Option<MemberId>) -> Result<(), String> {
        let s = self.settings().ok_or("程序还没启动好")?;
        if !s.team_votes {
            return Err("投票已经关掉了，在「功能」页打开".into());
        }
        if !self.in_team {
            return Err("不在队伍里".into());
        }
        if self.tally.view().is_some() || self.result_until.is_some() {
            return Err("已经有一个投票在进行".into());
        }
        let me = self.me().ok_or("读不到自己的身份")?;
        if kind == VoteKind::Kick {
            match target {
                Some(t) if t == me => return Err("不能踢自己".into()),
                Some(t) if self.names.contains_key(&t) => {}
                _ => return Err("队伍里没有这个人".into()),
            }
        }
        let id = crypto::random_bytes::<4>()
            .map(u32::from_le_bytes)
            .map_err(|e| e.to_string())?;
        let voters = crate::team::voters(&self.app, me);
        self.tally.start(id, kind, me, target, &voters, now_ms());
        self.send_live(LiveBody::Vote {
            id,
            kind: kind.code(),
            target,
        });
        self.last_ballot = Instant::now();
        self.show_vote(kind);
        self.settle();
        self.push();
        Ok(())
    }

    fn remote_vote(&mut self, from: MemberId, id: u32, kind: VoteKind, target: Option<MemberId>) {
        if !self.in_team || self.tally.view().is_some() || self.result_until.is_some() {
            return;
        }
        if !self.settings().is_some_and(|s| s.team_votes) {
            return;
        }
        let Some(me) = self.me() else { return };
        if from == me || !self.names.contains_key(&from) {
            return;
        }
        if kind == VoteKind::Kick {
            let known = target.is_some_and(|t| t == me || self.names.contains_key(&t));
            if !known || target == Some(from) {
                return;
            }
        }
        let voters = crate::team::voters(&self.app, me);
        self.tally.start(id, kind, from, target, &voters, now_ms());
        self.show_vote(kind);
        self.settle();
        self.push();
    }

    fn answer(&mut self, yes: bool) {
        let Some(me) = self.me() else { return };
        let Some(id) = self.tally.view().map(|o| o.id) else {
            return;
        };
        if self.tally.ballot(me, id, yes) {
            self.last_ballot = Instant::now();
            self.send_live(LiveBody::Ballot { id, yes });
            self.settle();
            self.push();
        }
    }

    /// 结果定了：投票窗口画最后一次（带结果），3 秒后收起；结果用提示说一声；踢的是我就退出队伍。
    fn settle(&mut self) {
        let last = self.tally.view().cloned();
        let Some(o) = self.tally.check(now_ms()) else {
            return;
        };
        let line = self.result_line(&o);
        if let Some(open) = last {
            let mut dto = self.dto(&open);
            dto.can_answer = false;
            dto.result = Some(line.clone());
            self.publish(Some(dto));
        }
        self.result_until = Some(Instant::now() + RESULT_SHOW);
        if let Some(state) = self.state() {
            state.overlays.send(Msg::Toast {
                title: line,
                body: String::new(),
            });
        }
        if o.passed && o.kind == VoteKind::Kick && o.target.is_some() && o.target == self.me {
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::team::team_leave(app.clone()).await;
                if let Some(state) = app.try_state::<AppState>() {
                    state.overlays.send(Msg::Toast {
                        title: "队友投票把你请出了队伍".into(),
                        body: "可以用队伍码再进来".into(),
                    });
                }
            });
        }
    }

    fn result_line(&self, o: &Outcome) -> String {
        match o.kind {
            VoteKind::Ff => format!(
                "投降投票{}（{}/{}）",
                if o.passed { "通过" } else { "没通过" },
                o.yes,
                o.voters
            ),
            VoteKind::Ready if o.passed => "大家都准备好了".into(),
            VoteKind::Ready => {
                let names: Vec<String> = o
                    .declined
                    .iter()
                    .chain(&o.missing)
                    .map(|m| self.name(m))
                    .collect();
                format!("还没准备好：{}", names.join("、"))
            }
            VoteKind::Kick => format!(
                "踢出 {}：{}（{}/{}）",
                o.target.map(|t| self.name(&t)).unwrap_or_default(),
                if o.passed { "通过" } else { "没通过" },
                o.yes,
                o.voters
            ),
        }
    }

    fn dto(&self, o: &Open) -> VoteDto {
        let mine = self.me.and_then(|me| o.ballot_of(&me));
        let voter = self.me.is_some_and(|me| o.voters.contains(&me));
        VoteDto {
            kind: o.kind,
            title: match o.kind {
                VoteKind::Ff => "投降投票".into(),
                VoteKind::Ready => "准备确认".into(),
                VoteKind::Kick => format!(
                    "踢出 {}？",
                    o.target.map(|t| self.name(&t)).unwrap_or_default()
                ),
            },
            voters: o
                .voters
                .iter()
                .map(|v| VoterDto {
                    name: self.name(v),
                    answer: match o.ballot_of(v) {
                        Some(true) => "yes",
                        Some(false) => "no",
                        None => "none",
                    },
                    me: Some(*v) == self.me,
                })
                .collect(),
            ends_at_ms: o.ends_ms,
            duration_ms: o.kind.duration_ms(),
            can_answer: voter && mine.is_none() && now_ms() < o.ends_ms,
            mine,
            result: None,
        }
    }

    /// 开着的投票交给窗口。
    fn push(&self) {
        if let Some(o) = self.tally.view() {
            self.publish(Some(self.dto(o)));
        }
    }

    fn publish(&self, dto: Option<VoteDto>) {
        *self.shared.lock().expect("投票锁没有中毒") = dto.clone();
        for w in [&self.roster, &self.vote].into_iter().flatten() {
            let _ = w.emit_to(w.label(), "vote:state", &dto);
        }
    }

    // ---------- 窗口 ----------

    fn window(&mut self, which: Which) -> Option<WebviewWindow> {
        let slot = match which {
            Which::Roster => &mut self.roster,
            Which::Vote => &mut self.vote,
            Which::Text => &mut self.text,
        };
        if let Some(w) = slot {
            return Some(w.clone());
        }
        let (label, page) = match which {
            Which::Roster => ("roster", "roster.html"),
            Which::Vote => ("vote", "vote.html"),
            Which::Text => ("text", "text.html"),
        };
        // 面板和投票：能点、不拿焦点；输入框要打字，得拿焦点。
        let built = crate::overlay::floating_window(
            &self.app,
            label,
            page,
            "lolGang",
            which == Which::Text,
        );
        match built {
            Ok(w) => {
                *slot = Some(w.clone());
                Some(w)
            }
            Err(e) => {
                eprintln!("{label} 窗口建不起来：{e}");
                None
            }
        }
    }

    fn show_roster(&mut self, on: bool) {
        if !on {
            if let Some(w) = &self.roster {
                let _ = w.hide();
            }
            return;
        }
        if !self.in_team || !self.settings().is_some_and(|s| s.team_panel) {
            return;
        }
        let Some(d) = cursor_display(&self.app) else {
            return;
        };
        let Some(w) = self.window(Which::Roster) else {
            return;
        };
        place(&w, centred(&d, ROSTER_SIZE));
        reveal(&w);
        let dto = self.shared.lock().expect("投票锁没有中毒").clone();
        let _ = w.emit_to(w.label(), "roster:show", &dto);
        let _ = w.emit_to(w.label(), "roster:speaking", &self.speaking);
    }

    /// 准备确认在主显示器中间；投降和踢人在右边、约 40% 高的地方（同英雄联盟）。
    fn show_vote(&mut self, kind: VoteKind) {
        let Some(list) = crate::overlay::current_displays(&self.app) else {
            return;
        };
        let Some(d) = list.first().cloned() else {
            return;
        };
        let Some(w) = self.window(Which::Vote) else {
            return;
        };
        let rect = if kind == VoteKind::Ready {
            centred(&d, READY_SIZE)
        } else {
            let (w_px, h_px) = physical(&d, VOTE_SIZE);
            let gap = (16.0 * d.scale) as i32;
            (
                d.rect.x + d.rect.width - w_px - gap,
                d.rect.y + (f64::from(d.rect.height) * 0.4) as i32,
                w_px,
                h_px,
            )
        };
        place(&w, rect);
        reveal(&w);
    }

    fn open_text(&mut self) {
        if !self.in_team || !self.settings().is_some_and(|s| s.team_text) {
            return;
        }
        let Some(list) = crate::overlay::current_displays(&self.app) else {
            return;
        };
        // 启动后鼠标还没动过时钩子不知道它在哪：放在主显示器中间。
        let (cx, cy) = crate::input::cursor().unwrap_or_else(|| {
            list.first().map_or((0, 0), |d| {
                (d.rect.x + d.rect.width / 2, d.rect.y + d.rect.height / 2)
            })
        });
        let Some(i) = displays::display_at(&list, cx, cy) else {
            return;
        };
        let d = &list[i];
        self.text_at = Some((
            u8::try_from(i + 1).unwrap_or(u8::MAX),
            to_ratio(f64::from(cx - d.rect.x), f64::from(d.rect.width)),
            to_ratio(f64::from(cy - d.rect.y), f64::from(d.rect.height)),
        ));
        // 聊天框在这块显示器的左下角（和英雄联盟的聊天框一个位置），放不下就挪进显示器里。
        let (w_px, h_px) = physical(d, CHAT_SIZE);
        let (mx, my) = physical(d, CHAT_MARGIN);
        let x = (d.rect.x + mx)
            .min(d.rect.x + d.rect.width - w_px)
            .max(d.rect.x);
        let y = (d.rect.y + d.rect.height - h_px - my).max(d.rect.y);
        let Some(w) = self.window(Which::Text) else {
            return;
        };
        place(&w, (x, y, w_px, h_px));
        reveal(&w);
        let _ = w.set_focus();
        let _ = w.emit_to(w.label(), "text:open", ());
    }

    fn close_text(&mut self) {
        self.text_at = None;
        if let Some(w) = &self.text {
            let _ = w.hide();
        }
        if let Some(state) = self.state() {
            state.voice.send(crate::voice::VoiceMsg::StopPlay);
        }
    }

    /// 输入框交来的一句话。返回 sent、empty、unknown、votesOff、busy。
    fn submit_text(&mut self, raw: &str) -> &'static str {
        let answer = match team_text::parse(raw) {
            Input::Empty => "empty",
            Input::Unknown => "unknown",
            Input::Text(text) => {
                if let (Some((display, x, y)), Some(state)) = (self.text_at, self.state()) {
                    state.history.add(crate::history::NewEntry {
                        member: None,
                        ts_ms: now_ms(),
                        kind: crate::history::Kind::Text,
                        text: text.clone(),
                        voice: None,
                    });
                    state.overlays.send(Msg::Text {
                        from: None,
                        display,
                        x,
                        y,
                        text: text.clone(),
                    });
                    state.node.send_live(LiveBody::Text {
                        display,
                        x,
                        y,
                        text,
                    });
                }
                "sent"
            }
            Input::NoFf => {
                let open_ff = self.tally.view().is_some_and(|o| o.kind == VoteKind::Ff);
                if !self.settings().is_some_and(|s| s.team_votes) {
                    "votesOff"
                } else if open_ff {
                    self.answer(false);
                    "sent"
                } else {
                    "unknown"
                }
            }
            Input::Ff | Input::Ready => {
                let kind = if team_text::parse(raw) == Input::Ff {
                    VoteKind::Ff
                } else {
                    VoteKind::Ready
                };
                match self.start_vote(kind, None) {
                    Ok(()) => "sent",
                    Err(_) if !self.settings().is_some_and(|s| s.team_votes) => "votesOff",
                    Err(_) => "busy",
                }
            }
        };
        // 发完不关，可以接着说；空着按回车就收起（同英雄联盟）
        if answer == "empty" {
            self.close_text();
        }
        answer
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Roster,
    Vote,
    Text,
}

fn physical(d: &Display, (w, h): (f64, f64)) -> (i32, i32) {
    ((w * d.scale).round() as i32, (h * d.scale).round() as i32)
}

fn centred(d: &Display, size: (f64, f64)) -> (i32, i32, i32, i32) {
    let (w, h) = physical(d, size);
    (
        d.rect.x + (d.rect.width - w) / 2,
        d.rect.y + (d.rect.height - h) / 2,
        w,
        h,
    )
}

fn cursor_display(app: &AppHandle) -> Option<Display> {
    let list = crate::overlay::current_displays(app)?;
    let (x, y) = crate::input::cursor().unwrap_or((0, 0));
    let i = displays::display_at(&list, x, y).unwrap_or(0);
    list.get(i).cloned()
}

fn place(w: &WebviewWindow, (x, y, width, height): (i32, i32, i32, i32)) {
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = w.set_size(PhysicalSize::new(width.max(1) as u32, height.max(1) as u32));
}

/// 显示并重新放到所有置顶窗口最上面（Windows 偶尔会把置顶窗口压下去；透明窗口也是置顶的）。
fn reveal(w: &WebviewWindow) {
    let _ = w.show();
    let _ = w.set_always_on_top(false);
    let _ = w.set_always_on_top(true);
}

// ---------- 命令 ----------

fn panel(app: &AppHandle) -> Result<Panel, String> {
    app.try_state::<AppState>()
        .map(|s| s.panel.clone())
        .ok_or_else(|| "程序还没启动好".into())
}

/// 投票窗口、面板页打开时取一次投票。
#[tauri::command]
pub fn vote_view(state: State<'_, AppState>) -> Option<VoteDto> {
    state.panel.vote.lock().expect("投票锁没有中毒").clone()
}

/// 发起投票：ff、ready，或 kick 加目标的公钥（十六进制）。
#[tauri::command]
pub async fn vote_start(
    app: AppHandle,
    kind: VoteKind,
    target: Option<String>,
) -> Result<(), String> {
    let target = match target {
        Some(hex) => Some(parse_member(&hex).ok_or("队员编号不对")?),
        None => None,
    };
    let (reply, rx) = oneshot::channel();
    panel(&app)?.send(PanelMsg::VoteStart {
        kind,
        target,
        reply,
    });
    rx.await.map_err(|_| "投票没有响应".to_string())?
}

#[tauri::command]
pub fn vote_answer(state: State<'_, AppState>, yes: bool) {
    state.panel.send(PanelMsg::Answer(yes));
}

/// 输入框回车：返回 sent、empty、unknown、votesOff、busy。
#[tauri::command]
pub async fn text_send(app: AppHandle, text: String) -> Result<&'static str, String> {
    let (reply, rx) = oneshot::channel();
    panel(&app)?.send(PanelMsg::TextSubmit(text, reply));
    rx.await.map_err(|_| "输入框没有响应".to_string())
}

#[tauri::command]
pub fn text_cancel(state: State<'_, AppState>) {
    state.panel.send(PanelMsg::TextCancel);
}

fn parse_member(hex: &str) -> Option<MemberId> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_ids_parse_from_hex() {
        let id = [0xabu8; 32];
        assert_eq!(parse_member(&crypto::hex(&id)), Some(id));
        assert_eq!(parse_member("ab"), None);
        assert_eq!(parse_member(&"zz".repeat(32)), None);
    }

    #[test]
    fn windows_are_centred_and_scaled() {
        let d = Display {
            key: "a".into(),
            primary: true,
            rect: displays::Rect {
                x: 100,
                y: 0,
                width: 2560,
                height: 1600,
            },
            scale: 1.5,
        };
        assert_eq!(physical(&d, ROSTER_SIZE), (1080, 705));
        assert_eq!(centred(&d, ROSTER_SIZE), (100 + 740, 447, 1080, 705));
    }
}
