// 表情的地址和名字：内置表情在 public/lol/emotes/，导入的和收到的队友表情经 Rust 的 emote:// 协议取。
// 规则（引用的形状、轮盘合法性、导入检查）在 Rust 的 emotes 模块，这里只拼地址。参考 lolPing src/shared/emotes.ts（MIT，© 2026 Danrui Meng）。
import { convertFileSrc } from "@tauri-apps/api/core";
import { EMOTE_CATALOG } from "./emoteCatalog";

/** 表情在屏幕上停留的秒数。 */
export const EMOTE_DURATION_S = 3;
export const MAX_CUSTOM_EMOTES = 20;

/** 和 Rust 的 emotes::CustomEmote 对应。 */
export interface CustomEmote {
  id: string;
  name: string;
  animated: boolean;
  sound: boolean;
  /** 出场效果 0–5。 */
  fx: number;
}

/** 出场效果的名字，下标就是效果号。 */
export const FX_NAMES = ["弹出", "旋转", "落下弹跳", "闪现", "摆荡", "心跳"] as const;

export const isBundled = (ref: string): boolean => EMOTE_CATALOG.some((e) => e.slug === ref);
/** `c:` 加 32 位十六进制里的哈希。 */
export const customHash = (ref: string): string | null => /^c:([0-9a-f]{32})$/.exec(ref)?.[1] ?? null;

export const bundledIconUrl = (slug: string): string => `/lol/emotes/${slug}.png`;
export const bundledAnimUrl = (slug: string): string => `/lol/emotes/${slug}.webp`;
export const bundledSoundUrl = (slug: string): string => `/lol/emotes/${slug}.ogg`;
/** 导入或收到的表情文件：webp 是静态图，anim 是动画（没有就给静态图），sound 是音效。 */
export const customUrl = (hash: string, part: "webp" | "anim" | "sound"): string => convertFileSrc(`${hash}.${part}`, "emote");

/** 轮盘和列表里的小图。 */
export function emoteIcon(ref: string): string {
  const hash = customHash(ref);
  return hash ? customUrl(hash, "webp") : bundledIconUrl(ref);
}

export function emoteName(ref: string, custom: readonly CustomEmote[]): string {
  const c = custom.find((x) => x.id === ref);
  if (c) return c.name;
  const e = EMOTE_CATALOG.find((x) => x.slug === ref);
  return e ? e.nameZh : "未知表情";
}
