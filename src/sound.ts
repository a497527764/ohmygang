// 心之钢音效（内置）。只在收到 Rust 的实时叠层通知时播放：补齐回来的历史叠层不会发这个通知（架构 5.6）。
// 每次新建一个播放器，两人几乎同时叠层时两声可以重叠，不会互相打断。
// 是否播放由调用方按「本机音效开关、房间免打扰」决定，这里只管音量。
import heartsteelSound from "./assets/heartsteel.mp3";

/// volume：0–100。
export function playHeartsteel(volume: number) {
  if (volume <= 0) return;
  const audio = new Audio(heartsteelSound);
  audio.volume = Math.min(1, volume / 100);
  audio.play().catch((e) => console.warn("心之钢音效播放失败", e));
}
