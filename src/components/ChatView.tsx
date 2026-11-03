// 中间的聊天区：顶栏（群名、人数、在线人数、中继状态、搜索、群资料），消息列表，输入区。
// 同一个人 3 分钟内连发的消息合并在一起，只在第一条显示头像（带层数角标）和名字；相隔超过 5 分钟插一条时间。
// 鼠标移到消息上出现「回复」「复制」「@TA」；引用块点一下跳到原消息；@ 到的人名高亮，@ 我的消息左边有一道色条。
// 实时叠层时浮出一次「叠到 n 层」。新消息只在已经看到底部或是自己发的时候才自动滚到底，否则出现「有新消息」。
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import heartsteel from "../assets/heartsteel.png";
import type { Identity, Member, Message, RelayHealth, Room } from "../api";
import { firstImage } from "../image";
import { displayName, noticeText, timeText } from "../names";
import { Avatar } from "./Avatar";
import { Composer } from "./Composer";
import { Icon } from "./Icon";
import { ImageViewer, MessageImage } from "./Images";

const GAP_MS = 5 * 60 * 1000;
const RUN_MS = 3 * 60 * 1000;
/// 对方时钟偏差超过 2 分钟时按本机收到时间显示（Rust 侧已处理），这里加一句提示（需求 3.3）。
const SKEW_MS = 2 * 60 * 1000;

/// undefined 表示还没查到。冷却中的中继也是连着的，算已连接。
function relayText(relays: RelayHealth[] | null | undefined): string {
  if (relays === undefined) return "";
  if (relays === null) return "只走局域网";
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
  /// 要跳到并闪一下的消息。
  highlight: { eventId: string; key: number } | null;
  onLoadOlder: () => Promise<void>;
  onSend: (text: string, replyTo: string | null, mentions: string[]) => Promise<boolean>;
  onSendImage: (file: Blob) => Promise<boolean>;
  onJump: (eventId: string) => void;
  onOpenDrawer: () => void;
  onOpenSearch: () => void;
  onCancelJoin: () => void;
  onToast: (text: string) => void;
}

/// 把文字里「@被提到的人」包成高亮。
function withMentions(text: string, names: string[]) {
  if (names.length === 0) return text;
  const escaped = names.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const parts = text.split(new RegExp(`(@(?:${escaped.join("|")}))`, "g"));
  return parts.map((part, i) =>
    i % 2 === 1 ? (
      <span key={i} className="mention">
        {part}
      </span>
    ) : (
      part
    ),
  );
}

