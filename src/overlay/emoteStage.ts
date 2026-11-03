// 透明窗口上的表情：每个人（自己、预览、每位队友）同时最多一个，3 秒后消失。
// 移植自 lolPing src/renderer/overlay/emoteStage.ts（MIT，© 2026 Danrui Meng），去掉了 AI 队友的「一直留着」。
import { EMOTE_DURATION_S, bundledAnimUrl } from "../shared/emotes";
import { bubble, emoteFxContent, envelope } from "./emoteFx";
import type { PingTag } from "./pingFx";

/** 画出来的样子：内置表情（动画做在图里）、导入的图（套出场效果）、占位问号。 */
export type EmoteArt = { kind: "bundled"; slug: string } | { kind: "custom"; url: string; fx: number } | { kind: "placeholder" };

interface Live {
  root: HTMLElement;
  timer: ReturnType<typeof setTimeout>;
  /** 内置表情动画的 object URL，取到以后才有。 */
  url: string | null;
  gone: boolean;
}

const div = (cls: string): HTMLDivElement => {
  const d = document.createElement("div");
  d.className = cls;
  return d;
};

export class EmoteStage {
  private readonly live = new Map<string, Live>();
  private readonly blobs = new Map<string, Promise<Blob>>();

  constructor(private readonly layer: HTMLElement) {}

  get isEmpty(): boolean {
    return this.live.size === 0;
  }

  /** 以 (x, y) 为中心放 art；这个人屏幕上已经有一个就先换掉。 */
  spawn(owner: string, art: EmoteArt, x: number, y: number, sizePx: number, tag?: PingTag): void {
    this.clear(owner);
    const root = div("emote");
    root.style.left = `${x}px`;
    root.style.top = `${y}px`;
    root.style.setProperty("--size", `${sizePx}px`);
    root.style.setProperty("--dur", `${EMOTE_DURATION_S}s`);
    const live: Live = { root, url: null, gone: false, timer: setTimeout(() => this.end(owner, live), EMOTE_DURATION_S * 1000 + 150) };
    this.live.set(owner, live);

    if (art.kind === "bundled") this.showBundled(live, art.slug);
    else if (art.kind === "custom") root.append(emoteFxContent(art.fx, art.url));
    else root.append(envelope(bubble(), 0));

    if (tag) {
      const t = div("tag");
      t.textContent = tag.name; // 只当文字：名字来自别人的电脑
      t.style.setProperty("--tag", tag.color);
      root.append(t);
    }
    this.layer.append(root);
  }

  clear(owner: string): void {
    const live = this.live.get(owner);
    if (live) this.end(owner, live);
  }

  /**
   * 内置表情的动画只播一次、从第一帧开始；Chromium 让同一个地址的图共用一个时钟，
   * 所以每次放都给一个新的 object URL（文件只取一次）。
   */
  private showBundled(live: Live, slug: string): void {
    const img = document.createElement("img");
    img.className = "sheet";
    img.alt = "";
    live.root.append(img);
    let swapped = false;
    const placeholder = (): void => {
      if (swapped || live.gone) return;
      swapped = true;
      img.replaceWith(envelope(bubble(), 0));
    };
    img.addEventListener("error", placeholder, { once: true });
    this.blob(slug).then((blob) => {
      if (live.gone || swapped) return;
      live.url = URL.createObjectURL(blob);
      img.src = live.url;
    }, placeholder);
  }

  private blob(slug: string): Promise<Blob> {
    let blob = this.blobs.get(slug);
    if (!blob) {
      blob = fetch(bundledAnimUrl(slug)).then((r) => (r.ok ? r.blob() : Promise.reject(new Error(String(r.status)))));
      this.blobs.set(slug, blob);
      const fetched = blob;
      blob.catch(() => {
        if (this.blobs.get(slug) === fetched) this.blobs.delete(slug); // 下次再试
      });
    }
    return blob;
  }

  private end(owner: string, live: Live): void {
    if (live.gone) return;
    live.gone = true;
    clearTimeout(live.timer);
    live.root.remove();
    if (live.url) URL.revokeObjectURL(live.url);
    if (this.live.get(owner) === live) this.live.delete(owner);
  }
}
