// 聊天记录搜索（V1.1）：右侧面板，在当前群的文字消息里找关键字，点结果跳到那条消息。
// 搜索在 Rust 侧做（解码聊天文字逐条匹配），这里输入停顿 250 毫秒后再查。
import { useEffect, useState } from "react";
import { api, type Member, type Message, type Room } from "../api";
import { displayName, timeText } from "../names";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";

interface Props {
  room: Room;
  members: Member[];
  onJump: (eventId: string) => void;
  onClose: () => void;
}

/// 结果里把关键字标出来。
function marked(text: string, q: string) {
  const i = text.toLowerCase().indexOf(q.toLowerCase());
  if (!q || i < 0) return text;
  return (
    <>
      {text.slice(0, i)}
      <mark>{text.slice(i, i + q.length)}</mark>
      {text.slice(i + q.length)}
    </>
  );
}

export function SearchPanel({ room, members, onJump, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Message[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults(null);
      return;
    }
    const t = setTimeout(() => {
      api
        .search(room.roomId, q)
        .then((r) => {
          setResults(r);
          setError(null);
        })
        .catch((e) => setError(String(e)));
    }, 250);
    return () => clearTimeout(t);
  }, [query, room.roomId]);

  const q = query.trim();
  return (
    <aside
      className="side-panel"
      aria-label="搜索聊天记录"
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
    >
      <header className="panel-head">
        <h2>搜索聊天记录</h2>
        <button className="icon-button" onClick={onClose} aria-label="关闭">
          <Icon name="close" />
        </button>
      </header>
      <div className="panel-body">
        <label className="search-box large">
          <Icon name="search" size={16} />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
            placeholder={`在「${room.title ?? "这个群"}」里搜索`}
          />
        </label>
        {error && <p className="field-error">{error}</p>}
        {results === null ? (
          <p className="panel-note">输入关键字，找这个群里的文字消息。图片不在搜索范围内。</p>
        ) : results.length === 0 ? (
          <p className="panel-note">没找到包含「{q}」的消息。</p>
        ) : (
          <>
            <p className="panel-note">
              {results.length >= 50 ? "最近 50 条结果" : `${results.length} 条结果`}，点一下跳到那条消息
            </p>
            <ul className="search-results">
              {results.map((m) => {
                const author = members.find((x) => x.memberId === m.authorId);
                return (
                  <li key={m.eventId}>
                    <button onClick={() => onJump(m.eventId)}>
                      <Avatar
                        fingerprint={author?.fingerprint ?? m.authorId}
                        name={author?.nickname}
                        avatar={author?.avatar}
                        size={32}
                      />
                      <span className="result-main">
                        <span className="result-line">
                          <span className="result-name">{displayName(m.authorId, members)}</span>
                          <span className="result-time">{timeText(m.displayTs)}</span>
                        </span>
                        <span className="result-text">{marked(m.text ?? "", q)}</span>
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </>
        )}
      </div>
    </aside>
  );
}
