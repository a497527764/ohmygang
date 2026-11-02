// 输入一行文字的小对话框，代替浏览器自带的 prompt（建房起名、粘贴房间码、改昵称）。
// Enter 确认，Esc 或点遮罩取消；确认失败时把 Rust 返回的中文错误显示在输入框下面，不关闭。
import { useEffect, useRef, useState } from "react";

interface Props {
  title: string;
  confirm: string;
  placeholder?: string;
  initial?: string;
  maxLength?: number;
  /// 返回 Promise；reject 时显示错误、保持打开。
  onConfirm: (value: string) => Promise<void>;
  onCancel: () => void;
}

export function Dialog({ title, confirm, placeholder, initial = "", maxLength, onConfirm, onCancel }: Props) {
  const [value, setValue] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.select();
  }, []);

  async function submit() {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await onConfirm(value);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <div className="overlay" onMouseDown={onCancel}>
      <form
        className="dialog"
        role="dialog"
        aria-label={title}
        onMouseDown={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
      >
        <h2>{title}</h2>
        <input
          ref={input}
          autoFocus
          value={value}
          placeholder={placeholder}
          maxLength={maxLength}
          onChange={(e) => setValue(e.currentTarget.value)}
        />
        {error && <p className="field-error">{error}</p>}
        <div className="dialog-actions">
          <button type="button" className="ghost" onClick={onCancel}>
            取消
          </button>
          <button type="submit" className="primary" disabled={busy || !value.trim()}>
            {confirm}
          </button>
        </div>
      </form>
    </div>
  );
}
