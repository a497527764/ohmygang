// 聊天区（V1.1 第二版，参考 WhatsApp 的布局）：
// 顶栏是群头像、群名和在线人数（点一下打开群资料），右边是我的心之钢层数、搜索和群资料按钮。
// 消息区铺着淡淡的底纹；按天插入「今天」「昨天」分隔；同一个人 3 分钟内连发合并，第一条带小尾巴、头像和名字；
// 时间和发送状态（时钟 = 发送中，双勾 = 群友已收到）放在气泡右下角。鼠标移到气泡上，右上角出现小箭头，
// 点开有回复、复制、@TA。引用块点一下跳到原消息；@ 到的人名用绿色标出。
// 新消息只在已经看到底部或是自己发的时候才自动滚到底，否则右下角的圆按钮上显示有新消息。
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import heartsteel from "../assets/heartsteel.png";
import type { Identity, Member, Message, RelayHealth, Room } from "../api";
import { nameStyle } from "../avatars";
import { firstImage } from "../image";
import { clockText, dayText, displayName, noticeText, timeText } from "../names";
import { Avatar } from "./Avatar";
import { Composer } from "./Composer";
import { Icon } from "./Icon";
import { ImageViewer, MessageImage } from "./Images";
import { RoomAvatar } from "./RoomList";

