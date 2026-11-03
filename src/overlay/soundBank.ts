// 音效：开窗口时一次解码成 Web Audio 缓冲，播放时不再读文件（每次新建 <audio> 会有听得出的延迟）。
// 移植自 lolPing src/renderer/overlay/soundBank.ts（MIT，© 2026 Danrui Meng）。

/** 同时响的声音超过这么多就跳过（信号照样画）。 */
export const MAX_VOICES = 8;

export class SoundBank {
  private readonly ctx = new AudioContext({ latencyHint: "interactive" });
  private readonly buffers = new Map<string, AudioBuffer>();
  private voices = 0;

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

  play(key: string, volume01: number): void {
    const buffer = this.buffers.get(key);
    if (!buffer || volume01 <= 0 || this.voices >= MAX_VOICES) return;
    if (this.ctx.state !== "running") void this.ctx.resume();
    const src = this.ctx.createBufferSource();
    const gain = this.ctx.createGain();
    src.buffer = buffer;
    gain.gain.value = volume01;
    src.connect(gain).connect(this.ctx.destination);
    this.voices++;
    src.onended = () => this.voices--;
    src.start();
  }
}
