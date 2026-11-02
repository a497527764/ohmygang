// 群资料抽屉（需求 3.5，参考微信群资料）：成员网格（可搜索）、心之钢排行榜、免打扰和置顶、房间码。
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

  const q = query.trim().toLowerCase();
  const shown = p.members
    .filter((m) => m.inRoom)
    .filter((m) => !q || (m.nickname ?? "").toLowerCase().includes(q) || m.fingerprint.toLowerCase().includes(q));
  const owner = p.members.find((m) => m.isOwner);
  const active = p.room.status === "active";
  const canManage = p.room.isOwner && active;
  const member = (id: string) => p.members.find((m) => m.memberId === id);

  return (
    <div className="drawer-overlay" onMouseDown={p.onClose}>
      <aside
        className="drawer"
        aria-label="群资料"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") p.onClose();
        }}
      >
        <header className="drawer-head">
          <h2>{p.room.title ?? "同步中…"}</h2>
          <button className="icon-button" onClick={p.onClose} aria-label="关闭" autoFocus>
            <Icon name="close" />
          </button>
        </header>

        <section className="drawer-section">
          <label className="search">
            <Icon name="search" size={15} />
            <input value={query} onChange={(e) => setQuery(e.currentTarget.value)} placeholder="搜索成员" />
          </label>
          <div className="member-grid">
            {shown.map((m) => {
              const self = m.memberId === p.me?.memberId;
              return (
                <div key={m.memberId} className="member-cell" title={`公钥指纹 ${m.fingerprint}`}>
                  <Avatar
                    fingerprint={m.fingerprint}
                    name={m.nickname}
                    size={44}
                    stacks={m.stacks}
                    pulseKey={p.pulse?.memberId === m.memberId ? p.pulse.key : undefined}
                  />
                  <span className="member-name">{m.nickname || m.fingerprint.slice(0, 6)}</span>
                  {!self && (
                    <span className={`path path-${m.path}`} title={`联系路径：${PATH_NAMES[m.path]}`}>
                      {PATH_NAMES[m.path]}
                    </span>
                  )}
                </div>
              );
            })}
            {!q && (
              <button className="member-cell invite" onClick={p.onCopyCode} title="复制房间码，发给群友">
                <span className="invite-box">
                  <Icon name="plus" size={20} />
                </span>
                <span className="member-name">邀请</span>
              </button>
            )}
          </div>
        </section>

        <section className="drawer-section">
          <h3>心之钢排行榜</h3>
          {board.every((b) => b.stacks === 0) ? (
            <p className="drawer-note">还没有人叠层。5 秒内连发 3 条消息，就能叠一层。</p>
          ) : (
            <ol className="board">
              {board.map((b) => {
                const m = member(b.memberId);
                return (
                  <li key={b.memberId} className={b.rank <= 3 ? `top top-${b.rank}` : undefined}>
                    <span className="rank">{b.rank}</span>
                    <Avatar fingerprint={m?.fingerprint ?? b.memberId} name={m?.nickname} size={28} />
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

        <section className="drawer-section">
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
          <p className="drawer-note">免打扰时，这个群的心之钢音效不响，未读只显示小红点。这两项只影响本机。</p>
        </section>

        <section className="drawer-section info">
          <div className="info-row">
            <span>群名称</span>
            <span className="info-value">
              {p.room.title ?? "同步中…"}
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
              {rule ? ruleText(rule) : "同步中…"}
              {canManage && rule && (
                <button className="link-button" onClick={() => p.onEditRule(rule)}>
                  修改
                </button>
              )}
            </span>
          </div>
          <div className="info-row">
            <span>房主</span>
            <span>{owner ? displayName(owner.memberId, p.members) : "未知"}</span>
          </div>
          <div className="info-row">
            <span>房间码</span>
            <button className="soft" onClick={p.onCopyCode}>
              <Icon name="copy" size={14} />
              复制
            </button>
          </div>
        </section>

        {active && (
          <section className="drawer-section danger-zone">
            <button className="danger-button" onClick={p.onLeave}>
              退出群聊
            </button>
            {p.room.isOwner && (
              <button className="danger-button" onClick={p.onDissolve}>
                解散群聊
              </button>
            )}
          </section>
        )}
      </aside>
    </div>
  );
}
