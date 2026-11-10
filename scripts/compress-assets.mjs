// 压缩打包进程序的表情动图、图片和信号音效（3.2.0-M7）。打包时不跑；以后加新的内置表情或音效时，只对新加的原文件手动跑一次：
//   node scripts/compress-assets.mjs public/lol/emotes/新表情.webp public/lol/emotes/新表情.png ...
//
// - .webp 动图：有损重新编码，颜色质量 80，透明边缘分 56 级（原来几乎不损，占了文件的六成）。帧数、每帧时长、只播一次都不变。
//   试过分 16 级：小 1/4，但柔光上有一圈圈台阶；降到每秒 15 帧动作不顺。所有者看过对比定的这一档（3.2.0 决定 4）。
//   每跑一次都会再损失一点，所以已经压过的别再跑（看不出压没压过：解码时会把分的级抹平）
// - .png：无损重新压缩，画出来的每个像素都不变；没变小就不动
// - .wav：左右声道几乎一样（差的那部分比声音本身小 50 dB 以上）的改成单声道，听不出区别，解码后占的内存也少一半；
//   不降采样：信号音效有不少声音在 11 kHz 以上，降到 22 kHz 听得出来
import { readFileSync, writeFileSync } from "node:fs";
import sharp from "sharp";

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("用法：node scripts/compress-assets.mjs <文件>...（.webp 动图、.png、.wav）");
  process.exit(1);
}

function webp(buf) {
  return sharp(buf, { animated: true }).webp({ quality: 80, alphaQuality: 75, effort: 6, minSize: true }).toBuffer();
}

async function png(buf) {
  const out = await sharp(buf).png({ compressionLevel: 9, adaptiveFiltering: false }).toBuffer();
  if (out.length >= buf.length) return null;
  const pixels = (b) => sharp(b).ensureAlpha().raw().toBuffer();
  if (!(await pixels(buf)).equals(await pixels(out))) throw new Error("重新压缩后像素变了");
  return out;
}

function wav(buf) {
  let fmt = null, data = null;
  for (let p = 12; p + 8 <= buf.length; ) {
    const id = buf.toString("ascii", p, p + 4);
    const size = buf.readUInt32LE(p + 4);
    if (id === "fmt ") fmt = { tag: buf.readUInt16LE(p + 8), channels: buf.readUInt16LE(p + 10), rate: buf.readUInt32LE(p + 12), bits: buf.readUInt16LE(p + 22) };
    if (id === "data") data = buf.subarray(p + 8, p + 8 + size);
    p += 8 + size + (size & 1);
  }
  if (!fmt || !data || fmt.tag !== 1 || fmt.bits !== 16) throw new Error("只认 16 位 PCM 的 WAV");
  if (fmt.channels !== 2) return null;
  const n = data.length / 4;
  let side = 0, mid = 0;
  for (let i = 0; i < n; i++) {
    const l = data.readInt16LE(i * 4), r = data.readInt16LE(i * 4 + 2);
    side += (l - r) ** 2;
    mid += (l + r) ** 2;
  }
  if (side > mid * 1e-5) return null; // 两边差得多于 -50 dB：是真的立体声，不动
  const out = Buffer.alloc(44 + n * 2);
  out.write("RIFF", 0, "ascii");
  out.writeUInt32LE(36 + n * 2, 4);
  out.write("WAVEfmt ", 8, "ascii");
  out.writeUInt32LE(16, 16);
  out.writeUInt16LE(1, 20); // PCM
  out.writeUInt16LE(1, 22); // 单声道
  out.writeUInt32LE(fmt.rate, 24);
  out.writeUInt32LE(fmt.rate * 2, 28);
  out.writeUInt16LE(2, 32);
  out.writeUInt16LE(16, 34);
  out.write("data", 36, "ascii");
  out.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) out.writeInt16LE(Math.round((data.readInt16LE(i * 4) + data.readInt16LE(i * 4 + 2)) / 2), 44 + i * 2);
  return out;
}

let before = 0, after = 0;
for (const file of files) {
  const buf = readFileSync(file);
  const ext = file.slice(file.lastIndexOf(".")).toLowerCase();
  const out = ext === ".webp" ? await webp(buf) : ext === ".png" ? await png(buf) : ext === ".wav" ? wav(buf) : undefined;
  if (out === undefined) throw new Error(`不认识的文件：${file}`);
  before += buf.length;
  after += (out ?? buf).length;
  if (out) writeFileSync(file, out);
  console.log(`${file}: ${out ? `${buf.length} → ${out.length}` : "不用改"}`);
}
console.log(`合计 ${before} → ${after} 字节`);
