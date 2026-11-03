// 队伍文字在透明窗口上的样子：在说话的位置冒出一个气泡（每人最多留 3 句，6 秒后消失），
// 或者当弹幕从右往左飘过屏幕上方。样子参考 lolPing 的 text-pop 和 danmaku（MIT，© 2026 Danrui Meng）。

export interface TextSpawn {
  owner: string;
  text: string;
  name: string;
  color: string;
  style: "spot" | "danmaku";
  x: number;
  y: number;
}

const SPOT_MS = 6000;
const SPOT_PER_OWNER = 3;
/** 弹幕的速度（CSS 像素每秒）、行高、最多几行、和上一条之间至少空多少。 */
const DANMAKU_SPEED = 180;
const LANE_PX = 38;
const LANES = 6;
const LANE_GAP_PX = 48;

const div = (cls: string, text?: string): HTMLDivElement => {
  const d = document.createElement("div");
  d.className = cls;
  if (text !== undefined) d.textContent = text; // 只当文字：内容来自别人的电脑
  return d;
};

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
    pop.style.setProperty("--dur", `${SPOT_MS / 1000}s`);
    pop.append(div("text-name", t.name), div("text-body", t.text));
    const mine = this.spots.get(t.owner) ?? [];
    // 同一个人的旧气泡往上让，最多留 3 句
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
    }, SPOT_MS);
  }

  private danmaku(t: TextSpawn): void {
    const line = div("danmaku");
    line.style.setProperty("--tag", t.color);
    line.append(div("dm-name", t.name), div("dm-text", t.text));
    line.style.visibility = "hidden";
    this.layer.append(line);
    const width = line.offsetWidth;
    const now = performance.now();
    let lane = this.laneFree.findIndex((free) => free <= now);
    if (lane < 0) lane = this.laneFree.indexOf(Math.min(...this.laneFree));
    const travel = window.innerWidth + width;
    const durS = travel / DANMAKU_SPEED;
    this.laneFree[lane] = Math.max(now, this.laneFree[lane]) + ((width + LANE_GAP_PX) / DANMAKU_SPEED) * 1000;
    line.style.top = `${Math.round(window.innerHeight * 0.08) + lane * LANE_PX}px`;
    line.style.setProperty("--travel", `${-travel}px`);
    line.style.setProperty("--dur", `${durS}s`);
    line.style.visibility = "";
    line.addEventListener("animationend", () => line.remove(), { once: true });
  }
}
