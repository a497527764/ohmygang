// 群资料（需求 3.5，参考微信群资料）：右侧面板。成员网格（可搜索，头像左下角是在线小绿点）、心之钢排行榜、
// 免打扰和置顶、群名称、心之钢规则、房主、房间码、清空聊天记录，最下面是退出和解散。
// 排行榜和角标都来自 Rust 重放出的层数，所以一定一致；层数变化时由 refreshKey 触发重新拉取。
import { useEffect, useState } from "react";
import heartsteel from "../assets/heartsteel.png";
import { api, type Identity, type LeaderboardEntry, type Member, type Room, type RoomRule } from "../api";
import { displayName, PATH_NAMES } from "../names";
import { ruleText } from "../rule";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";
import type { Pulse } from "./ChatView";

interface Props {
  room: Room;
  me: Identity | null;
  members: Member[];
  refreshKey: number;
  pulse: Pulse | null;
  onClose: () => void;
  onCopyCode: () => void;
  onPrefs: (prefs: { muted?: boolean; pinned?: boolean }) => void;
  onRename: () => void;
  onEditRule: (rule: RoomRule) => void;
  onClearHistory: () => void;
  onLeave: () => void;
  onDissolve: () => void;
  onError: (e: unknown) => void;
}

export function GroupDrawer(p: Props) {
  const [query, setQuery] = useState("");
  const [board, setBoard] = useState<LeaderboardEntry[]>([]);
  const [rule, setRule] = useState<RoomRule | null>(null);
  const { roomId } = p.room;
  const { onError } = p;

  useEffect(() => {
    api.leaderboard(roomId).then(setBoard).catch(onError);
    api.roomRule(roomId).then(setRule).catch(onError);
  }, [roomId, p.refreshKey, onError]);

  // Esc 关闭（焦点不一定在面板里）
  const { onClose } = p;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector(".overlay")) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const q = query.trim().toLowerCase();
  const inRoom = p.members.filter((m) => m.inRoom);
  const shown = inRoom.filter(
    (m) => !q || (m.nickname ?? "").toLowerCase().includes(q) || m.fingerprint.toLowerCase().includes(q),
  );
  const owner = p.members.find((m) => m.isOwner);
  const active = p.room.status === "active";
  const canManage = p.room.isOwner && active;
  const member = (id: string) => p.members.find((m) => m.memberId === id);

  return (
    <aside
      className="side-panel"
      aria-label="群资料"
    >
      <header className="panel-head">
        <h2>群资料</h2>
        <button className="icon-button" onClick={p.onClose} aria-label="关闭">
          <Icon name="close" />
        </button>
      </header>

      <div className="panel-body">
        <section className="panel-card">
          <div className="card-head">
            <h3>群成员 {inRoom.length}</h3>
            {inRoom.length > 8 && (
              <label className="search-box small">
                <Icon name="search" size={13} />
                <input value={query} onChange={(e) => setQuery(e.currentTarget.value)} placeholder="搜索" />
              </label>
            )}
          </div>
          <div className="member-grid">
            {shown.map((m) => {
              const self = m.memberId === p.me?.memberId;
              const online = self || m.path !== "offline";
              return (
                <div
                  key={m.memberId}
                  className="member-cell"
                  title={`${self ? "我" : PATH_NAMES[m.path]}\n公钥指纹 ${m.fingerprint}`}
                >
                  <Avatar
                    fingerprint={m.fingerprint}
                    name={m.nickname}
                    avatar={m.avatar}
                    size={44}
                    stacks={m.stacks}
                    online={online}
                    pulseKey={p.pulse?.memberId === m.memberId ? p.pulse.key : undefined}
                  />
                  <span className="member-name">
                    {m.isOwner && <span className="owner-tag">群主</span>}
                    {m.nickname || m.fingerprint.slice(0, 6)}
                  </span>
                </div>
              );
            })}
            {!q && active && (
              <button className="member-cell invite" onClick={p.onCopyCode} title="复制房间码，发给群友">
                <span className="invite-box">
                  <Icon name="plus" size={20} />
                </span>
                <span className="member-name">邀请</span>
              </button>
            )}
          </div>
        </section>

        <section className="panel-card">
          <div className="card-head">
            <h3>心之钢排行榜</h3>
          </div>
          {board.every((b) => b.stacks === 0) ? (
            <p className="panel-note">还没有人叠层。5 秒内连发 3 条消息，就能叠一层。</p>
          ) : (
            <ol className="board">
              {board.map((b) => {
                const m = member(b.memberId);
                return (
                  <li key={b.memberId} className={b.rank <= 3 ? `top top-${b.rank}` : undefined}>
                    <span className="rank">{b.rank}</span>
                    <Avatar fingerprint={m?.fingerprint ?? b.memberId} name={m?.nickname} avatar={m?.avatar} size={28} />
                    <span className="board-name">{displayName(b.memberId, p.members)}</span>
                    <span className="board-stacks">
                      <img src={heartsteel} alt="" />
                      {b.stacks}
                    </span>
                  </li>
                );
              })}
            </ol>
          )}
        </section>

        <section className="panel-card rows">
          <div className="info-row">
            <span>群名称</span>
            <span className="info-value">
              <span className="ellipsis">{p.room.title ?? "同步中…"}</span>
              {canManage && (
                <button className="link-button" onClick={p.onRename}>
                  修改
                </button>
              )}
            </span>
          </div>
          <div className="info-row">
            <span>心之钢规则</span>
            <span className="info-value">
              <span>{rule ? ruleText(rule) : "同步中…"}</span>
              {canManage && rule && (
                <button className="link-button" onClick={() => p.onEditRule(rule)}>
                  修改
                </button>
              )}
            </span>
          </div>
          <div className="info-row">
            <span>群主</span>
            <span className="info-value">{owner ? displayName(owner.memberId, p.members) : "未知"}</span>
          </div>
          <div className="info-row">
            <span>房间码</span>
            <button className="soft small" onClick={p.onCopyCode}>
              <Icon name="copy" size={14} />
              复制
            </button>
          </div>
        </section>

        <section className="panel-card rows">
          <label className="switch-row">
            <span>消息免打扰</span>
            <input
              type="checkbox"
              role="switch"
              checked={p.room.muted}
              onChange={(e) => p.onPrefs({ muted: e.currentTarget.checked })}
            />
          </label>
          <label className="switch-row">
            <span>置顶聊天</span>
            <input
              type="checkbox"
              role="switch"
              checked={p.room.pinned}
              onChange={(e) => p.onPrefs({ pinned: e.currentTarget.checked })}
            />
          </label>
          <p className="panel-note">免打扰时这个群不响音效、不提醒，未读只显示小红点；有人 @ 你时照样提醒。这两项只影响本机。</p>
        </section>

        <section className="panel-card rows">
          <button className="row-button" onClick={p.onClearHistory}>
            <Icon name="trash" size={16} />
            清空聊天记录
          </button>
          {active && (
            <button className="row-button danger" onClick={p.onLeave}>
              <Icon name="logout" size={16} />
              退出群聊
            </button>
          )}
          {active && p.room.isOwner && (
            <button className="row-button danger" onClick={p.onDissolve}>
              <Icon name="close" size={16} />
              解散群聊
            </button>
          )}
        </section>
      </div>
    </aside>
  );
}
