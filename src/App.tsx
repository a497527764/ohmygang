// 最小可用界面（M1-3）：左栏房间列表与建房/进房，中栏聊天，右栏成员与层数。
// 数据都从 api.ts 取；收到 hs://event 时只刷新受影响的部分。排行榜、设置页、美化在 M1-5。
// M1-4 加了成员的联系路径（局域网/中继/离线）和中继连接数，这两样没有推送，每 5 秒刷新一次。
import { useCallback, useEffect, useRef, useState } from "react";
import heartsteel from "./assets/heartsteel.png";
import heartsteelSound from "./assets/heartsteel.mp3";
import {
  api,
  mergeMessages,
  type HsEvent,
  type Identity,
  type Member,
  type MemberPath,
  type Message,
  type RelayHealth,
  type Room,
} from "./api";
import "./app.css";

const PAGE = 100;

const PATH_NAMES: Record<MemberPath, string> = { lan: "局域网", relay: "中继", offline: "离线" };

/// undefined 表示还没查到。冷却中的中继也是连着的，算已连接。
function relayText(relays: RelayHealth[] | null | undefined): string {
  if (relays === undefined) return "";
  if (relays === null) return "未联网，只走局域网";
  const connected = relays.filter((r) => r.state === "ok" || r.state === "cooling").length;
  return `中继 已连接 ${connected}/${relays.length}`;
}

// 心之钢音效（内置）。只在收到 Rust 的实时叠层通知时播放：补齐回来的历史叠层不会发这个通知（架构 5.6）。
// 每次新建一个播放器，两人几乎同时叠层时两声可以重叠，不会互相打断。
function playHeartsteel() {
  new Audio(heartsteelSound).play().catch((e) => console.warn("心之钢音效播放失败", e));
}

function displayName(id: string, members: Member[]): string {
  const m = members.find((x) => x.memberId === id);
  return m?.nickname || m?.fingerprint.slice(0, 6) || id.slice(0, 6);
}

function noticeText(m: Message, members: Member[]): string {
  const who = displayName(m.authorId, members);
  switch (m.notice) {
    case "join":
      return `${who} 加入了群聊`;
    case "leave":
      return `${who} 退出了群聊`;
    case "rename":
      return `${who} 把群名改为「${m.title ?? ""}」`;
    case "config":
      return `${who} 修改了心之钢规则`;
    case "dissolve":
      return `${who} 解散了群聊`;
    default:
      return "";
  }
}

