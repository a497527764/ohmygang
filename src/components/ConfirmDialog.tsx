// 需要二次确认的操作（退出、解散、取消加入）：说清后果，确认按钮写明要做的事。
import { useState } from "react";

interface Props {
  title: string;
  message: string;
  confirm: string;
  danger?: boolean;
  onConfirm: () => Promise<void>;
  onCancel: () => void;
}

export function ConfirmDialog({ title, message, confirm, danger, onConfirm, onCancel }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="overlay" onMouseDown={onCancel}>
      <div
        className="dialog"
        role="alertdialog"
        aria-label={title}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
      >
        <h2>{title}</h2>
        <p className="dialog-text">{message}</p>
        {error && <p className="field-error">{error}</p>}
        <div className="dialog-actions">
          <button className="ghost" onClick={onCancel} autoFocus>
            取消
          </button>
          <button
            className={danger ? "primary danger" : "primary"}
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                await onConfirm();
              } catch (e) {
                setError(String(e));
                setBusy(false);
              }
            }}
          >
            {confirm}
          </button>
        </div>
      </div>
    </div>
  );
}
