// 「信号」页：开关、轮盘编辑、触发方式、大小和声音。改动交给 App 统一保存。
import type { ReactNode } from "react";
import type { NamedTrigger, Settings, Status, TriggerKey } from "../api";
import { hotkeyParts } from "../shared/keys";
import { ALL_PINGS, type PingId } from "../shared/pings";
import { KeyCapture } from "./KeyCapture";
import { WheelEditor } from "./WheelEditor";

const TRIGGERS: { value: NamedTrigger; label: string }[] = [
  { value: "alt", label: "Alt" },
  { value: "ctrl", label: "Ctrl" },
  { value: "shift", label: "Shift" },
  { value: "win", label: "Win" },
  { value: "capslock", label: "Caps Lock" },
  { value: "mouse4", label: "鼠标侧键（后退）" },
  { value: "mouse5", label: "鼠标侧键（前进）" },
];
const CUSTOM = "custom";

/** 「按住 Alt 用左键拖动」这样的一句用法说明。 */
function howTo(t: TriggerKey): string {
  if (t === "mouse4" || t === "mouse5") return "按住鼠标侧键拖动";
  const name = typeof t === "object" ? `${hotkeyParts({ mods: 0, vk: t.vk })[0]} 键` : TRIGGERS.find((x) => x.value === t)?.label;
  return `按住 ${name} 用左键拖动`;
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="row">
      <div className="row-text">
        <span>{label}</span>
        {hint && <small className="hint">{hint}</small>}
      </div>
      <div className="row-control">{children}</div>
    </div>
  );
}

function Switch({ checked, onChange, label }: { checked: boolean; onChange: (on: boolean) => void; label: string }) {
  return <input type="checkbox" role="switch" aria-label={label} checked={checked} onChange={(e) => onChange(e.target.checked)} />;
}

function Slider(props: { label: string; value: number; min: number; max: number; step: number; unit: string; onChange: (v: number) => void }) {
  const { label, value, min, max, step, unit, onChange } = props;
  return (
    <Row label={label}>
      <input
        type="range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <output className="value">
        {step < 1 ? value.toFixed(1) : value}
        {unit}
      </output>
    </Row>
  );
}

interface Props {
  settings: Settings;
  status: Status;
  onChange: (patch: Partial<Settings>) => void;
  onToggle: (on: boolean) => void;
}

export function PingsPage({ settings: s, status, onChange, onToggle }: Props) {
  const custom = typeof s.trigger === "object";
  return (
    <div className="page">
      <h1>信号</h1>

      {status.inputError ? (
        <p className="banner error">键鼠监听没有启动：{status.inputError}。轮盘暂时用不了。</p>
      ) : (
        <section className={`hero${status.enabled ? " on" : ""}`}>
          <div>
            <b>{status.enabled ? "信号开启中" : "信号已暂停"}</b>
            <p>
              {howTo(s.trigger)}，在按下的位置打开轮盘，松开放信号。暂停或恢复：
              {hotkeyParts(s.toggleHotkey).map((p) => (
                <kbd key={p}>{p}</kbd>
              ))}
            </p>
          </div>
          <Switch label="信号开关" checked={status.enabled} onChange={onToggle} />
        </section>
      )}

      <section className="card">
        <h2>轮盘</h2>
        <WheelEditor wheel={s.wheel} onChange={(wheel) => onChange({ wheel })} />
      </section>

      <section className="card">
        <h2>触发</h2>
        <Row label="触发键" hint="只拦拖动这个手势，别的点击和按键照常">
          <select
            value={custom ? CUSTOM : (s.trigger as NamedTrigger)}
            onChange={(e) => {
              const v = e.target.value;
              if (v === CUSTOM) onChange({ trigger: { vk: 0x56 } }); // 先给个 V，用户再录
              else onChange({ trigger: v as NamedTrigger });
            }}
          >
            {TRIGGERS.map((t) => (
              <option key={t.value} value={t.value}>
                {t.label}
              </option>
            ))}
            <option value={CUSTOM}>自定义按键…</option>
          </select>
          {typeof s.trigger === "object" && (
            <KeyCapture kind="key" value={s.trigger.vk} onPick={(vk) => onChange({ trigger: { vk } })} />
          )}
        </Row>
        <Slider
          label="拖多远才打开轮盘"
          value={s.dragThresholdPx}
          min={2}
          max={40}
          step={1}
          unit=" 像素"
          onChange={(v) => onChange({ dragThresholdPx: v })}
        />
        <Row label="单击也放信号" hint="关着时，触发键加单击照常交给下面的程序">
          <Switch label="单击也放信号" checked={s.clickPing} onChange={(clickPing) => onChange({ clickPing })} />
          {s.clickPing && (
            <select value={s.clickPingId} onChange={(e) => onChange({ clickPingId: e.target.value as PingId })}>
              {ALL_PINGS.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          )}
        </Row>
        <Row label="全屏游戏里不拦截" hint="游戏占着整块屏幕时，触发键加点击原样交给游戏">
          <Switch label="全屏游戏里不拦截" checked={s.gameCheck} onChange={(gameCheck) => onChange({ gameCheck })} />
        </Row>
        <Row label="暂停快捷键" hint="随时停掉或恢复全部拦截">
          <KeyCapture kind="hotkey" value={s.toggleHotkey} onPick={(toggleHotkey) => onChange({ toggleHotkey })} />
        </Row>
      </section>

      <section className="card">
        <h2>大小和声音</h2>
        <Slider label="轮盘大小" value={s.wheelSizePx} min={300} max={900} step={10} unit=" 像素" onChange={(v) => onChange({ wheelSizePx: v })} />
        <Slider label="信号大小" value={s.pingSizePx} min={60} max={220} step={1} unit=" 像素" onChange={(v) => onChange({ pingSizePx: v })} />
        <Slider
          label="信号停留"
          value={s.pingDurationS}
          min={1}
          max={8}
          step={0.1}
          unit=" 秒"
          onChange={(v) => onChange({ pingDurationS: v })}
        />
        <Slider label="音量" value={s.volume} min={0} max={100} step={1} unit="" onChange={(v) => onChange({ volume: v })} />
        <Row label="静音">
          <Switch label="静音" checked={s.muted} onChange={(muted) => onChange({ muted })} />
        </Row>
        <Row label="轮盘刻度声" hint="拖过每一格时轻响一下">
          <Switch label="轮盘刻度声" checked={s.tickSound} onChange={(tickSound) => onChange({ tickSound })} />
        </Row>
      </section>
    </div>
  );
}
