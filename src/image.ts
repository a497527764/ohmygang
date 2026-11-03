// 图片压缩（V1.1）：在界面里把图片画到画布上再导出 WebP，压到协议允许的大小，再交给 Rust。
// 聊天图片：最长边 1280 像素，目标 150KB 以内（Rust 侧上限 192KB）。
// 头像：裁成正方形，64 像素起，压到 800 字节以内，放不下就逐步缩小、降画质。

export interface Compressed {
  /// Base64，不带 data: 前缀。
  data: string;
  bytes: number;
  width: number;
  height: number;
}

const CHAT_MAX_SIDE = 1280;
const CHAT_TARGET_BYTES = 150 * 1024;
const AVATAR_MAX_BYTES = 800;

function canvasBlob(canvas: HTMLCanvasElement, quality: number): Promise<Blob> {
  return new Promise((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("图片导出失败"))), "image/webp", quality),
  );
}

async function toBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}

async function decode(file: Blob): Promise<ImageBitmap> {
  try {
    return await createImageBitmap(file);
  } catch {
    throw new Error("读不了这张图片，换一张试试");
  }
}

/// 把 (sx, sy, sw, sh) 这块画成 w×h，按给定画质逐档尝试，返回第一个不超过 maxBytes 的结果。
async function fit(
  img: ImageBitmap,
  src: [number, number, number, number],
  sizes: [number, number][],
  qualities: number[],
  maxBytes: number,
): Promise<Compressed | null> {
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("图片处理失败");
  for (const [w, h] of sizes) {
    canvas.width = w;
    canvas.height = h;
    ctx.imageSmoothingQuality = "high";
    ctx.clearRect(0, 0, w, h);
    ctx.drawImage(img, ...src, 0, 0, w, h);
    for (const q of qualities) {
      const blob = await canvasBlob(canvas, q);
      if (blob.size <= maxBytes) return { data: await toBase64(blob), bytes: blob.size, width: w, height: h };
    }
  }
  return null;
}

/// 聊天图片：等比缩到最长边 1280，画质从 0.85 往下试；还不行就再缩小。
export async function compressForChat(file: Blob): Promise<Compressed> {
  const img = await decode(file);
  const scale = Math.min(1, CHAT_MAX_SIDE / Math.max(img.width, img.height));
  const sizes: [number, number][] = [1, 0.75, 0.55, 0.4].map((k) => [
    Math.max(1, Math.round(img.width * scale * k)),
    Math.max(1, Math.round(img.height * scale * k)),
  ]);
  const out = await fit(img, [0, 0, img.width, img.height], sizes, [0.85, 0.72, 0.6, 0.48], CHAT_TARGET_BYTES);
  img.close();
  if (!out) throw new Error("图片压缩后还是太大，换一张小一点的");
  return out;
}

/// 头像：取中间的正方形，64 像素起压到 800 字节以内。
export async function compressForAvatar(file: Blob): Promise<Compressed> {
  const img = await decode(file);
  const side = Math.min(img.width, img.height);
  const src: [number, number, number, number] = [(img.width - side) / 2, (img.height - side) / 2, side, side];
  const sizes: [number, number][] = [64, 56, 48, 40, 32].map((s) => [s, s]);
  const out = await fit(img, src, sizes, [0.8, 0.65, 0.5, 0.35, 0.2], AVATAR_MAX_BYTES);
  img.close();
  if (!out) throw new Error("这张图做不成头像，换一张简单点的");
  return out;
}

/// 剪贴板或拖进来的文件里有没有图片。
export function firstImage(items: DataTransferItemList | null | undefined): File | null {
  for (const it of Array.from(items ?? [])) {
    if (it.kind === "file" && it.type.startsWith("image/")) return it.getAsFile();
  }
  return null;
}
