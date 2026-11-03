// 左侧房间列表：顶部搜索群名和「+」菜单（建群、用房间码进群）；每行是群头像、群名、最后一条消息、时间和未读。
// 置顶的在前（Rust 已排好）；免打扰的未读只显示小红点；有人 @ 我时预览前面加红色的「[有人@我]」。
import { useEffect, useRef, useState } from "react";
import type { Room } from "../api";
import { hueOf } from "../avatars";
import { shortTime } from "../names";
import { Icon } from "./Icon";

interface Props {
  rooms: Room[];
  active: string | null;
  onOpen: (roomId: string) => void;
  onCreate: () => void;
  onJoin: () => void;
}

function statusText(r: Room): string | null {
  if (r.status === "joining") return "等待群友上线";
  if (r.status === "dissolved") return "已解散";
  if (r.status === "left") return "已退出";
  return null;
}

export function RoomAvatar({ room, size = 42 }: { room: Room; size?: number }) {
  return (
    <span
      className="room-avatar"
      style={{ width: size, height: size, ["--h" as string]: hueOf(room.roomId), fontSize: Math.round(size * 0.42) }}
      aria-hidden="true"
    >
      {Array.from(room.title ?? "…")[0]}
    </span>
  );
}

export function RoomList({ rooms, active, onOpen, onCreate, onJoin }: Props) {
  const [query, setQuery] = useState("");
  const [menu, setMenu] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenu(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  const q = query.trim().toLowerCase();
  const shown = q ? rooms.filter((r) => (r.title ?? "").toLowerCase().includes(q)) : rooms;

  return (
    <aside className="room-list">
      <div className="room-list-top">
        <label className="search-box">
          <Icon name="search" size={15} />
          <input value={query} onChange={(e) => setQuery(e.currentTarget.value)} placeholder="搜索群聊" />
        </label>
        <div className="menu-anchor" ref={menuRef}>
          <button
            className="icon-button filled"
            onClick={() => setMenu((m) => !m)}
            title="建群或进群"
            aria-label="建群或进群"
            aria-expanded={menu}
          >
            <Icon name="plus" size={18} />
          </button>
          {menu && (
            <div className="menu" role="menu">
              <button
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  onCreate();
                }}
              >
                <Icon name="users" size={16} />
                建一个群
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  onJoin();
                }}
              >
                <Icon name="door" size={16} />
                用房间码进群
              </button>
            </div>
          )}
        </div>
      </div>

      {rooms.length === 0 && (
        <div className="list-empty">
          <p>还没有群聊</p>
          <span>点右上角的 + 建一个群，或者用群友发来的房间码进群。</span>
        </div>
      )}
      {rooms.length > 0 && shown.length === 0 && <p className="list-none">没有名字里带「{query.trim()}」的群</p>}
      <ul>
        {shown.map((r) => {
          const status = statusText(r);
          return (
            <li key={r.roomId}>
              <button
                className={`room-row${r.roomId === active ? " selected" : ""}${r.pinned ? " pinned" : ""}`}
                onClick={() => onOpen(r.roomId)}
              >
                <RoomAvatar room={r} />
                <span className="room-main">
                  <span className="room-line">
                    <span className="room-title">{r.title ?? "同步中…"}</span>
                    <span className="room-time">{shortTime(r.lastActive)}</span>
                  </span>
                  <span className="room-line">
                    <span className="room-sub">
                      {r.mentioned && <em className="at-me-tag">[有人@我]</em>}
                      {status ? <span className="room-status">{status}</span> : r.preview}
                    </span>
                    <span className="room-flags">
                      {r.muted && (
                        <span className="muted-icon" title="消息免打扰">
                          <Icon name="bellOff" size={13} />
                        </span>
                      )}
                      {r.unread > 0 &&
                        (r.muted && !r.mentioned ? (
                          <span className="dot" title={`${r.unread} 条未读`} />
                        ) : (
                          <span className="unread">{r.unread > 99 ? "99+" : r.unread}</span>
                        ))}
                    </span>
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </aside>
  );
}
