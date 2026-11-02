// 心之钢规则的文字说明。
import type { RoomRule } from "./api";

function duration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} 秒`;
  const m = Math.floor(s / 60);
  return s % 60 ? `${m} 分 ${s % 60} 秒` : `${m} 分钟`;
}

export function ruleText(r: RoomRule): string {
  return `${duration(r.windowMs)}内连发 ${r.threshold} 条叠一层，之后冷却 ${duration(r.cooldownMs)}`;
}
