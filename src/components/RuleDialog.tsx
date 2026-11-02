// 房主改心之钢规则（需求 3.4）：窗口 1–60 秒、条数 2–10、冷却 1 秒–1 小时。改了只影响以后的消息。
import { useState } from "react";
import type { RoomRule } from "../api";

interface Props {
  rule: RoomRule;
  onConfirm: (rule: { windowMs: number; threshold: number; cooldownMs: number }) => Promise<void>;
  onCancel: () => void;
}

export function RuleDialog({ rule, onConfirm, onCancel }: Props) {
  const [windowS, setWindowS] = useState(String(rule.windowMs / 1000));
  const [threshold, setThreshold] = useState(String(rule.threshold));
  const [cooldownS, setCooldownS] = useState(String(rule.cooldownMs / 1000));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const w = Number(windowS);
  const n = Number(threshold);
  const c = Number(cooldownS);
  const problem =
    !(w >= 1 && w <= 60)
      ? "时间窗口要在 1 到 60 秒之间"
      : !(Number.isInteger(n) && n >= 2 && n <= 10)
        ? "条数要在 2 到 10 之间"
        : !(c >= 1 && c <= 3600)
          ? "冷却要在 1 秒到 1 小时之间"
          : null;

  async function submit() {
    if (problem || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onConfirm({ windowMs: Math.round(w * 1000), threshold: n, cooldownMs: Math.round(c * 1000) });
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
        aria-label="修改心之钢规则"
        onMouseDown={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
      >
        <h2>修改心之钢规则</h2>
        <div className="rule-fields">
          <label>
            <input type="number" min={1} max={60} value={windowS} onChange={(e) => setWindowS(e.currentTarget.value)} autoFocus />
            <span>秒内</span>
          </label>
          <label>
            <span>连发</span>
            <input type="number" min={2} max={10} value={threshold} onChange={(e) => setThreshold(e.currentTarget.value)} />
            <span>条叠一层</span>
          </label>
          <label>
            <span>之后冷却</span>
            <input type="number" min={1} max={3600} value={cooldownS} onChange={(e) => setCooldownS(e.currentTarget.value)} />
            <span>秒</span>
          </label>
        </div>
        <p className="dialog-text">新规则只影响以后发的消息，已经叠的层数不变。所有群友都会看到这次修改。</p>
        {(problem || error) && <p className="field-error">{problem ?? error}</p>}
        <div className="dialog-actions">
          <button type="button" className="ghost" onClick={onCancel}>
            取消
          </button>
          <button type="submit" className="primary" disabled={busy || !!problem}>
            保存规则
          </button>
        </div>
      </form>
    </div>
  );
}
