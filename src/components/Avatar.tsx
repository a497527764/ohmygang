// 头像（V1.1）：有自定义头像时显示上传的图片或内置图标；没有时按公钥指纹算出固定的浅色底，写昵称第一个字。
// 各端算法相同，同一个人在所有人那里样子一样。右下角是心之钢层数角标（0 层不显示），左下角可选在线小绿点。
import type { Avatar as AvatarData } from "../api";
import { hueOf, initialOf, PRESET_COLORS, PRESET_ICONS } from "../avatars";

interface Props {
  fingerprint: string;
  name?: string | null;
  avatar?: AvatarData | null;
  size?: number;
  stacks?: number;
  /// 变化时角标跳一下（实时叠层）。
  pulseKey?: number;
  /// true 显示在线小绿点，false 显示灰点，不给就不显示。
  online?: boolean;
}

function Face({ fingerprint, name, avatar, size }: Required<Pick<Props, "fingerprint" | "size">> & Props) {
  if (avatar?.kind === "image") {
    return <img className="avatar-img" src={avatar.dataUrl} alt="" draggable={false} />;
  }
  if (avatar?.kind === "preset" && PRESET_ICONS[avatar.icon]) {
    return (
      <span
        className="avatar-face preset"
        style={{ background: PRESET_COLORS[avatar.color] ?? PRESET_COLORS[0], fontSize: Math.round(size * 0.56) }}
      >
        {PRESET_ICONS[avatar.icon]}
      </span>
    );
  }
  return (
    <span
      className="avatar-face auto"
      style={{ ["--h" as string]: hueOf(fingerprint), fontSize: Math.round(size * 0.42) }}
    >
      {initialOf(name, fingerprint)}
    </span>
  );
}

export function Avatar({ fingerprint, name, avatar, size = 36, stacks = 0, pulseKey, online }: Props) {
  return (
    <span className="avatar" style={{ width: size, height: size }} aria-hidden="true">
      <Face fingerprint={fingerprint} name={name} avatar={avatar} size={size} />
      {online !== undefined && <span className={online ? "presence on" : "presence"} />}
      {stacks > 0 && (
        <span key={pulseKey} className={pulseKey ? "badge pulse" : "badge"}>
          {stacks}
        </span>
      )}
    </span>
  );
}