const RUN_MS = 3 * 60 * 1000;
/// 对方时钟偏差超过 2 分钟时按本机收到时间显示（Rust 侧已处理），这里加一句提示（需求 3.3）。
const SKEW_MS = 2 * 60 * 1000;

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
  const [unseen, setUnseen] = useState(0);
  const [replyTo, setReplyTo] = useState<Message | null>(null);
  const [mentionRequest, setMentionRequest] = useState<{ memberId: string; key: number } | null>(null);
  const [viewing, setViewing] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [menuFor, setMenuFor] = useState<string | null>(null);

  const last = p.messages[p.messages.length - 1];
  const first = p.messages[0];

  useEffect(() => {
    setReplyTo(null);
    setUnseen(0);
    setMenuFor(null);
    atBottom.current = true;
  }, [p.room.roomId]);

  useEffect(() => {
    if (!menuFor) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest(".bubble-menu, .bubble-menu-btn")) setMenuFor(null);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menuFor]);

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

  // 新消息：在底部或是自己发的就滚到底，否则记一条没看到的
  useEffect(() => {
    const el = list.current;
    if (!el || !last) return;
    if (atBottom.current || last.authorId === p.me?.memberId) {
      el.scrollTop = el.scrollHeight;
      setUnseen(0);
    } else {
      setUnseen((n) => n + 1);
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
  // 群里只有自己时没人能确认收到，不显示发送状态
  const alone = inRoom.length <= 1;
  const online = inRoom.filter((m) => m.memberId === p.me?.memberId || m.path !== "offline").length;
  const myStacks = p.members.find((m) => m.memberId === p.me?.memberId)?.stacks ?? 0;
  const relaysDown =
    p.relays !== undefined && p.relays !== null && p.relays.length > 0 && !p.relays.some((r) => r.state === "ok");
  const subtitle =
    p.room.status === "joining"
      ? "正在等群友上线"
      : p.room.status === "dissolved"
        ? "群已解散"
        : p.room.status === "left"
          ? "你已退出这个群"
          : relaysDown
            ? "连不上中继，只能和同一局域网的群友聊天"
            : inRoom.length > 0
              ? `${inRoom.length} 位成员，${online} 人在线`
              : "";
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

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      p.onToast("已复制");
    } catch {
      p.onToast("复制失败");
    }
  }

  function toBottom() {
    const el = list.current;
    if (el) el.scrollTop = el.scrollHeight;
    setUnseen(0);
  }

  return (
    <main
      className="chat"
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
        <button className="head-main" onClick={p.onOpenDrawer} title="群资料">
          <RoomAvatar room={p.room} size={40} />
          <span className="head-text">
            <span className="head-title">{p.room.title ?? "同步中…"}</span>
            {subtitle && <span className={relaysDown ? "head-sub warn" : "head-sub"}>{subtitle}</span>}
          </span>
        </button>
        {myStacks > 0 && (
          <button className="stack-chip" onClick={p.onOpenDrawer} title="我在这个群叠了几层心之钢">
            <img src={heartsteel} alt="" />
            {myStacks}
          </button>
        )}
        <button className="icon-button" onClick={p.onOpenSearch} title="搜索聊天记录" aria-label="搜索聊天记录">
          <Icon name="search" size={20} />
        </button>
        <button className="icon-button" onClick={p.onOpenDrawer} title="群资料" aria-label="群资料">
          <Icon name="more" size={20} />
        </button>
      </header>

      <div className={dragging ? "chat-body dragging" : "chat-body"}>
        <div
          className="messages"
          ref={list}
          onScroll={(e) => {
            const el = e.currentTarget;
            atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
            if (atBottom.current) setUnseen(0);
          }}
        >
          <div className="messages-inner">
            {p.canLoadOlder && (
              <button className="load-older" onClick={() => p.onLoadOlder()}>
                查看更早的消息
              </button>
            )}
            {p.messages.length === 0 && canSend && (
              <div className="chat-empty">
                <img src={heartsteel} alt="" width={48} height={48} />
                <p>还没有消息。5 秒内连发 3 条，叠一层心之钢。</p>
              </div>
            )}
            {p.messages.map((m, i) => {
              const prev = p.messages[i - 1];
              const day = dayText(m.displayTs);
              const newDay = !prev || dayText(prev.displayTs) !== day;
              const separator = newDay && (
                <div className="day">
                  <span>{day}</span>
                </div>
              );
              if (m.kind === "notice") {
                return (
                  <div key={m.eventId} data-id={m.eventId}>
                    {separator}
                    <div className="notice">
                      <span>{noticeText(m, p.members)}</span>
                    </div>
                  </div>
                );
              }
              const author = member(m.authorId);
              const mine = m.authorId === p.me?.memberId;
              const lead =
                newDay || prev?.kind !== "chat" || prev.authorId !== m.authorId || m.displayTs - prev.displayTs >= RUN_MS;
              const flashing = p.highlight?.eventId === m.eventId;
              const skewed = Math.abs(m.tsMs - m.displayTs) > SKEW_MS;
              const meta = (
                <span className={m.image ? "meta on-image" : "meta"}>
                  {skewed && (
                    <span className="skew" title={`对方电脑上的时间是 ${timeText(m.tsMs)}，按收到的时间显示`}>
                      !
                    </span>
                  )}
                  {clockText(m.displayTs)}
                  {mine && !alone && m.delivery === "pending" && (
                    <span className="tick" title="发送中：还没有群友确认收到，会自动重发">
                      <Icon name="clock" size={13} />
                    </span>
                  )}
                  {mine && !alone && m.delivery === "sent" && (
                    <span className="tick sent" title="群友已收到">
                      <Icon name="checks" size={15} />
                    </span>
                  )}
                </span>
              );
              return (
                <div key={m.eventId} data-id={m.eventId}>
                  {separator}
                  <div
                    className={`msg ${mine ? "out" : "in"}${lead ? " lead" : ""}${flashing ? " flash-hl" : ""}`}
                    key={flashing ? p.highlight?.key : undefined}
                  >
                    {!mine && (
                      <span className="msg-avatar">
                        {lead && (
                          <Avatar
                            fingerprint={author?.fingerprint ?? m.authorId}
                            name={author?.nickname}
                            avatar={author?.avatar}
                            size={34}
                            stacks={author?.stacks ?? 0}
                            pulseKey={p.pulse?.memberId === m.authorId ? p.pulse.key : undefined}
                          />
                        )}
                      </span>
                    )}
                    <div className={m.image ? "bubble image" : "bubble"}>
                      {!mine && lead && (
                        <div className="sender" style={nameStyle(author?.fingerprint ?? m.authorId)}>
                          {displayName(m.authorId, p.members)}
                        </div>
                      )}
                      {m.reply && (
                        <button
                          className="quote"
                          style={m.reply.authorId ? nameStyle(member(m.reply.authorId)?.fingerprint ?? m.reply.authorId) : undefined}
                          onClick={() => p.onJump(m.reply!.eventId)}
                          title="跳到原消息"
                        >
                          <span className="quote-who">
                            {m.reply.authorId
                              ? m.reply.authorId === p.me?.memberId
                                ? "你"
                                : displayName(m.reply.authorId, p.members)
                              : "原消息"}
                          </span>
                          <span className="quote-text">{m.reply.excerpt}</span>
                        </button>
                      )}
                      {m.image ? (
                        <MessageImage roomId={p.room.roomId} eventId={m.eventId} info={m.image} onOpen={setViewing}>
                          {meta}
                        </MessageImage>
                      ) : (
                        <>
                          <span className="text">{withMentions(m.text ?? "", mentionedNames(m))}</span>
                          <span className={mine && !alone ? "meta-space wide" : "meta-space"} />
                          {meta}
                        </>
                      )}
                      {canSend && (
                        <button
                          className="bubble-menu-btn"
                          aria-label="更多操作"
                          onClick={() => setMenuFor(menuFor === m.eventId ? null : m.eventId)}
                        >
                          <Icon name="chevronDown" size={18} />
                        </button>
                      )}
                      {menuFor === m.eventId && (
                        <div className="menu bubble-menu" role="menu">
                          <button
                            role="menuitem"
                            onClick={() => {
                              setReplyTo(m);
                              setMenuFor(null);
                            }}
                          >
                            <Icon name="reply" size={17} />
                            回复
                          </button>
                          {m.text && (
                            <button
                              role="menuitem"
                              onClick={() => {
                                copy(m.text!);
                                setMenuFor(null);
                              }}
                            >
                              <Icon name="copy" size={17} />
                              复制
                            </button>
                          )}
                          {!mine && author?.inRoom && (
                            <button
                              role="menuitem"
                              onClick={() => {
                                setMentionRequest({ memberId: m.authorId, key: Date.now() });
                                setMenuFor(null);
                              }}
                            >
                              <Icon name="at" size={17} />
                              @TA
                            </button>
                          )}
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        </div>

        {unseen > 0 && (
          <button className="to-bottom" onClick={toBottom} aria-label="回到最新消息" title="回到最新消息">
            <Icon name="chevronDown" size={22} />
            <span className="to-bottom-count">{unseen > 99 ? "99+" : unseen}</span>
          </button>
        )}
        {p.flash && (
          <div key={p.flash.key} className="flash" role="status">
            <img src={heartsteel} alt="" />
            {p.flash.text}
          </div>
        )}
        {dragging && <div className="drop-hint">松开发送图片</div>}
      </div>

      {hint ? (
        <div className="hint">
          <span>{hint}</span>
          {p.room.status === "joining" && (
            <button className="link-button" onClick={p.onCancelJoin}>
              取消加入
            </button>
          )}
        </div>
      ) : (
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
      {viewing && <ImageViewer url={viewing} onClose={() => setViewing(null)} />}
    </main>
  );
}
