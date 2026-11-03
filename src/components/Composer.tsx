// 输入区（V1.1）：表情、发图片、引用回复、@ 选人、多行输入。
// Enter 发送，Shift+Enter 换行；输入「@」弹出群成员，选中后插入「@昵称 」并记下这个人，发送时文字里还留着的才算 @。
// 粘贴或拖进来的图片直接发送（压缩在 App 里做）。
import { useEffect, useRef, useState } from "react";
import type { Identity, Member, Message } from "../api";
import { EMOJI } from "../emoji";
import { firstImage } from "../image";
import { displayName } from "../names";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";

const MAX_BYTES = 900;
const encoder = new TextEncoder();

interface Props {
  roomId: string;
  me: Identity | null;
  members: Member[];
  enabled: boolean;
  replyTo: Message | null;
  onCancelReply: () => void;
  /// 成功返回 true，输入框随之清空。
  onSend: (text: string, replyTo: string | null, mentions: string[]) => Promise<boolean>;
  onSendImage: (file: Blob) => Promise<boolean>;
  /// 别处要求插入 @某人（消息上的「@TA」）。
  mentionRequest: { memberId: string; key: number } | null;
}

interface Picker {
  /// 「@」在文字里的位置。
  at: number;
  query: string;
  index: number;
}

export function Composer(p: Props) {
  const [draft, setDraft] = useState("");
  const [picked, setPicked] = useState<Map<string, string>>(new Map());
  const [picker, setPicker] = useState<Picker | null>(null);
  const [emojiOpen, setEmojiOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const file = useRef<HTMLInputElement>(null);
  const emojiRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setDraft("");
    setPicked(new Map());
    setPicker(null);
    input.current?.focus();
  }, [p.roomId]);

  useEffect(() => {
    if (p.replyTo) input.current?.focus();
  }, [p.replyTo]);

  // 输入框随内容长高，最多 6 行
  useEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = "auto";
    // 边框不算在 scrollHeight 里，要加回来，否则总差 2 像素、出现滚动条
    const full = el.scrollHeight + 2;
    el.style.height = `${Math.min(full, 132)}px`;
    el.style.overflowY = full > 132 ? "auto" : "hidden";
  }, [draft]);

  useEffect(() => {
    if (!emojiOpen) return;
    const close = (e: MouseEvent) => {
      if (!emojiRef.current?.contains(e.target as Node)) setEmojiOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [emojiOpen]);

  const nameOf = (m: Member) => m.nickname || m.fingerprint.slice(0, 6);
  const candidates = picker
    ? p.members
        .filter((m) => m.inRoom && m.memberId !== p.me?.memberId)
        .filter((m) => {
          const q = picker.query.toLowerCase();
          return !q || nameOf(m).toLowerCase().includes(q) || m.fingerprint.toLowerCase().startsWith(q);
        })
        .slice(0, 8)
    : [];

  /// 在光标处插入文字。
  function insert(text: string) {
    const el = input.current;
    const start = el?.selectionStart ?? draft.length;
    const end = el?.selectionEnd ?? draft.length;
    const next = draft.slice(0, start) + text + draft.slice(end);
    setDraft(next);
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(start + text.length, start + text.length);
    });
  }

  function mention(m: Member, at: number | null) {
    const name = nameOf(m);
    setPicked((old) => new Map(old).set(m.memberId, name));
    if (at === null) {
      insert(`@${name} `);
    } else {
      const caret = input.current?.selectionStart ?? draft.length;
      const next = `${draft.slice(0, at)}@${name} ${draft.slice(caret)}`;
      setDraft(next);
      const pos = at + name.length + 2;
      requestAnimationFrame(() => {
        input.current?.focus();
        input.current?.setSelectionRange(pos, pos);
      });
    }
    setPicker(null);
  }

  useEffect(() => {
    const req = p.mentionRequest;
    if (!req) return;
    const m = p.members.find((x) => x.memberId === req.memberId);
    if (m) mention(m, null);
    // 只在收到新的请求时插入，所以只依赖 mentionRequest
  }, [p.mentionRequest]);

  function onChange(value: string, caret: number) {
    setDraft(value);
    const before = value.slice(0, caret);
    const m = /@([^\s@]{0,16})$/.exec(before);
    setPicker(m ? { at: caret - m[0].length, query: m[1], index: 0 } : null);
  }

  const bytes = encoder.encode(draft).length;
  const tooLong = bytes > MAX_BYTES;

  async function submit() {
    const text = draft.trim();
    if (!text || !p.enabled || busy || tooLong) return;
    const mentions = [...picked].filter(([, name]) => text.includes(`@${name}`)).map(([id]) => id);
    setBusy(true);
    const ok = await p.onSend(text, p.replyTo?.eventId ?? null, mentions);
    setBusy(false);
    if (ok) {
      setDraft("");
      setPicked(new Map());
      setPicker(null);
    }
    input.current?.focus();
  }

  async function sendFile(f: Blob | null | undefined) {
    if (!f || !p.enabled) return;
    setBusy(true);
    await p.onSendImage(f);
    setBusy(false);
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing) return;
    if (picker && candidates.length > 0) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const step = e.key === "ArrowDown" ? 1 : -1;
        setPicker({ ...picker, index: (picker.index + step + candidates.length) % candidates.length });
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        mention(candidates[picker.index] ?? candidates[0], picker.at);
        return;
      }
    }
    if (e.key === "Escape") {
      if (picker) setPicker(null);
      else if (p.replyTo) p.onCancelReply();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  return (
    <div className="composer">
      {p.replyTo && (
        <div className="reply-bar">
          <Icon name="reply" size={14} />
          <span className="reply-bar-text">
            回复 {displayName(p.replyTo.authorId, p.members)}：
            {p.replyTo.image ? "[图片]" : p.replyTo.text}
          </span>
          <button className="icon-button small" onClick={p.onCancelReply} aria-label="取消回复" title="取消回复">
            <Icon name="close" size={14} />
          </button>
        </div>
      )}
      {picker && candidates.length > 0 && (
        <ul className="mention-picker" role="listbox" aria-label="选择要 @ 的人">
          {candidates.map((m, i) => (
            <li key={m.memberId}>
              <button
                role="option"
                aria-selected={i === picker.index}
                className={i === picker.index ? "active" : undefined}
                onMouseDown={(e) => {
                  e.preventDefault();
                  mention(m, picker.at);
                }}
              >
                <Avatar fingerprint={m.fingerprint} name={m.nickname} avatar={m.avatar} size={24} />
                <span>{nameOf(m)}</span>
                <span className="mention-fp">{m.fingerprint.slice(0, 6)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="composer-tools">
        <div className="menu-anchor" ref={emojiRef}>
          <button
            className="icon-button"
            disabled={!p.enabled}
            onClick={() => setEmojiOpen((o) => !o)}
            title="表情"
            aria-label="表情"
            aria-expanded={emojiOpen}
          >
            <Icon name="smile" size={20} />
          </button>
          {emojiOpen && (
            <div className="emoji-panel" role="dialog" aria-label="表情">
              {EMOJI.map((e) => (
                <button
                  key={e}
                  onClick={() => {
                    insert(e);
                    setEmojiOpen(false);
                  }}
                >
                  {e}
                </button>
              ))}
            </div>
          )}
        </div>
        <button
          className="icon-button"
          disabled={!p.enabled || busy}
          onClick={() => file.current?.click()}
          title="发图片（也可以直接粘贴或拖进来）"
          aria-label="发图片"
        >
          <Icon name="image" size={20} />
        </button>
        <input
          ref={file}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif,image/bmp"
          hidden
          onChange={(e) => {
            sendFile(e.currentTarget.files?.[0]);
            e.currentTarget.value = "";
          }}
        />
        <button
          className="icon-button"
          disabled={!p.enabled}
          onClick={() => {
            insert("@");
            const caret = (input.current?.selectionStart ?? draft.length) + 1;
            setPicker({ at: caret - 1, query: "", index: 0 });
          }}
          title="@ 某人"
          aria-label="@ 某人"
        >
          <Icon name="at" size={19} />
        </button>
        {bytes > MAX_BYTES - 150 && (
          <span className={tooLong ? "byte-count over" : "byte-count"}>
            {tooLong ? "太长了，" : ""}
            {bytes}/{MAX_BYTES} 字节
          </span>
        )}
      </div>
      <div className="composer-row">
        <textarea
          ref={input}
          rows={1}
          value={draft}
          onChange={(e) => onChange(e.currentTarget.value, e.currentTarget.selectionStart)}
          onKeyDown={onKeyDown}
          onPaste={(e) => {
            const img = firstImage(e.clipboardData?.items);
            if (img) {
              e.preventDefault();
              sendFile(img);
            }
          }}
          placeholder={p.enabled ? "说点什么，Enter 发送，Shift+Enter 换行" : ""}
          disabled={!p.enabled}
          aria-label="输入消息"
        />
        <button className="primary send" disabled={!p.enabled || busy || !draft.trim() || tooLong} onClick={submit}>
          发送
        </button>
      </div>
    </div>
  );
}
