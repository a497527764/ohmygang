// 音效：开窗口时一次解码成 Web Audio 缓冲，播放时不再读文件（每次新建 <audio> 会有听得出的延迟）。
// 移植自 lolPing src/renderer/overlay/soundBank.ts（MIT，© 2026 Danrui Meng）。

/** 同时响的声音超过这么多就跳过（信号照样画）。 */
export const MAX_VOICES = 8;
/**
 * 没有声音在响这么久后暂停音频（3.1.1）：开着的 AudioContext 闲着也一直在混静音，
 * 系统的音频服务一直占着 CPU。连着放的几个声音之间不暂停，免得每次重新启动的那一点延迟。
 */
const IDLE_SUSPEND_MS = 3000;

export class SoundBank {
  private readonly ctx = new AudioContext({ latencyHint: "interactive" });
  private readonly buffers = new Map<string, AudioBuffer>();
  private voices = 0;
  private idleTimer: number | undefined;

  constructor() {
    this.suspendLater();
  }

  /** 把每个声音解码好存在自己的名字下。返回没加载成功的地址。 */
  async load(entries: readonly { key: string; url: string }[]): Promise<string[]> {
    const results = await Promise.all(
      entries.map(async ({ key, url }) => {
        try {
          const res = await fetch(url);
          if (!res.ok) throw new Error(String(res.status));
          this.buffers.set(key, await this.ctx.decodeAudioData(await res.arrayBuffer()));
          return null;
        } catch {
          return url;
        }
      }),
    );
    return results.filter((r): r is string => r !== null);
  }

  /** 第一次放时才加载（表情音效 44 个，不在开窗口时全部解码）。 */
  async playLazy(key: string, url: string, volume01: number): Promise<void> {
    if (volume01 <= 0) return;
    if (!this.buffers.has(key)) await this.load([{ key, url }]);
    this.play(key, volume01);
  }

  /** 忘掉这些声音，下次放时重新加载。 */
  forget(match: (key: string) => boolean): void {
    for (const key of [...this.buffers.keys()]) if (match(key)) this.buffers.delete(key);
  }

  play(key: string, volume01: number): void {
    const buffer = this.buffers.get(key);
    if (!buffer || volume01 <= 0 || this.voices >= MAX_VOICES) return;
    window.clearTimeout(this.idleTimer);
    this.idleTimer = undefined;
    if (this.ctx.state !== "running") void this.ctx.resume();
    const src = this.ctx.createBufferSource();
    const gain = this.ctx.createGain();
    src.buffer = buffer;
    gain.gain.value = volume01;
    src.connect(gain).connect(this.ctx.destination);
    this.voices++;
    src.onended = () => {
      this.voices--;
      if (this.voices === 0) this.suspendLater();
    };
    src.start();
  }

  /** 一段时间里没再放声音就暂停音频，下次放时再启动。 */
  private suspendLater(): void {
    window.clearTimeout(this.idleTimer);
    this.idleTimer = window.setTimeout(() => {
      this.idleTimer = undefined;
      if (this.voices === 0 && this.ctx.state === "running") void this.ctx.suspend();
    }, IDLE_SUSPEND_MS);
  }
}
