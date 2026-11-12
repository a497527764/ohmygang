// 游戏内轮盘 V2 厚重电竞版：10px 荧光青外环、八向粗刻度、加粗扇区分隔、青 + 品红双层中心环、四角切角。
// 选格仍由 Rust 算好再告诉这里（setHot），这里只负责画；几何尺寸和 Rust 的 overlay/geometry.rs 一致。
// 中心与八个格子全是图标，不再放 PING / EMOTE / BACK 这类文字。

const NS = "http://www.w3.org/2000/svg";
/** 设计尺寸（CSS 像素）：中心圆半径 86，环 86–188，正方向的四格延伸到 288，图标在半径 137 处。 */
export const WHEEL = { size: 600, innerR: 86, outerR: 188, extR: 288, iconR: 137, iconSize: 56 } as const;
const SLOTS = 8;

function el<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number>, parent: Element): SVGElementTagNameMap[K] {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  parent.append(e);
  return e;
}

const polar = (r: number, deg: number): [number, number] => [r * Math.cos((deg * Math.PI) / 180), r * Math.sin((deg * Math.PI) / 180)];

function sector(r0: number, r1: number, a0: number, a1: number): string {
  const [x0, y0] = polar(r0, a0);
  const [x1, y1] = polar(r1, a0);
  const [x2, y2] = polar(r1, a1);
  const [x3, y3] = polar(r0, a1);
  return `M${x0},${y0} L${x1},${y1} A${r1},${r1} 0 0 1 ${x2},${y2} L${x3},${y3} A${r0},${r0} 0 0 0 ${x0},${y0}Z`;
}

export class Wheel {
  private readonly wedges: SVGPathElement[] = [];
  private readonly icons: SVGImageElement[] = [];
  private hub: SVGImageElement | null = null;
  private hot = -1;
  private opened = false;

  constructor(
    private readonly svg: SVGSVGElement,
    /** 高亮换格时调用（放刻度声）。 */
    private readonly onHover: () => void,
  ) {
    this.build();
  }

  /** 每格的图标地址，从正上方顺时针。 */
  setIcons(urls: readonly string[]): void {
    this.icons.forEach((icon, i) => {
      const url = urls[i];
      if (url) icon.setAttribute("href", url);
      else icon.removeAttribute("href");
    });
  }

  /** 中心的图标：触发键加单击会放的那个信号或表情。 */
  setCenterIcon(url: string): void {
    this.hub?.setAttribute("href", url);
  }

  /** SVG 和死区一起缩放，按下的点保持在中心。 */
  setSize(sizePx: number): void {
    this.svg.style.width = `${sizePx}px`;
    this.svg.style.height = `${sizePx}px`;
    this.svg.style.marginLeft = `${-sizePx / 2}px`;
    this.svg.style.marginTop = `${-sizePx / 2}px`;
  }

  get isOpen(): boolean {
    return this.opened;
  }

  open(x: number, y: number): void {
    this.svg.style.left = `${x}px`;
    this.svg.style.top = `${y}px`;
    this.setHot(-1);
    this.opened = true;
    this.svg.classList.add("open");
  }

  close(): void {
    this.opened = false;
    this.svg.classList.remove("open");
    this.setHot(-1);
  }

  /** 高亮第 i 格（-1 表示在中心，不高亮）。 */
  setHot(i: number): void {
    if (i === this.hot) return;
    if (this.hot >= 0) {
      this.wedges[this.hot].setAttribute("fill-opacity", "0");
      this.icons[this.hot].classList.remove("hot");
    }
    this.hot = i;
    if (i >= 0 && i < SLOTS) {
      this.wedges[i].setAttribute("fill-opacity", "0.55");
      this.icons[i].classList.add("hot");
      if (this.opened) this.onHover();
    }
  }

