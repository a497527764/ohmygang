// 群资料（需求 3.5，V1.1 第二版参考 WhatsApp 的群信息页）：右侧面板，各部分之间用留白隔开。
// 顶部是群头像、群名（群主可改）、人数和「邀请」「搜索」两个按钮；
// 「心之钢排行榜」就是成员列表：按层数排名，头像左下角是在线小绿点，右边是层数。
// 下面是心之钢规则、免打扰和置顶，最后是清空聊天记录、退出、解散。
// 排行和角标都来自 Rust 重放出的层数，所以一定一致；层数变化时由 refreshKey 触发重新拉取。
import { useEffect, useState } from "react";
import heartsteel from "../assets/heartsteel.png";
import { api, type Identity, type LeaderboardEntry, type Member, type Room, type RoomRule } from "../api";
import { PATH_NAMES } from "../names";
import { ruleText } from "../rule";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";
import { RoomAvatar } from "./RoomList";
import type { Pulse } from "./ChatView";

interface Props {
  room: Room;
  me: Identity | null;
  members: Member[];
  refreshKey: number;
  pulse: Pulse | null;
  onClose: () => void;
  onCopyCode: () => void;
  onSearch: () => void;
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
  const { onError, onClose } = p;

  useEffect(() => {
    api.leaderboard(roomId).then(setBoard).catch(onError);
    api.roomRule(roomId).then(setRule).catch(onError);
  }, [roomId, p.refreshKey, onError]);

  // Esc 关闭（焦点不一定在面板里；有对话框开着时让对话框先处理）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector(".overlay")) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const inRoom = p.members.filter((m) => m.inRoom);
  const active = p.room.status === "active";
  const canManage = p.room.isOwner && active;
  const q = query.trim().toLowerCase();
  // 排行榜里只有当前成员，按 Rust 给的名次；还没拉到时按成员列表的顺序（同样按层数降序）
  const ranked = (board.length > 0 ? board.map((b) => ({ rank: b.rank, m: p.members.find((x) => x.memberId === b.memberId) })) : inRoom.map((m, i) => ({ rank: i + 1, m })))
    .filter((x): x is { rank: number; m: Member } => !!x.m)
    .filter(({ m }) => !q || (m.nickname ?? "").toLowerCase().includes(q) || m.fingerprint.toLowerCase().includes(q));

  return (
    <aside className="side-panel" aria-label="群资料">
      <header className="panel-head">
        <button className="icon-button" onClick={p.onClose} aria-label="关闭" title="关闭">
          <Icon name="close" size={20} />
        </button>
        <h2>群资料</h2>
      </header>

      <div className="panel-body">
        <section className="info-hero">
          <RoomAvatar room={p.room} size={104} />
          <div className="hero-title">
            <span>{p.room.title ?? "同步中…"}</span>
            {canManage && (
              <button className="icon-button small" onClick={p.onRename} title="修改群名称" aria-label="修改群名称">
                <Icon name="edit" size={16} />
              </button>
            )}
          </div>
          <p className="hero-sub">群聊，{inRoom.length} 位成员</p>
          <div className="hero-actions">
            {active && (
              <button className="hero-action" onClick={p.onCopyCode}>
                <Icon name="link" size={20} />
                邀请
              </button>
            )}
            <button className="hero-action" onClick={p.onSearch}>
              <Icon name="search" size={20} />
              搜索记录
            </button>
          </div>
        </section>

        <section className="info-section">
          <div className="section-head">
            <h3>心之钢规则</h3>
            {canManage && rule && (
              <button className="link-button" onClick={() => p.onEditRule(rule)}>
                修改
              </button>
            )}
          </div>
          <p className="section-text">{rule ? ruleText(rule) : "同步中…"}</p>
        </section>

        <section className="info-section flush">
          <div className="section-head padded">
            <h3>心之钢排行榜</h3>
            {inRoom.length > 8 && (
              <label className="search-pill small">
                <Icon name="search" size={14} />
                <input value={query} onChange={(e) => setQuery(e.currentTarget.value)} placeholder="找人" />
              </label>
            )}
          </div>
          {active && !q && (
            <button className="member-row invite" onClick={p.onCopyCode}>
              <span className="invite-icon">
                <Icon name="plus" size={22} />
              </span>
              <span className="member-text">
                <span className="member-name">邀请群友</span>
                <span className="member-sub">复制房间码，发给群友就能进群</span>
              </span>
            </button>
          )}
          {ranked.map(({ rank, m }) => {
            const self = m.memberId === p.me?.memberId;
            const online = self || m.path !== "offline";
            return (
              <div key={m.memberId} className="member-row" title={`公钥指纹 ${m.fingerprint}`}>
                <span className={rank <= 3 && m.stacks > 0 ? `rank top-${rank}` : "rank"}>{rank}</span>
                <Avatar
                  fingerprint={m.fingerprint}
                  name={m.nickname}
                  avatar={m.avatar}
                  size={40}
                  online={online}
                  pulseKey={p.pulse?.memberId === m.memberId ? p.pulse.key : undefined}
                />
                <span className="member-text">
                  <span className="member-name">
                    {self ? "你" : m.nickname || m.fingerprint.slice(0, 6)}
                    {m.isOwner && <span className="owner-tag">群主</span>}
                  </span>
                  <span className="member-sub">{self ? "本机" : PATH_NAMES[m.path]}</span>
                </span>
                <span className={m.stacks > 0 ? "member-stacks" : "member-stacks zero"}>
                  <img src={heartsteel} alt="" />
                  {m.stacks}
                </span>
              </div>
            );
          })}
        </section>

        <section className="info-section flush">
          <label className="toggle-row">
            <Icon name="bellOff" size={20} />
            <span className="toggle-text">
              <span>消息免打扰</span>
              <small>不响音效、不提醒；有人 @ 你时照样提醒</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={p.room.muted}
              onChange={(e) => p.onPrefs({ muted: e.currentTarget.checked })}
            />
          </label>
          <label className="toggle-row">
            <Icon name="pin" size={20} />
            <span className="toggle-text">
              <span>置顶聊天</span>
              <small>这两项只影响本机</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={p.room.pinned}
              onChange={(e) => p.onPrefs({ pinned: e.currentTarget.checked })}
            />
          </label>
        </section>

        <section className="info-section flush">
          <button className="action-row danger" onClick={p.onClearHistory}>
            <Icon name="trash" size={20} />
            清空聊天记录
          </button>
          {active && (
            <button className="action-row danger" onClick={p.onLeave}>
              <Icon name="logout" size={20} />
              退出群聊
            </button>
          )}
          {active && p.room.isOwner && (
            <button className="action-row danger" onClick={p.onDissolve}>
              <Icon name="close" size={20} />
              解散群聊
            </button>
          )}
        </section>
      </div>
    </aside>
  );
}
