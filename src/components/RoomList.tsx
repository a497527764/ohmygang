// 左栏的聊天列表：标题「聊天」和新建按钮（建一个群、用房间码进群），搜索框，筛选（全部、未读、@我），
// 每行是群头像、群名、时间、最后一条消息和未读数。置顶的在前（Rust 已排好）；免打扰的未读数是灰色的；
// 有人 @ 我时预览前面有「[有人@我]」。
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

type Filter = "all" | "unread" | "mentioned";
const FILTERS: { id: Filter; name: string }[] = [
  { id: "all", name: "全部" },
  { id: "unread", name: "未读" },
  { id: "mentioned", name: "@我" },
];

function statusText(r: Room): string | null {
  if (r.status === "joining") return "等群友上线后就能聊天";
  if (r.status === "dissolved") return "群已解散";
  if (r.status === "left") return "你已退出这个群";
  return null;
}

export function RoomAvatar({ room, size = 48 }: { room: Room; size?: number }) {
  return (
    <span
      className="room-avatar"
      style={{ width: size, height: size, ["--h" as string]: hueOf(room.roomId), fontSize: Math.round(size * 0.4) }}
      aria-hidden="true"
    >
      {Array.from(room.title ?? "…")[0]}
    </span>
  );
}

export function RoomList({ rooms, active, onOpen, onCreate, onJoin }: Props) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
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
  const shown = rooms
    .filter((r) => !q || (r.title ?? "").toLowerCase().includes(q))
    .filter((r) => (filter === "unread" ? r.unread > 0 : filter === "mentioned" ? r.mentioned : true));

  return (
    <>
      <header className="side-head">
        <h1>聊天</h1>
        <div className="menu-anchor" ref={menuRef}>
          <button
            className="icon-button"
            onClick={() => setMenu((m) => !m)}
            title="建群或进群"
            aria-label="建群或进群"
            aria-expanded={menu}
          >
            <Icon name="chatPlus" size={21} />
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
                <Icon name="users" size={18} />
                建一个群
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  onJoin();
                }}
              >
                <Icon name="link" size={18} />
                用房间码进群
              </button>
            </div>
          )}
        </div>
      </header>

      <div className="side-tools">
        <label className="search-pill">
          <Icon name="search" size={16} />
          <input value={query} onChange={(e) => setQuery(e.currentTarget.value)} placeholder="搜索群聊" />
        </label>
        <div className="chips" role="tablist" aria-label="筛选">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              role="tab"
              aria-selected={filter === f.id}
              className={filter === f.id ? "chip on" : "chip"}
              onClick={() => setFilter(f.id)}
            >
              {f.name}
            </button>
          ))}
        </div>
      </div>

      <div className="chat-list">
        {rooms.length === 0 && (
          <div className="list-empty">
            <p>还没有群聊</p>
            <span>点右上角的按钮建一个群，或者用群友发来的房间码进群。</span>
          </div>
        )}
        {rooms.length > 0 && shown.length === 0 && (
          <p className="list-none">
            {q ? `没有名字里带「${query.trim()}」的群` : filter === "unread" ? "没有未读消息" : "没有人 @ 你"}
          </p>
        )}
        {shown.map((r) => {
          const status = statusText(r);
          const loud = r.unread > 0 && (!r.muted || r.mentioned);
          return (
            <button
              key={r.roomId}
              className={`chat-row${r.roomId === active ? " selected" : ""}`}
              onClick={() => onOpen(r.roomId)}
            >
              <RoomAvatar room={r} />
              <span className="row-main">
                <span className="row-line">
                  <span className="row-title">{r.title ?? "同步中…"}</span>
                  <span className={loud ? "row-time loud" : "row-time"}>{shortTime(r.lastActive)}</span>
                </span>
                <span className="row-line">
                  <span className="row-sub">
                    {r.mentioned && <em className="at-me-tag">[有人@我]</em>}
                    {status ? <span className="row-status">{status}</span> : r.preview}
                  </span>
                  <span className="row-flags">
                    {r.muted && (
                      <span className="muted-icon" title="消息免打扰">
                        <Icon name="bellOff" size={15} />
                      </span>
                    )}
                    {r.pinned && (
                      <span className="pin-icon" title="已置顶">
                        <Icon name="pin" size={15} />
                      </span>
                    )}
                    {r.unread > 0 && (
                      <span className={loud ? "unread" : "unread quiet"}>{r.unread > 99 ? "99+" : r.unread}</span>
                    )}
                  </span>
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </>
  );
}
