// 信号动画：落点一个光圈、几道扩散的环、弹出的图标，到时自己删掉。
// 移植自 lolPing src/renderer/overlay/pingFx.ts（MIT，© 2026 Danrui Meng），去掉了 AI 队友用的文字。
import { FALLBACK_TEXTURE, textureUrl, type PingDef } from "../shared/pings";

/** 屏幕上的信号超过这么多就先删最早的，免得一阵刷屏拖慢透明窗口。 */
export const MAX_PINGS = 60;

/** 队友信号下面的名字标签。 */
export interface PingTag {
  name: string;
  color: string;
}

export interface PingFxOptions {
  sizePx: number;
  durationS: number;
  tag?: PingTag;
}

function img(name: string, cls = ""): HTMLImageElement {
  const el = document.createElement("img");
  if (cls) el.className = cls;
  el.src = textureUrl(name);
  el.onerror = () => {
    el.onerror = null;
    el.src = textureUrl(FALLBACK_TEXTURE);
  };
  return el;
}

const div = (cls: string): HTMLDivElement => {
  const d = document.createElement("div");
  d.className = cls;
  return d;
};

/** 在 (x, y) 放一个信号动画，播完自动删掉。 */
export function spawnPing(layer: HTMLElement, def: PingDef, x: number, y: number, opts: PingFxOptions): HTMLElement {
  const root = div("ping");
  root.style.left = `${x}px`;
  root.style.top = `${y}px`;
  root.style.setProperty("--c", def.color);
  root.style.setProperty("--size", `${opts.sizePx}px`);
  root.style.setProperty("--dur", `${opts.durationS}s`);
  root.append(div("ground"), div("ring"), div("ring r2"), div("ring r3"));
  const art = div("art");
  if (def.art.length === 2) {
    const clash = div("clash");
    clash.append(img(def.art[0], "a1"), img(def.art[1], "a2"));
    art.append(clash, div("flash"));
  } else {
    const pop = div("pop");
    pop.append(img(def.art[0]));
    art.append(pop);
  }
  root.append(art);
  if (opts.tag) {
    const tag = div("tag");
    tag.textContent = opts.tag.name; // 不用 HTML：名字来自别的电脑
    tag.style.setProperty("--tag", opts.tag.color);
    root.append(tag);
  }
  while (layer.childElementCount >= MAX_PINGS) layer.firstElementChild?.remove();
  layer.append(root);
  setTimeout(() => root.remove(), opts.durationS * 1000 + 150);
  return root;
}
