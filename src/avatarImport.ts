// 个人资料的大头像：选图对用户零限制（任意形状、任意尺寸都行），
// 界面里用方形取景框拖动缩放选好区域，确认时自动裁成 200×200、压成 32 KB 以内的 WebP 发给后端。
// 队伍里传输的分辨率统一 200×200。Rust 写盘前还会按文件头再查一遍（emotes/store.rs 的 check_avatar）。
import { imageSize, sniffImage, toBase64 } from "./emoteImport";
import { PRESET_COLORS, PRESET_ICONS } from "./avatars";

type Result<T> = { ok: true; value: T } | { ok: false; error: string };
const fail = <T>(error: string): Result<T> => ({ ok: false, error });

const AVATAR_SIDE = 200;
const AVATAR_MAX_BYTES = 32 * 1024;
/** 只是解码安全上限（和表情导入一致），不是对用户选图的要求。 */
const INPUT_MAX_BYTES = 8 * 1024 * 1024;
const INPUT_MAX_SIDE = 8192;
const QUALITIES = [0.9, 0.82, 0.74, 0.66, 0.58, 0.5, 0.42, 0.34, 0.26, 0.2];

export interface PreparedAvatar {
  /** WebP 字节的 base64，发给 profile_set 的 avatarImage。 */
  data: string;
  /** 预览用的 data URL。 */
  dataUrl: string;
}

/** 选来的图先解出来给裁剪框用；GIF / 动图取第一帧。 */
export async function loadAvatarImage(file: File): Promise<Result<ImageBitmap>> {
  if (file.size > INPUT_MAX_BYTES) return fail("图片太大了（超过 8 MB），换一张小点的");
  const bytes = new Uint8Array(await file.arrayBuffer());
  const type = sniffImage(bytes);
  if (!type) return fail("只能选图片（PNG、JPEG、GIF、WebP）");
  const declared = imageSize(bytes);
  if (!declared) return fail("这张图打不开");
  if (Math.max(declared.width, declared.height) > INPUT_MAX_SIDE)
    return fail("这张图太大了：最长边超过 8192 像素");
  try {
    const bitmap = await createImageBitmap(new Blob([bytes.slice()], { type: `image/${type}` }));
    return { ok: true, value: bitmap };
  } catch {
    return fail("这张图打不开");
  }
}

/** 取景框选中的区域（原图坐标）→ 200×200 方形 WebP，自动压到 32 KB 以内。 */
export async function cropAvatar(
  bitmap: ImageBitmap,
  src: { x: number; y: number; w: number; h: number },
): Promise<Result<PreparedAvatar>> {
  const canvas = new OffscreenCanvas(AVATAR_SIDE, AVATAR_SIDE);
  const ctx = canvas.getContext("2d");
  if (!ctx) return fail("裁剪失败，再试一次");
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(bitmap, src.x, src.y, src.w, src.h, 0, 0, AVATAR_SIDE, AVATAR_SIDE);
  for (const quality of QUALITIES) {
    const blob = await canvas.convertToBlob({ type: "image/webp", quality });
    if (blob.type === "image/webp" && blob.size <= AVATAR_MAX_BYTES) {
      const data = toBase64(new Uint8Array(await blob.arrayBuffer()));
      return { ok: true, value: { data, dataUrl: `data:image/webp;base64,${data}` } };
    }
  }
  return fail("这张图压不进 32 KB，换一张简单点的");
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
