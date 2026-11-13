// 个人资料的大头像：选图时在控制面板里先校验（正方形、边长 ≤500、文件 ≤200 KB），
// 再居中裁方缩到 200×200、压成 32 KB 以内的 WebP 发给后端。队伍里传输的分辨率统一 200×200。
// Rust 写盘前还会按文件头再查一遍（emotes/store.rs 的 check_avatar）。
import { imageSize, sniffImage, toBase64 } from "./emoteImport";
import { PRESET_COLORS, PRESET_ICONS } from "./avatars";

type Result<T> = { ok: true; value: T } | { ok: false; error: string };
const fail = <T>(error: string): Result<T> => ({ ok: false, error });

const AVATAR_SIDE = 200;
const AVATAR_MAX_BYTES = 32 * 1024;
const INPUT_MAX_SIDE = 500;
const INPUT_MAX_BYTES = 200 * 1024;
const QUALITIES = [0.9, 0.82, 0.74, 0.66, 0.58, 0.5, 0.42, 0.34];

export interface PreparedAvatar {
  /** WebP 字节的 base64，发给 profile_set 的 avatarImage。 */
  data: string;
  /** 预览用的 data URL。 */
  dataUrl: string;
}

/** 选来的图 → 200×200 方形 WebP。要求原图就是正方形、最长边 500 以内、200 KB 以内。 */
export async function prepareAvatar(file: File): Promise<Result<PreparedAvatar>> {
  if (file.size > INPUT_MAX_BYTES) return fail("图片太大了，最多 200 KB");
  const bytes = new Uint8Array(await file.arrayBuffer());
  const type = sniffImage(bytes);
  if (!type) return fail("只能选 PNG、JPEG 或 WebP 图片");
  if (type === "gif") return fail("头像不能用动图，选一张静态图片");
  const declared = imageSize(bytes);
  if (!declared) return fail("这张图打不开");
  if (declared.width !== declared.height)
    return fail(`头像只能是正方形：这张是 ${declared.width}×${declared.height}`);
  if (Math.max(declared.width, declared.height) > INPUT_MAX_SIDE)
    return fail(`图片太大了：最长边不超过 500 像素`);
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(new Blob([bytes.slice()], { type: `image/${type}` }));
  } catch {
    return fail("这张图打不开");
  }
  try {
    const canvas = new OffscreenCanvas(AVATAR_SIDE, AVATAR_SIDE);
    const ctx = canvas.getContext("2d");
    if (!ctx) return fail("这张图打不开");
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(bitmap, 0, 0, bitmap.width, bitmap.height, 0, 0, AVATAR_SIDE, AVATAR_SIDE);
    for (const quality of QUALITIES) {
      const blob = await canvas.convertToBlob({ type: "image/webp", quality });
      if (blob.type === "image/webp" && blob.size <= AVATAR_MAX_BYTES) {
        const data = toBase64(new Uint8Array(await blob.arrayBuffer()));
        return { ok: true, value: { data, dataUrl: `data:image/webp;base64,${data}` } };
      }
    }
    return fail("这张图细节太多，压不进 32 KB，换一张简单点的");
  } finally {
    bitmap.close();
  }
}

/** 随机内置头像（没设过大图的人用；不设时界面也会按指纹自动生成一个）。 */
export function randomPreset(): { kind: "preset"; icon: number; color: number } {
  return {
    kind: "preset",
    icon: Math.floor(Math.random() * PRESET_ICONS.length),
    color: Math.floor(Math.random() * PRESET_COLORS.length),
  };
}

/** data URL 里的 base64 段取出来（存回 profile_set 用）。不是 data URL 时返回 null。 */
export function base64OfDataUrl(url: string): string | null {
  const i = url.indexOf("base64,");
  return i >= 0 ? url.slice(i + 7) : null;
}
