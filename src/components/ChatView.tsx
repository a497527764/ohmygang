// 中间的聊天区（微信式）：每条消息旁是发送者头像和层数角标，自己的消息在右边；
// 相隔超过 5 分钟插一条时间；实时叠层时浮出一次「叠到 n 层」。
import { useEffect, useRef, useState } from "react";
import heartsteel from "../assets/heartsteel.png";
import type { Identity, Member, Message, RelayHealth, Room } from "../api";
import { displayName, noticeText, timeText } from "../names";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";

const GAP_MS = 5 * 60 * 1000;

/// undefined 表示还没查到。冷却中的中继也是连着的，算已连接。
function relayText(relays: RelayHealth[] | null | undefined): string {
  if (relays === undefined) return "";
  if (relays === null) return "未联网，只走局域网";
  const connected = relays.filter((r) => r.state === "ok" || r.state === "cooling").length;
  return `中继 ${connected}/${relays.length}`;
}

export interface Pulse {
  memberId: string;
  key: number;
}

export interface Flash {
  text: string;
  key: number;
}

interface Props {
  room: Room;
  me: Identity | null;
  members: Member[];
  messages: Message[];
  relays: RelayHealth[] | null | undefined;
  canLoadOlder: boolean;
  pulse: Pulse | null;
  flash: Flash | null;
  onLoadOlder: () => void;
  /// 成功返回 true，输入框随之清空。
  onSend: (text: string) => Promise<boolean>;
  onOpenDrawer: () => void;
}

export function ChatView(p: Props) {
  const [draft, setDraft] = useState("");
  const bottom = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);

  // 用花括号包住：新版 WebView2 的 scrollIntoView 会返回 Promise，直接返回会被 React 当成清理函数而崩溃
  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [p.messages.length]);

  useEffect(() => {
    setDraft("");
    input.current?.focus();
  }, [p.room.roomId]);

  const member = (id: string) => p.members.find((m) => m.memberId === id);
  const canSend = p.room.status === "active";
  const hint =
    p.room.status === "joining"
      ? "正在等待群里有人上线：和任意一位在线成员同步后就能发言。"
      : p.room.status === "dissolved"
        ? "这个群已解散，只能查看聊天记录。"
        : "";

  async function submit() {
    const text = draft.trim();
    if (!text || !canSend) return;
    if (await p.onSend(text)) setDraft("");
  }

  return (
    <main className="chat">
      <header className="chat-head">
        <h1>{p.room.title ?? "同步中…"}</h1>
        <span className="net">{relayText(p.relays)}</span>
        <button className="icon-button" onClick={p.onOpenDrawer} title="群资料" aria-label="群资料">
          <Icon name="more" size={20} />
        </button>
      </header>

      <div className="messages">
        {p.canLoadOlder && (
          <button className="load-older" onClick={p.onLoadOlder}>
            查看更早的消息
          </button>
        )}
        {p.messages.map((m, i) => {
          const prev = p.messages[i - 1];
          const stamp = !prev || m.displayTs - prev.displayTs > GAP_MS;
          const author = member(m.authorId);
          const mine = m.authorId === p.me?.memberId;
          return (
            <div key={m.eventId}>
              {stamp && <div className="stamp">{timeText(m.displayTs)}</div>}
              {m.kind === "notice" ? (
                <div className="notice">{noticeText(m, p.members)}</div>
              ) : (
                <div className={`msg${mine ? " mine" : ""}`}>
                  <Avatar
                    fingerprint={author?.fingerprint ?? m.authorId}
                    name={author?.nickname}
                    stacks={author?.stacks ?? 0}
                    pulseKey={p.pulse?.memberId === m.authorId ? p.pulse.key : undefined}
                  />
                  <div className="msg-body">
                    {!mine && <div className="msg-name">{displayName(m.authorId, p.members)}</div>}
                    <div className="bubble">{m.text}</div>
                  </div>
                </div>
              )}
            </div>
          );
        })}
        <div ref={bottom} />
      </div>

      {p.flash && (
        <div key={p.flash.key} className="flash" role="status">
          <img src={heartsteel} alt="" />
          {p.flash.text}
        </div>
      )}

      {hint && <div className="hint">{hint}</div>}
      <form
        className="composer"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <input
          ref={input}
          value={draft}
          onChange={(e) => setDraft(e.currentTarget.value)}
          placeholder={canSend ? "说点什么。5 秒内连发 3 条，叠一层心之钢" : ""}
          disabled={!canSend}
          maxLength={300}
          aria-label="输入消息"
        />
        <button type="submit" className="primary" disabled={!canSend || !draft.trim()}>
          发送
        </button>
      </form>
    </main>
  );
}
