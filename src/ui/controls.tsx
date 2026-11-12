// V2 设置控件：一行只有一个图标、一个短标签、一个操作，不再写解释句。
// 需要解释的地方用 title 悬停提示，界面上不占位置。
import { useState, type CSSProperties, type ReactNode } from "react";
import { Glyph, type GlyphName } from "./Glyph";

export function Row({
  icon,
  label,
  title,
  children,
}: {
  icon?: GlyphName;
  label: string;
  title?: string;
  children: ReactNode;
}) {
  return (
    <div className="v-row" title={title}>
      <span className="v-row-name">
        {icon && <Glyph name={icon} size={20} />}
        <b>{label}</b>
      </span>
      <span className="v-row-ctl">{children}</span>
    </div>
  );
}

export function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (on: boolean) => void; label: string }) {
  return (
    <input
      type="checkbox"
      role="switch"
      className="v2-switch"
      aria-label={label}
      checked={checked}
      onChange={(e) => onChange(e.target.checked)}
    />
  );
}

export function Range(props: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  onChange: (v: number) => void;
}) {
  const { label, value, min, max, step = 1, unit = "", onChange } = props;
  const fill = { "--fill": `${((value - min) / (max - min)) * 100}%` } as CSSProperties;
  return (
    <>
      <input
        type="range"
        className="v2-range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        style={fill}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <output className="v-value">
        {step < 1 ? value.toFixed(1) : value}
        {unit}
      </output>
    </>
  );
}

export function Choice<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div className="v-choice" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          className={value === o.value ? "on" : ""}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** 默认折叠的低频设置：标题只用图标 + 2~4 字，展开后才出现具体项。 */
export function Fold({
  icon,
  label,
  count,
  children,
  defaultOpen = false,
}: {
  icon: GlyphName;
  label: string;
  count?: number;
  children: ReactNode;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <section className={`v-fold${open ? " open" : ""}`}>
      <button type="button" className="v-fold-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Glyph name={icon} size={22} />
        <b>{label}</b>
        {count !== undefined && <span className="v-fold-count">{count}</span>}
        <span className="v-fold-arrow">
          <Glyph name="chevron" size={20} />
        </span>
      </button>
      {open && <div className="v-fold-body">{children}</div>}
    </section>
  );
}

export function Action({ icon, label, onClick, tone = "solid", disabled }: {
  icon?: GlyphName;
  label: string;
  onClick: () => void;
  tone?: "solid" | "ghost" | "danger";
  disabled?: boolean;
}) {
  return (
    <button type="button" className={`v-btn ${tone}`} onClick={onClick} disabled={disabled}>
      {icon && <Glyph name={icon} size={18} />}
      {label}
    </button>
  );
}
