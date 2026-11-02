// 中间的聊天区（微信式）：每条消息旁是发送者头像和层数角标，自己的消息在右边；
// 相隔超过 5 分钟插一条时间；实时叠层时浮出一次「叠到 n 层」。
import { useEffect, useRef, useState } from "react";
import heartsteel from "../assets/heartsteel.png";
import type { Identity, Member, Message, RelayHealth, Room } from "../api";
import { displayName, noticeText, timeText } from "../names";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";

const GAP_MS = 5 * 60 * 1000;
/// 对方时钟偏差超过 2 分钟时按本机收到时间显示（Rust 侧已处理），这里加一句提示（需求 3.3）。
const SKEW_MS = 2 * 60 * 1000;

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
  onCancelJoin: () => void;
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
        : p.room.status === "left"
          ? "你已退出这个群，聊天记录只能查看。用房间码可以重新进房。"
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
                    {!mine && (
                      <div className="msg-name">
                        {displayName(m.authorId, p.members)}
                        {Math.abs(m.tsMs - m.displayTs) > SKEW_MS && (
                          <span className="skew" title={`对方电脑上的时间是 ${timeText(m.tsMs)}`}>
                            对方时钟不准，按收到时间显示
                          </span>
                        )}
                      </div>
                    )}
                    <div className="bubble-line">
                      {mine && m.delivery === "pending" && (
                        <span className="sending" title="发送中：还没有群友确认收到，会自动重发" aria-label="发送中" />
                      )}
                      <div className="bubble">{m.text}</div>
                    </div>
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

      {hint && (
        <div className="hint">
          <span>{hint}</span>
          {p.room.status === "joining" && (
            <button className="link-button" onClick={p.onCancelJoin}>
              取消加入
            </button>
          )}
        </div>
      )}
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
