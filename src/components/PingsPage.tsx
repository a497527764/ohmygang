// 「信号」页：开关、轮盘编辑、手势、大小和声音。按键在「快捷键」页。改动交给 App 统一保存。
import type { Settings, Status } from "../api";
import { hotkeyParts } from "../shared/keys";
import { ALL_PINGS, type PingId } from "../shared/pings";
import { Row, Slider, Switch } from "./controls";
import { triggerLabel } from "./ShortcutsPage";
import { WheelEditor } from "./WheelEditor";

interface Props {
  settings: Settings;
  status: Status;
  onChange: (patch: Partial<Settings>) => void;
  onToggle: (on: boolean) => void;
}

export function PingsPage({ settings: s, status, onChange, onToggle }: Props) {
  return (
    <div className="page">
      <h1>信号</h1>

      {status.inputError ? (
        <p className="banner error">键鼠监听没有启动：{status.inputError}</p>
      ) : (
        <section className={`hero${status.enabled ? " on" : ""}`}>
          <div>
            <b>{status.enabled ? "信号开启中" : "信号已暂停"}</b>
            <p>
              <kbd>{triggerLabel(s.trigger)}</kbd>+ 拖动
              <span className="muted">　暂停</span>
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
        <h2>手势</h2>
        <Slider
          label="拖多远才打开轮盘"
          value={s.dragThresholdPx}
          min={2}
          max={40}
          step={1}
          unit=" 像素"
          onChange={(v) => onChange({ dragThresholdPx: v })}
        />
        <Row label="单击也放信号" tip="关着时，触发键加单击照常交给下面的程序">
          <Switch label="单击也放信号" checked={s.clickPing} onChange={(clickPing) => onChange({ clickPing })} />
          {s.clickPing && (
            <select aria-label="单击放的信号" value={s.clickPingId} onChange={(e) => onChange({ clickPingId: e.target.value as PingId })}>
              {ALL_PINGS.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          )}
        </Row>
        <Row label="全屏游戏里不拦截" tip="游戏占着整块屏幕时，触发键加点击原样交给游戏">
          <Switch label="全屏游戏里不拦截" checked={s.gameCheck} onChange={(gameCheck) => onChange({ gameCheck })} />
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
        <Row label="轮盘刻度声" tip="拖过每一格时轻响一下">
          <Switch label="轮盘刻度声" checked={s.tickSound} onChange={(tickSound) => onChange({ tickSound })} />
        </Row>
      </section>
    </div>
  );
}
