// 空闲上报：透明窗口上什么都没有 250 毫秒后告诉 Rust，Rust 把窗口挪开；一有东西就马上说不空闲了。
// 盖满整块屏幕的窗口哪怕鼠标穿透，截图工具也会把它当成光标下的窗口。
// 移植自 lolPing src/renderer/overlay/idleWatch.ts（MIT，© 2026 Danrui Meng）。

/** 比轮盘 0.12 秒的淡出长。 */
export const IDLE_MS = 250;

export class IdleWatch {
  private idle = false;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly isEmpty: () => boolean, private readonly report: (idle: boolean) => void) {}

  /** 画面上可能多了或少了东西时调用。 */
  check(): void {
    if (!this.isEmpty()) {
      if (this.timer !== null) clearTimeout(this.timer);
      this.timer = null;
      if (this.idle) {
        this.idle = false;
        this.report(false);
      }
      return;
    }
    if (this.idle || this.timer !== null) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      if (!this.isEmpty()) return;
      this.idle = true;
      this.report(true);
    }, IDLE_MS);
  }
}