  private build(): void {
    const { innerR, outerR, extR, iconR, iconSize } = WHEEL;
    const svg = this.svg;
    const id = `gang-${svg.id}`;
    el("defs", {}, svg).innerHTML = `
      <radialGradient id="${id}-ext" cx="0" cy="0" r="${extR}" gradientUnits="userSpaceOnUse">
        <stop offset="${outerR / extR}" stop-color="#0E3A4C" stop-opacity=".78"/><stop offset="1" stop-color="#0E3A4C" stop-opacity="0"/></radialGradient>
      <radialGradient id="${id}-ring" cx="0" cy="0" r="${outerR}" gradientUnits="userSpaceOnUse">
        <stop offset="${innerR / outerR}" stop-color="#0A111C"/><stop offset="1" stop-color="#16283A"/></radialGradient>
      <radialGradient id="${id}-hot" cx="0" cy="0" r="${extR}" gradientUnits="userSpaceOnUse">
        <stop offset="${innerR / extR}" stop-color="#00E7FF" stop-opacity=".3"/><stop offset="${outerR / extR}" stop-color="#00E7FF" stop-opacity=".16"/>
        <stop offset="1" stop-color="#00E7FF" stop-opacity="0"/></radialGradient>`;

    const slots = [...Array(SLOTS).keys()];
    // 正方向四格的延伸区
    slots.forEach((i) => {
      if (i % 2 === 0) {
        const mid = -90 + i * 45;
        el("path", { d: sector(outerR, extR, mid - 22.5, mid + 22.5), fill: `url(#${id}-ext)` }, svg);
      }
    });
    // 环本体 + 10px 粗荧光青外环 + 深青内圈
    el("circle", { r: (innerR + outerR) / 2, fill: "none", stroke: `url(#${id}-ring)`, "stroke-width": outerR - innerR, opacity: 0.98 }, svg);
    el("circle", { r: outerR, fill: "none", stroke: "#00E7FF", "stroke-width": 10 }, svg);
    el("circle", { r: outerR - 14, fill: "none", stroke: "#0B3A47", "stroke-width": 4 }, svg);

    slots.forEach((i) => {
      const mid = -90 + i * 45;
      const reach = i % 2 === 0 ? extR : outerR;
      this.wedges.push(el("path", { d: sector(innerR, reach, mid - 22.5, mid + 22.5), fill: `url(#${id}-hot)`, "fill-opacity": 0, class: "wedge" }, svg));
      const [x0, y0] = polar(innerR, mid - 22.5);
      const [x1, y1] = polar(outerR, mid - 22.5);
      el("line", { x1: x0, y1: y0, x2: x1, y2: y1, stroke: "#1B6E7D", "stroke-width": 5 }, svg);
    });
    slots.forEach((i) => {
      const [ix, iy] = polar(iconR, -90 + i * 45);
      this.icons.push(el("image", { x: ix - iconSize / 2, y: iy - iconSize / 2, width: iconSize, height: iconSize, class: "icon" }, svg));
    });

    // 八向粗刻度
    slots.forEach((i) => {
      const a = -90 + i * 45;
      const [x0, y0] = polar(outerR + 2, a);
      const [x1, y1] = polar(outerR + 18, a);
      el("line", { x1: x0, y1: y0, x2: x1, y2: y1, stroke: "#00E7FF", "stroke-width": 8, "stroke-linecap": "round" }, svg);
    });
    // 四角赛事 HUD 切角
    el("path", { d: "M-290 -246 V-290 H-246", fill: "none", stroke: "#FF2E63", "stroke-width": 9 }, svg);
    el("path", { d: "M290 246 V290 H246", fill: "none", stroke: "#FF2E63", "stroke-width": 9 }, svg);
    el("path", { d: "M246 -290 H290 V-246", fill: "none", stroke: "#C8FF2E", "stroke-width": 6 }, svg);
    el("path", { d: "M-290 246 V290 H-246", fill: "none", stroke: "#C8FF2E", "stroke-width": 6 }, svg);

    // 中心：黑色厚边 + 粗青环 + 品红内环，只放图标
    el("circle", { r: innerR + 5, fill: "none", stroke: "#080B12", "stroke-width": 10 }, svg);
    el("circle", { r: innerR, fill: "#080B12", stroke: "#00E7FF", "stroke-width": 6, class: "core" }, svg);
    el("circle", { r: innerR - 18, fill: "none", stroke: "#FF2E63", "stroke-width": 3 }, svg);
    this.hub = el("image", { x: -28, y: -28, width: 56, height: 56, class: "hub" }, svg);
  }
}
