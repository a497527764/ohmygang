// 设置页共用的小控件：一行设置、开关、滑条、ⓘ 提示。
// 页面上每项只写名字；要解释的话放进 ⓘ（悬停或用 Tab 移到上面时显示），状态文字（比如「已关」）用 hint。
import { useId, type CSSProperties, type ReactNode } from "react";

export function Tip({ text }: { text: string }) {
  const id = useId();
  return (
    <span className="tip">
      <button type="button" className="tip-btn" aria-label="说明" aria-describedby={id}>
        i
      </button>
      <span role="tooltip" id={id} className="tip-body">
        {text}
      </span>
    </span>
  );
}

export function Row({ label, tip, hint, children }: { label: string; tip?: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="row">
      <div className="row-text">
        <span className="row-label">
          {label}
          {tip && <Tip text={tip} />}
        </span>
        {hint && <small className="hint">{hint}</small>}
      </div>
      <div className="row-control">{children}</div>
    </div>
  );
}

export function Switch({ checked, onChange, label }: { checked: boolean; onChange: (on: boolean) => void; label: string }) {
  return <input type="checkbox" role="switch" aria-label={label} checked={checked} onChange={(e) => onChange(e.target.checked)} />;
}

export function Slider(props: { label: string; value: number; min: number; max: number; step: number; unit: string; onChange: (v: number) => void }) {
  const { label, value, min, max, step, unit, onChange } = props;
  // 已拖过的一段画亮（CSS 里的 --fill）
  const fill = { "--fill": `${((value - min) / (max - min)) * 100}%` } as CSSProperties;
  return (
    <Row label={label}>
      <input
        type="range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        style={fill}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <output className="value">
        {step < 1 ? value.toFixed(1) : value}
        {unit}
      </output>
    </Row>
  );
}
