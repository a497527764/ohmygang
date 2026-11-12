// 麦克风实时频谱：Rust 语音线程每 60 毫秒发一次各频段的幅度（voice://mic，24 个 0..1 的数），
// 每根柱子只按自己那一段频率起伏 —— 低频的人声、高频的齿音各自高低不同，不再是统一高度的包络。
// 语音关着、离线或麦克风没声音时，所有柱子停在基线。
import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Glyph } from "./Glyph";

const MIN_H = 4;
const BANDS = 24;
const IDLE: number[] = Array(BANDS).fill(0);

/** 后端固定 24 段，界面要画几根柱子就线性插值成几根（主页 17、设置页 56 都能用）。 */
function resample(bands: number[], bars: number): number[] {
  return Array.from({ length: bars }, (_, i) => {
    const at = (i / Math.max(1, bars - 1)) * (bands.length - 1);
    const lo = Math.floor(at);
    const hi = Math.min(bands.length - 1, lo + 1);
    return bands[lo] + (bands[hi] - bands[lo]) * (at - lo);
  });
}

export function MicWave({ live, bars = 22, max = 50, className = "", icon = false }: {
  live: boolean;
  bars?: number;
  max?: number;
  className?: string;
  /** 左边加一个小麦克风图标，说明这排柱子是什么。 */
  icon?: boolean;
}) {
  const [bands, setBands] = useState<number[]>(IDLE);
  const lastAt = useRef(0);
  useEffect(() => {
    setBands(IDLE);
    // 订阅不跟 live 绑定：开关切换时不用重新注册，后端一发就能动
    const un = listen<number[]>("voice://mic", (e) => {
      const raw = e.payload;
      // 兼容旧版单个音量的事件：整排按同一值兜底，不会报错
      const arr = Array.isArray(raw) ? raw : new Array(BANDS).fill(Number(raw) || 0);
      lastAt.current = Date.now();
      setBands(arr.slice(0, BANDS).map((v) => Math.min(1, Math.max(0, Number(v) || 0))));
    });
    // 后端停发（下线、关语音、麦被拔）时把各频段收回基线
    const stale = setInterval(() => {
      if (Date.now() - lastAt.current > 400) {
        setBands((v) => (v.some((x) => x > 0.02) ? v.map((x) => x * 0.7) : IDLE));
      }
    }, 120);
    return () => {
      void un.then((f) => f());
      clearInterval(stale);
    };
  }, []);

  const values = resample(bands, bars);
  return (
    <span className={`mic-wave${live ? "" : " mic-wave-off"} ${className}`} aria-hidden="true">
      {icon && <Glyph name="mic" size={13} />}
      {values.map((v, i) => (
        <i key={i} style={{ height: `${MIN_H + v * (max - MIN_H)}px` }} />
      ))}
    </span>
  );
}