export function ChatView(p: Props) {
  const list = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const prevFirst = useRef<{ id: string | null; height: number }>({ id: null, height: 0 });
  const [unseen, setUnseen] = useState(false);
  const [replyTo, setReplyTo] = useState<Message | null>(null);
  const [mentionRequest, setMentionRequest] = useState<{ memberId: string; key: number } | null>(null);
  const [viewing, setViewing] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);

  const last = p.messages[p.messages.length - 1];
  const first = p.messages[0];

  useEffect(() => {
    setReplyTo(null);
    setUnseen(false);
    atBottom.current = true;
  }, [p.room.roomId]);

  // 往前翻页时保持当前看到的位置不动
  useLayoutEffect(() => {
    const el = list.current;
    if (!el) return;
    const prev = prevFirst.current;
    if (prev.id && first && prev.id !== first.eventId && !atBottom.current) {
      el.scrollTop += el.scrollHeight - prev.height;
    }
    prevFirst.current = { id: first?.eventId ?? null, height: el.scrollHeight };
  }, [first?.eventId]);

  // 新消息：在底部或是自己发的就滚到底，否则提示有新消息
  useEffect(() => {
    const el = list.current;
    if (!el || !last) return;
    if (atBottom.current || last.authorId === p.me?.memberId) {
      el.scrollTop = el.scrollHeight;
      setUnseen(false);
    } else {
      setUnseen(true);
    }
    prevFirst.current.height = el.scrollHeight;
  }, [last?.eventId, p.me?.memberId]);

  useEffect(() => {
    if (!p.highlight) return;
    const el = list.current?.querySelector(`[data-id="${p.highlight.eventId}"]`);
    el?.scrollIntoView({ block: "center" });
  }, [p.highlight]);

  const member = (id: string) => p.members.find((m) => m.memberId === id);
  const canSend = p.room.status === "active";
  const inRoom = p.members.filter((m) => m.inRoom);
  // 群里只有自己时没人能确认收到，不显示「发送中」
  const alone = inRoom.length <= 1;
  const online = inRoom.filter((m) => m.memberId === p.me?.memberId || m.path !== "offline").length;
  const hint =
    p.room.status === "joining"
      ? "正在等群里有人上线：和任意一位在线的群友同步之后就能发言。"
      : p.room.status === "dissolved"
        ? "这个群已解散，只能查看聊天记录。"
        : p.room.status === "left"
          ? "你已退出这个群，聊天记录只能查看。用房间码可以重新进群。"
          : "";

  const mentionedNames = (m: Message) =>
    m.mentions
      .map((s) => p.members.find((x) => x.memberId.startsWith(s)))
      .filter((x): x is Member => !!x)
      .map((x) => x.nickname || x.fingerprint.slice(0, 6));
  const myShort = p.me?.memberId.slice(0, 16);

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      p.onToast("已复制");
    } catch {
      p.onToast("复制失败");
    }
  }

  return (
    <main
      className={dragging ? "chat dragging" : "chat"}
      onDragOver={(e) => {
        if (canSend && Array.from(e.dataTransfer.items).some((i) => i.type.startsWith("image/"))) {
          e.preventDefault();
          setDragging(true);
        }
      }}
      onDragLeave={(e) => {
        if (e.currentTarget === e.target) setDragging(false);
      }}
      onDrop={(e) => {
        setDragging(false);
        const img = firstImage(e.dataTransfer.items);
        if (img && canSend) {
          e.preventDefault();
          p.onSendImage(img);
        }
      }}
    >
      <header className="chat-head">
        <div className="chat-title">
          <h1>
            {p.room.title ?? "同步中…"}
            {inRoom.length > 0 && <span className="count">（{inRoom.length}）</span>}
          </h1>
          <div className="chat-meta">
            {p.room.status === "active" && inRoom.length > 0 && (
              <span className="meta-item">
                <span className="online-dot" />
                {online} 人在线
              </span>
            )}
            {relayText(p.relays) && <span className="meta-item">{relayText(p.relays)}</span>}
          </div>
        </div>
        <button className="icon-button" onClick={p.onOpenSearch} title="搜索聊天记录" aria-label="搜索聊天记录">
          <Icon name="search" size={19} />
        </button>
        <button className="icon-button" onClick={p.onOpenDrawer} title="群资料" aria-label="群资料">
          <Icon name="more" size={20} />
        </button>
      </header>

      <div
        className="messages"
        ref={list}
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
          if (atBottom.current) setUnseen(false);
        }}
      >
        {p.canLoadOlder && (
          <button className="load-older" onClick={() => p.onLoadOlder()}>
            查看更早的消息
          </button>
        )}
        {p.messages.length === 0 && canSend && (
          <div className="chat-empty">
            <img src={heartsteel} alt="" width={56} height={56} />
            <p>还没有消息。5 秒内连发 3 条，叠一层心之钢。</p>
          </div>
        )}
        {p.messages.map((m, i) => {
          const prev = p.messages[i - 1];
          const stamp = !prev || m.displayTs - prev.displayTs > GAP_MS;
          if (m.kind === "notice") {
            return (
              <div key={m.eventId} data-id={m.eventId}>
                {stamp && <div className="stamp">{timeText(m.displayTs)}</div>}
                <div className="notice">{noticeText(m, p.members)}</div>
              </div>
            );
          }
          const author = member(m.authorId);
          const mine = m.authorId === p.me?.memberId;
          const cont =
            !stamp && prev?.kind === "chat" && prev.authorId === m.authorId && m.displayTs - prev.displayTs < RUN_MS;
          const atMe = !mine && !!myShort && m.mentions.includes(myShort);
          const flashing = p.highlight?.eventId === m.eventId;
          return (
            <div key={m.eventId} data-id={m.eventId}>
              {stamp && <div className="stamp">{timeText(m.displayTs)}</div>}
              <div
                className={`msg${mine ? " mine" : ""}${cont ? " cont" : ""}${atMe ? " at-me" : ""}${flashing ? " flash-hl" : ""}`}
                key={flashing ? p.highlight?.key : undefined}
              >
                <span className="msg-avatar">
                  {!cont && (
                    <Avatar
                      fingerprint={author?.fingerprint ?? m.authorId}
                      name={author?.nickname}
                      avatar={author?.avatar}
                      stacks={author?.stacks ?? 0}
                      pulseKey={p.pulse?.memberId === m.authorId ? p.pulse.key : undefined}
                    />
                  )}
                </span>
                <div className="msg-body">
                  {!mine && !cont && (
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
                    {mine && m.delivery === "pending" && !alone && (
                      <span className="sending" title="发送中：还没有群友确认收到，会自动重发" aria-label="发送中" />
                    )}
                    {m.image ? (
                      <MessageImage roomId={p.room.roomId} eventId={m.eventId} info={m.image} onOpen={setViewing} />
                    ) : (
                      <div className="bubble">
                        {m.reply && (
                          <button className="quote" onClick={() => p.onJump(m.reply!.eventId)} title="跳到原消息">
                            <span className="quote-who">
                              {m.reply.authorId ? displayName(m.reply.authorId, p.members) : "原消息"}
                            </span>
                            <span className="quote-text">{m.reply.excerpt}</span>
                          </button>
                        )}
                        <span className="bubble-text">{withMentions(m.text ?? "", mentionedNames(m))}</span>
                      </div>
                    )}
                    {canSend && (
                      <div className="msg-actions">
                        <button onClick={() => setReplyTo(m)} title="回复" aria-label="回复">
                          <Icon name="reply" size={15} />
                        </button>
                        {m.text && (
                          <button onClick={() => copy(m.text!)} title="复制" aria-label="复制">
                            <Icon name="copy" size={15} />
                          </button>
                        )}
                        {!mine && author?.inRoom && (
                          <button
                            onClick={() => setMentionRequest({ memberId: m.authorId, key: Date.now() })}
                            title="@TA"
                            aria-label="@TA"
                          >
                            <Icon name="at" size={15} />
                          </button>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {unseen && (
        <button
          className="new-below"
          onClick={() => {
            const el = list.current;
            if (el) el.scrollTop = el.scrollHeight;
            setUnseen(false);
          }}
        >
          <Icon name="arrowDown" size={14} />
          有新消息
        </button>
      )}

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
      {(canSend || !hint) && (
        <Composer
          roomId={p.room.roomId}
          me={p.me}
          members={p.members}
          enabled={canSend}
          replyTo={replyTo}
          onCancelReply={() => setReplyTo(null)}
          onSend={async (text, reply, mentions) => {
            const ok = await p.onSend(text, reply, mentions);
            if (ok) setReplyTo(null);
            return ok;
          }}
          onSendImage={p.onSendImage}
          mentionRequest={mentionRequest}
        />
      )}
      {dragging && <div className="drop-hint">松开发送图片</div>}
      {viewing && <ImageViewer url={viewing} onClose={() => setViewing(null)} />}
    </main>
  );
}
