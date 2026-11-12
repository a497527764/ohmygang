// 图标风格的数据流：设置里的 iconStyle（league / vector）由 Rust 保存并广播，
// 控制面板、透明浮层、聊天记录都调下面两个函数取图，保证一处切换全窗口生效。
// league 用项目内置的英雄联盟原图，vector 用 public/vector 下的加粗几何图。
import { bundledIconUrl, emoteIcon } from "./emotes";
import { pingById, textureUrl, FALLBACK_TEXTURE } from "./pings";

export type IconStyle = "league" | "vector";

const VECTOR_PINGS = [
  "danger", "push", "omw", "allin", "assist", "needvision",
  "missing", "enemyvision", "generic", "bait", "visioncleared", "missingflip",
] as const;

const VECTOR_EMOTES = 10;

/** 同一个表情每次都落到同一张矢量脸，不会切一次变一次。 */
function stableIndex(ref: string): number {
  let h = 0;
  for (let i = 0; i < ref.length; i++) h = (h * 31 + ref.charCodeAt(i)) >>> 0;
  return h % VECTOR_EMOTES;
}

export const vectorPingUrl = (id: string): string =>
  `/vector/pings/${VECTOR_PINGS.includes(id as (typeof VECTOR_PINGS)[number]) ? id : "generic"}.svg`;

export const vectorEmoteUrl = (ref: string): string =>
  `/vector/emotes/${String(stableIndex(ref) + 1).padStart(2, "0")}.svg`;

/** 信号图：轮盘、聊天记录、设置里的预览统一走这里。 */
export function pingIcon(id: string, style: IconStyle): string {
  return style === "vector" ? vectorPingUrl(id) : textureUrl(pingById(id)?.icon ?? FALLBACK_TEXTURE);
}

/** 表情图：内置表情有两套；导入的表情只有用户自己的原图，矢量风格不改动它。 */
export function emoteIconStyled(ref: string, style: IconStyle): string {
  if (ref.startsWith("c:")) return emoteIcon(ref);
  return style === "vector" ? vectorEmoteUrl(ref) : (bundledIconUrl(ref) || emoteIcon(ref));
}