function App() {
  const [me, setMe] = useState<Identity | null>(null);
  const [rooms, setRooms] = useState<Room[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [members, setMembers] = useState<Member[]>([]);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [relays, setRelays] = useState<RelayHealth[] | null | undefined>(undefined);
  const activeRef = useRef<string | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  const fail = (e: unknown) => setError(String(e));

  const loadRooms = useCallback(() => api.rooms().then(setRooms).catch(fail), []);
  const loadMembers = useCallback((roomId: string) => api.members(roomId).then(setMembers).catch(fail), []);

  const openRoom = useCallback(
    async (roomId: string) => {
      activeRef.current = roomId;
      setActive(roomId);
      setMessages([]);
      try {
        setMessages(await api.messages(roomId, null, PAGE));
        setMembers(await api.members(roomId));
        await api.markRead(roomId);
        await loadRooms();
      } catch (e) {
        fail(e);
      }
    },
    [loadRooms],
  );

  useEffect(() => {
    api.identity().then(setMe).catch(fail);
    loadRooms();
    const off = api.onEvent((e: HsEvent) => {
      const current = activeRef.current;
      if (e.type === "live") playHeartsteel(); // 任何房间的实时叠层都响
      if (e.roomId === current) {
        if (e.type === "timeline") {
          setMessages((list) => mergeMessages(list, [e.message]));
          api.markRead(e.roomId).then(loadRooms).catch(fail);
        }
        if (e.type === "stacks" || e.type === "room" || e.type === "status") loadMembers(e.roomId);
        if (e.type === "live") setNotice(`心之钢 +1！`);
      }
      if (e.type === "mismatch") setError("有成员的版本和你不同，请双方更新到同一版本");
      if (e.type !== "timeline" || e.roomId !== current) loadRooms();
    });
    return () => {
      off.then((f) => f());
    };
  }, [loadRooms, loadMembers]);

  useEffect(() => {
    const refresh = () => {
      api.relayStatus().then(setRelays).catch(fail);
      if (active) loadMembers(active);
    };
    refresh();
    const t = setInterval(refresh, 5000);
    return () => clearInterval(t);
  }, [active, loadMembers]);

  // 用花括号包住：新版 WebView2 的 scrollIntoView 会返回 Promise，直接返回会被 React 当成清理函数而崩溃
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [messages.length]);
  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(() => setNotice(null), 2000);
    return () => clearTimeout(t);
  }, [notice]);

  const room = rooms.find((r) => r.roomId === active) ?? null;

  async function createRoom() {
    const title = window.prompt("群名称");
    if (!title) return;
    try {
      const created = await api.createRoom(title);
      await loadRooms();
      await openRoom(created.room.roomId);
      await navigator.clipboard.writeText(created.code).catch(() => {});
      setNotice("建房成功，房间码已复制，发给群友即可加入");
    } catch (e) {
      fail(e);
    }
  }

  async function joinRoom() {
    const code = window.prompt("粘贴房间码");
    if (!code) return;
    try {
      const joined = await api.joinRoom(code.trim());
      await loadRooms();
      await openRoom(joined.roomId);
    } catch (e) {
      fail(e);
    }
  }

  async function copyCode() {
    if (!active) return;
    try {
      await navigator.clipboard.writeText(await api.roomCode(active));
      setNotice("房间码已复制");
    } catch (e) {
      fail(e);
    }
  }

  async function rename() {
    const nickname = window.prompt("你的昵称（最多 32 个字）", me?.nickname ?? "");
    if (nickname === null) return;
    try {
      setMe(await api.setNickname(nickname));
      if (active) loadMembers(active);
    } catch (e) {
      fail(e);
    }
  }

  async function send() {
    const text = draft.trim();
    if (!active || !text) return;
    try {
      const sent = await api.send(active, text);
      setDraft("");
      setMessages((list) => mergeMessages(list, [sent]));
      loadMembers(active);
    } catch (e) {
      fail(e);
    }
  }

  async function loadOlder() {
    if (!active || messages.length === 0) return;
    const first = messages[0];
    try {
      const older = await api.messages(active, { displayTs: first.displayTs, eventId: first.eventId }, PAGE);
      setMessages((list) => mergeMessages(list, older));
    } catch (e) {
      fail(e);
    }
  }

  const canSend = room?.status === "active";
  const hint =
    room?.status === "joining"
      ? "等待成员上线：和任意一位在线成员同步后才能发言"
      : room?.status === "dissolved"
        ? "群已解散，只能查看历史"
        : "";

  return (
    <div className="app">
      <header className="top">
        <img src={heartsteel} alt="心之钢" className="logo" />
        <span className="brand">心之钢群聊</span>
        <button className="link" onClick={rename} title="修改昵称">
          {me ? me.nickname || `未设置昵称（${me.fingerprint.slice(0, 6)}）` : "…"}
        </button>
      </header>

      {error && (
        <div className="banner error" onClick={() => setError(null)}>
          {error}（点击关闭）
        </div>
      )}
      {notice && <div className="banner">{notice}</div>}

      <div className="columns">
        <aside className="rooms">
          <div className="actions">
            <button onClick={createRoom}>建房</button>
            <button onClick={joinRoom}>进房</button>
          </div>
          {rooms.length === 0 && <p className="muted">还没有房间</p>}
          {rooms.map((r) => (
            <div
              key={r.roomId}
              className={`room ${r.roomId === active ? "selected" : ""}`}
              onClick={() => openRoom(r.roomId)}
            >
              <span className="title">{r.title ?? "（同步中）"}</span>
              {r.status === "joining" && <span className="tag">等待</span>}
              {r.status === "dissolved" && <span className="tag">已解散</span>}
              {r.unread > 0 && <span className="unread">{r.unread}</span>}
            </div>
          ))}
        </aside>

        <main className="chat">
          {room ? (
            <>
              <div className="chat-head">
                <span>{room.title ?? "（同步中）"}</span>
                <span className="net">{relayText(relays)}</span>
              </div>
              <div className="messages">
                {messages.length >= PAGE && (
                  <button className="link older" onClick={loadOlder}>
                    加载更早的消息
                  </button>
                )}
                {messages.map((m) =>
                  m.kind === "notice" ? (
                    <div key={m.eventId} className="notice">
                      {noticeText(m, members)}
                    </div>
                  ) : (
                    <div key={m.eventId} className={`msg ${m.authorId === me?.memberId ? "mine" : ""}`}>
                      <div className="meta">
                        {displayName(m.authorId, members)} · {new Date(m.displayTs).toLocaleTimeString()}
                        {m.delivery === "pending" && " · 发送中"}
                      </div>
                      <div className="text">{m.text}</div>
                    </div>
                  ),
                )}
                <div ref={bottomRef} />
              </div>
              {hint && <div className="hint">{hint}</div>}
              <form
                className="composer"
                onSubmit={(e) => {
                  e.preventDefault();
                  send();
                }}
              >
                <input
                  value={draft}
                  onChange={(e) => setDraft(e.currentTarget.value)}
                  placeholder={canSend ? "说点什么，5 秒内连发 3 条叠一层心之钢" : ""}
                  disabled={!canSend}
                  maxLength={300}
                />
                <button type="submit" disabled={!canSend || !draft.trim()}>
                  发送
                </button>
              </form>
            </>
          ) : (
            <div className="empty">
              <img src={heartsteel} alt="" width={120} height={120} />
              <p>建一个房间，或者粘贴房间码进房</p>
            </div>
          )}
        </main>

        <aside className="members">
          {room && (
            <>
              <button onClick={copyCode}>复制房间码</button>
              <div className="section">成员</div>
              {members
                .filter((m) => m.inRoom)
                .map((m) => (
                  <div key={m.memberId} className="member">
                    <span className="name">
                      {m.nickname || m.fingerprint.slice(0, 6)}
                      {m.isOwner && <span className="tag">房主</span>}
                      {m.memberId !== me?.memberId && (
                        <span className={`tag path-${m.path}`}>{PATH_NAMES[m.path]}</span>
                      )}
                    </span>
                    <span className="stacks" title="心之钢层数">
                      <img src={heartsteel} alt="" />
                      {m.stacks}
                    </span>
                  </div>
                ))}
            </>
          )}
        </aside>
      </div>
    </div>
  );
}

export default App;
