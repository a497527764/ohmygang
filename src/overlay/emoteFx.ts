// 导入表情的出场效果和问号占位，透明窗口和设置页的效果预览共用。移植自 lolPing src/renderer/shared/emoteFx.ts
// （MIT，© 2026 Danrui Meng）。

const div = (cls: string): HTMLDivElement => {
  const d = document.createElement("div");
  d.className = cls;
  return d;
};

/** 文件取不到的表情（队友的图还没传过来、或关了「显示队友的表情」）显示成这个。 */
export function bubble(): HTMLDivElement {
  const b = div("bubble");
  b.textContent = "?";
  return b;
}

/** 导入表情（或占位）外面套的动画：内置表情的动画是做在图里的，导入的靠这个。 */
export function envelope(child: HTMLElement, variant: number): HTMLDivElement {
  const fx = div(`fx v${variant}`);
  fx.append(child);
  return fx;
}

/** 按第 variant 种效果放 url 这张图；图取不到就换成占位。 */
export function emoteFxContent(variant: number, url: string): HTMLDivElement {
  const img = document.createElement("img");
  img.alt = "";
  img.draggable = false;
  img.addEventListener("error", () => img.replaceWith(bubble()), { once: true });
  img.src = url;
  return envelope(img, variant);
}
