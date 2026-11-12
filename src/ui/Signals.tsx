// V2 信号页：只做最高频的两件事——看轮盘、换格子。其余尺寸和声音收进「设置」。
// 图标网格不放文字，名字用 title 悬停，符合「少文字多图标」。
import { useState } from "react";
import { previewPing, type Settings } from "../api";
import { ALL_PINGS, pingById, type PingId } from "../shared/pings";
import { pingIcon } from "../shared/vectorAssets";
import { Glyph } from "./Glyph";
import { WheelDial } from "./WheelDial";
import { Action, Row, Toggle } from "./controls";

const DIRS = ["上", "右上", "右", "右下", "下", "左下", "左", "左上"];

function placeOnSlot(wheel: readonly PingId[], id: PingId, slot: number): PingId[] {
  const next = [...wheel];
  const from = next.indexOf(id);
  if (from >= 0) [next[from], next[slot]] = [next[slot], next[from]];
  else next[slot] = id;
  return next;
}

export function Signals({ settings: s, onChange }: { settings: Settings; onChange: (patch: Partial<Settings>) => void }) {
  const [slot, setSlot] = useState(0);
  const selected = pingById(s.wheel[slot]);

  return (
    <div className="v-page">
      <div className="v-page-head">
        <Glyph name="target" size={26} />
        <h1>信号</h1>
        <span className="v-page-spacer" />
        <Action icon="play" label="预览" tone="ghost" onClick={() => selected && void previewPing(selected.id)} />
      </div>

      <div className="v-signal-grid">
        <section className="v-tile v-editor">
          <div className="v-mini-wheel">
            <svg viewBox="0 0 280 280" aria-hidden="true"><WheelDial /></svg>
            {s.wheel.slice(0, 8).map((id, i) => {
              const a = ((-90 + i * 45) * Math.PI) / 180;
              return (
                <button
                  key={i}
                  className={`v-mini-slot${i === slot ? " on" : ""}`}
                  style={{ left: `${50 + 34 * Math.cos(a)}%`, top: `${50 + 34 * Math.sin(a)}%` }}
                  onClick={() => setSlot(i)}
                  aria-label={`${DIRS[i]}：${pingById(id)?.name ?? id}`}
                  aria-pressed={i === slot}
                >
                  <img src={pingIcon(id, s.iconStyle)} alt="" />
                </button>
              );
            })}
            <span className="v-mini-core">ALT</span>
          </div>
          <div className="v-editor-side">
            <div className="v-editor-cur">
              <img src={pingIcon(s.wheel[slot], s.iconStyle)} alt="" />
              <b>{selected?.name}</b>
              <span>{DIRS[slot]}</span>
            </div>
            <div className="v-icon-grid">
              {ALL_PINGS.map((p) => (
                <button
                  key={p.id}
                  className={`v-icon-cell${p.id === s.wheel[slot] ? " on" : ""}`}
                  title={p.name}
                  onClick={() => onChange({ wheel: placeOnSlot(s.wheel, p.id, slot) })}
                >
                  <img src={pingIcon(p.id, s.iconStyle)} alt="" />
                </button>
              ))}
            </div>
          </div>
        </section>

        <div className="v-side-col">
          <section className="v-tile">
            <Row icon="mouse" label="单击信号">
              <Toggle label="单击信号" checked={s.clickPing} onChange={(clickPing) => onChange({ clickPing })} />
            </Row>
            {s.clickPing && (
              <div className="v-inline-pick">
                {ALL_PINGS.slice(0, 8).map((p) => (
                  <button
                    key={p.id}
                    className={`v-icon-cell sm${s.clickPingId === p.id ? " on" : ""}`}
                    title={p.name}
                    onClick={() => onChange({ clickPingId: p.id })}
                  >
                    <img src={pingIcon(p.id, s.iconStyle)} alt="" />
                  </button>
                ))}
              </div>
            )}
          </section>
          <section className="v-tile">
            <Row icon="monitor" label="全屏不拦截" title="游戏占着整块屏幕时，触发键加点击原样交给游戏">
              <Toggle label="全屏不拦截" checked={s.gameCheck} onChange={(gameCheck) => onChange({ gameCheck })} />
            </Row>
            <Row icon="volume" label="静音">
              <Toggle label="静音" checked={s.muted} onChange={(muted) => onChange({ muted })} />
            </Row>
          </section>
        </div>
      </div>
    </div>
  );
}
