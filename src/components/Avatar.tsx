// 头像（沿用 1.x）：有自定义头像时显示上传的图片或内置图标；没有时按公钥指纹算出固定的浅色底，写昵称第一个字。
// 各端算法相同，同一个人在所有人那里样子一样。左下角可选在线小圆点。
import type { AvatarData } from "../api";
import { hueOf, initialOf, PRESET_COLORS, PRESET_ICONS } from "../avatars";

interface Props {
  fingerprint: string;
  name?: string | null;
  avatar?: AvatarData | null;
  size?: number;
  /** online 绿点、away 黄点、paused 灰点；不给就不显示。 */
  dot?: "online" | "away" | "paused" | "offline";
}

export function Avatar({ fingerprint, name, avatar, size = 36, dot }: Props) {
  let face;
  if (avatar?.kind === "image") {
    face = <img className="avatar-img" src={avatar.dataUrl} alt="" draggable={false} />;
  } else if (avatar?.kind === "preset" && PRESET_ICONS[avatar.icon]) {
    face = (
      <span
        className="avatar-face preset"
        style={{ background: PRESET_COLORS[avatar.color] ?? PRESET_COLORS[0], fontSize: Math.round(size * 0.56) }}
      >
        {PRESET_ICONS[avatar.icon]}
      </span>
    );
  } else {
    face = (
      <span className="avatar-face auto" style={{ ["--h" as string]: hueOf(fingerprint), fontSize: Math.round(size * 0.42) }}>
        {initialOf(name, fingerprint)}
      </span>
    );
  }
  return (
    <span className="avatar" style={{ width: size, height: size }} aria-hidden="true">
      {face}
      {dot && <span className={`dot ${dot}`} />}
    </span>
  );
}
