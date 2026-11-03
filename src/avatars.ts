// 头像相关的界面数据（V1.1）：内置头像的图标和底色、自动头像的色相。
// 内置头像在网络上只传「第几个图标、第几种底色」，所以这两个列表只能在末尾追加，已有的顺序不能改
// （Rust 侧 AVATAR_PRESET_ICONS = 48、AVATAR_PRESET_COLORS = 12 限定了范围，追加时两边一起改）。

export const PRESET_ICONS = [
  "🐱", "🐶", "🦊", "🐼", "🐨", "🐯", "🦁", "🐸",
  "🐵", "🐰", "🐻", "🐷", "🐮", "🐔", "🐧", "🦄",
  "🐲", "🦖", "🐙", "🦈", "🐳", "🦉", "🐝", "🦋",
  "⚔️", "🛡️", "🏹", "🔥", "⚡", "❄️", "🌙", "⭐",
  "🍀", "🌸", "🍑", "🍉", "🍓", "🍔", "🍜", "☕",
  "🎮", "🎧", "🎲", "🏀", "⚽", "👑", "💎", "🚀",
] as const;

/// 内置头像的底色：浅色为主，最后一个是深色。
export const PRESET_COLORS = [
  "#fde2e4", "#ffe5d0", "#fff1bf", "#e3f4d2", "#cdeee0", "#d2ecf6",
  "#dbe5fb", "#e6dffb", "#f6dcf0", "#ece7df", "#e2e7e9", "#2f3e3a",
] as const;

/// 指纹或房间 ID → 色相（0–359）。各端算法相同，同一个人在所有人那里颜色一样。
export function hueOf(seed: string): number {
  let h = 2166136261;
  for (const ch of seed) {
    h ^= ch.charCodeAt(0);
    h = Math.imul(h, 16777619);
  }
  return (h >>> 0) % 360;
}

/// 昵称第一个字（按字符算，表情也算一个）；没昵称时用指纹前两位。
export function initialOf(name: string | null | undefined, fallback: string): string {
  const first = Array.from((name ?? "").trim())[0];
  return first ?? fallback.slice(0, 2).toUpperCase();
}

/// 群聊里每个人名字的颜色：按指纹算色相，浅色背景上用深一点，深色背景上用亮一点（由 CSS 的 --name-l 决定）。
export function nameStyle(fingerprint: string): Record<string, string | number> {
  return { ["--h"]: hueOf(fingerprint) };
}
