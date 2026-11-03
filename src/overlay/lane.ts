// 对线模式的画面：对手的小英雄（跟着他的鼠标走）、「700 码」范围圈、充能圈、冷却倒计时、「谁在对你充能」，
// 以及心之钢爆炸特效。位置和状态都由 Rust 算好（lane.rs、laning.rs），这里只画。

export interface LaneOpponent {
  id: string;
  name: string;
  color: string;
  x: number;
  y: number;
  phase: "far" | "charging" | "charged" | "cooldown";
  /** 充能进度 0 到 1。 */
  progress: number;
  cooldownS: number;
}

export interface LaneState {
  opponents: LaneOpponent[];
  chargingMe: string[];
  /** 「700 码」范围和命中半径，CSS 像素。 */
  rangePx: number;
  hitPx: number;
}

const NS = "http://www.w3.org/2000/svg";
const RING_R = 24;
const RING_LEN = 2 * Math.PI * RING_R;

interface Minion {
  root: HTMLDivElement;
  range: HTMLDivElement;
  ring: SVGCircleElement;
  face: HTMLDivElement;
  cd: HTMLDivElement;
  label: HTMLDivElement;
}

function div(cls: string, parent: HTMLElement): HTMLDivElement {
  const d = document.createElement("div");
  d.className = cls;
  parent.append(d);
  return d;
}

function makeMinion(layer: HTMLElement): Minion {
  const root = div("minion", layer);
  const range = div("range", root);
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("class", "charge");
  svg.setAttribute("viewBox", "-30 -30 60 60");
  const track = document.createElementNS(NS, "circle");
  track.setAttribute("r", String(RING_R));
  track.setAttribute("class", "track");
  const ring = document.createElementNS(NS, "circle");
  ring.setAttribute("r", String(RING_R));
  ring.setAttribute("class", "fill");
  ring.setAttribute("stroke-dasharray", String(RING_LEN));
  svg.append(track, ring);
  root.append(svg);
  const face = div("face", root);
  const cd = div("cd", root);
  const label = div("label", root);
  return { root, range, ring, face, cd, label };
}

export class LaneStage {
  private readonly minions = new Map<string, Minion>();
  private readonly warn: HTMLDivElement;

  constructor(private readonly layer: HTMLElement) {
    this.warn = div("lane-warn", layer);
  }

  render(s: LaneState): void {
    const seen = new Set<string>();
    for (const o of s.opponents) {
      seen.add(o.id);
      let m = this.minions.get(o.id);
      if (!m) {
        m = makeMinion(this.layer);
        this.minions.set(o.id, m);
      }
      m.root.className = `minion ${o.phase}`;
      m.root.style.left = `${o.x}px`;
      m.root.style.top = `${o.y}px`;
      m.root.style.setProperty("--tag", o.color);
      m.range.style.width = m.range.style.height = `${s.rangePx * 2}px`;
      m.range.style.margin = `${-s.rangePx}px 0 0 ${-s.rangePx}px`;
      m.ring.setAttribute("stroke-dashoffset", String(RING_LEN * (1 - o.progress)));
      m.face.textContent = Array.from(o.name)[0] ?? "?"; // 不用 HTML：名字来自别的电脑
      m.label.textContent = o.name;
      m.cd.textContent = o.phase === "cooldown" ? `${o.cooldownS}` : "";
    }
    for (const [id, m] of this.minions) {
      if (!seen.has(id)) {
        m.root.remove();
        this.minions.delete(id);
      }
    }
    this.warn.textContent = s.chargingMe.length > 0 ? `${s.chargingMe.join("、")} 正在对你充能，快把鼠标甩开！` : "";
    this.warn.classList.toggle("on", s.chargingMe.length > 0);
  }

  /** 有没有东西在画（空闲上报用）。 */
  get isEmpty(): boolean {
    return this.minions.size === 0 && !this.warn.classList.contains("on");
  }
}

const HIT_MS = 1600;

/** 心之钢爆炸：图标弹出、两道光环、一行字。 */
export function spawnHeartsteel(layer: HTMLElement, icon: string, x: number, y: number, text: string): void {
  const root = div("hs-fx", layer);
  root.style.left = `${x}px`;
  root.style.top = `${y}px`;
  div("hs-ring", root);
  div("hs-ring r2", root);
  const img = document.createElement("img");
  img.src = icon;
  img.alt = "";
  img.className = "hs-icon";
  root.append(img);
  div("hs-text", root).textContent = text;
  setTimeout(() => root.remove(), HIT_MS);
}
