// 队伍文字在透明窗口上的样子：在说话的位置冒出来（每人最多留 3 句，出现 1.5 秒后开始渐隐、3 秒整消失），
// 或者当弹幕从右往左飘过屏幕上方。冒出和弹幕的做法参考 lolPing 的 text-pop 和 danmaku（MIT，© 2026 Danrui Meng）。
// 3.0 加了字号和字幕特效（像剪映的字幕模板）：描边大字、金色渐变、霓虹发光、逐字弹跳、打字机、抖动强调。
// 字号、特效、冒出还是弹幕都由 Rust 随每一句交过来（看的人自己在功能页选的），这里只管画。
import { div } from "../shared/dom";

export type TextEffect = "outline" | "gold" | "neon" | "bounce" | "typewriter" | "shake";

export interface TextSpawn {
  owner: string;
  text: string;
  name: string;
  color: string;
  style: "spot" | "danmaku";
  /** 字号（CSS 像素）。 */
  size: number;
  effect: TextEffect;
  x: number;
  y: number;
}

/** 一句话在屏幕上的寿命：出现 1.5 秒后开始渐隐（CSS 的 textOut），3 秒整消失（两种显示方式都一样）。 */
const TEXT_LIFE_MS = 3000;
const SPOT_PER_OWNER = 3;
/** 弹幕的速度（CSS 像素每秒）、最多几行、和上一条之间至少空多少。行高跟着字号走。 */
const DANMAKU_SPEED = 180;
const LANES = 6;
const LANE_GAP_PX = 48;
/** 逐字出现的特效：一个字最多隔多久，整句最多用多久出完（120 个字也不拖太久）。 */
const CHAR_STEP_MS: Partial<Record<TextEffect, number>> = { bounce: 45, typewriter: 70 };
const REVEAL_MAX_MS = 1600;

/** 按「字」拆开：一个汉字、一个表情（哪怕由几个码点组成）都算一个。 */
function graphemes(s: string): string[] {
  if (typeof Intl.Segmenter === "function") {
    return [...new Intl.Segmenter("zh", { granularity: "grapheme" }).segment(s)].map((g) => g.segment);
  }
  return Array.from(s);
}

/** 一句话本身：带上特效的类名和字号；逐字的特效把每个字包起来，按顺序错开出场。 */
function line(t: TextSpawn, cls: string): HTMLDivElement {
  const body = div(`${cls} fx-${t.effect}`);
  body.style.setProperty("--size", `${t.size}px`);
  const step = CHAR_STEP_MS[t.effect];
  if (step === undefined) {
    body.textContent = t.text;
    return body;
  }
  const chars = graphemes(t.text);
  const gap = Math.min(step, REVEAL_MAX_MS / Math.max(1, chars.length));
  chars.forEach((c, i) => {
    const s = document.createElement("span");
    s.className = "ch";
    s.textContent = c;
    s.style.animationDelay = `${Math.round(i * gap)}ms`;
    body.append(s);
  });
  return body;
}

export class TextStage {
  /** 每人还在屏幕上的气泡，旧的在前。 */
  private readonly spots = new Map<string, HTMLElement[]>();
  /** 每行弹幕什么时候空出来（上一条完全进屏并留出间隔的时间）。 */
  private readonly laneFree: number[] = Array<number>(LANES).fill(0);

  constructor(private readonly layer: HTMLElement) {}

  get isEmpty(): boolean {
    return this.layer.childElementCount === 0;
  }

  spawn(t: TextSpawn): void {
    if (t.style === "danmaku") this.danmaku(t);
    else this.spot(t);
  }

  private spot(t: TextSpawn): void {
    const pop = div("text-pop");
    pop.style.left = `${t.x}px`;
    pop.style.top = `${t.y}px`;
    pop.style.setProperty("--tag", t.color);
    pop.style.setProperty("--size", `${t.size}px`);
    pop.append(div("text-name", t.name), line(t, "text-body"));
    const mine = this.spots.get(t.owner) ?? [];
    // 同一个人的旧句子往上让，最多留 3 句
    for (const old of mine) old.style.setProperty("--lift", String(Number(old.style.getPropertyValue("--lift") || 0) + 1));
    while (mine.length >= SPOT_PER_OWNER) mine.shift()?.remove();
    mine.push(pop);
    this.spots.set(t.owner, mine);
    this.layer.append(pop);
    setTimeout(() => {
      pop.remove();
      const list = this.spots.get(t.owner);
      if (list) {
        const left = list.filter((e) => e !== pop);
        if (left.length) this.spots.set(t.owner, left);
        else this.spots.delete(t.owner);
      }
    }, TEXT_LIFE_MS);
  }

  private danmaku(t: TextSpawn): void {
    const row = div("danmaku");
    row.style.setProperty("--tag", t.color);
    row.style.setProperty("--size", `${t.size}px`);
    row.append(div("dm-name", t.name), line(t, "dm-text"));
    row.style.visibility = "hidden";
    this.layer.append(row);
    const width = row.offsetWidth;
    const now = performance.now();
    let lane = this.laneFree.findIndex((free) => free <= now);
    if (lane < 0) lane = this.laneFree.indexOf(Math.min(...this.laneFree));
    const travel = window.innerWidth + width;
    const durS = travel / DANMAKU_SPEED;
    // 道路只占到这条弹幕真正消失（3 秒）为止，飘不完也不挡后面的
    this.laneFree[lane] = Math.max(now, this.laneFree[lane]) + Math.min(((width + LANE_GAP_PX) / DANMAKU_SPEED) * 1000, TEXT_LIFE_MS);
    row.style.top = `${Math.round(window.innerHeight * 0.08) + lane * Math.round(t.size * 1.55)}px`;
    row.style.setProperty("--travel", `${-travel}px`);
    row.style.setProperty("--dur", `${durS}s`);
    row.style.visibility = "";
    // textOut 的 animationend 或 3 秒整的兜底计时，先到的把它收走
    row.addEventListener("animationend", (e) => e.target === row && row.remove());
    setTimeout(() => row.remove(), TEXT_LIFE_MS);
  }
}
