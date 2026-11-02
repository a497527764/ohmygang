// 左侧房间列表：置顶的在前（Rust 已排好），免打扰的房间未读只显示小红点。
import type { Room } from "../api";
import { shortTime } from "../names";
import { Icon } from "./Icon";

interface Props {
  rooms: Room[];
  active: string | null;
  onOpen: (roomId: string) => void;
  onCreate: () => void;
  onJoin: () => void;
}

function subtitle(r: Room): string {
  if (r.status === "joining") return "等待成员上线";
  if (r.status === "dissolved") return "已解散";
  if (r.status === "left") return "已退出";
  return "";
}

export function RoomList({ rooms, active, onOpen, onCreate, onJoin }: Props) {
  return (
    <aside className="room-list">
      <div className="room-list-actions">
        <button className="soft" onClick={onCreate}>
          <Icon name="plus" size={15} />
          建房
        </button>
        <button className="soft" onClick={onJoin}>
          进房
        </button>
      </div>
      {rooms.length === 0 && <p className="list-empty">还没有房间。建一个，或者用群友发来的房间码进房。</p>}
      <ul>
        {rooms.map((r) => (
          <li key={r.roomId}>
            <button
              className={`room-row${r.roomId === active ? " selected" : ""}${r.pinned ? " pinned" : ""}`}
              onClick={() => onOpen(r.roomId)}
            >
              <span className="room-avatar">{Array.from(r.title ?? "?")[0]}</span>
              <span className="room-main">
                <span className="room-title">{r.title ?? "同步中…"}</span>
                <span className="room-sub">{subtitle(r)}</span>
              </span>
              <span className="room-side">
                <span className="room-time">{shortTime(r.lastActive)}</span>
                <span className="room-flags">
                  {r.muted && (
                    <span className="muted-icon" title="消息免打扰">
                      <Icon name="bellOff" size={13} />
                    </span>
                  )}
                  {r.unread > 0 &&
                    (r.muted ? (
                      <span className="dot" title={`${r.unread} 条未读`} />
                    ) : (
                      <span className="unread">{r.unread > 99 ? "99+" : r.unread}</span>
                    ))}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </aside>
  );
}
