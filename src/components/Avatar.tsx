// 成员头像（M1-5）：第一版没有自定义头像，用公钥指纹算出固定颜色，写昵称第一个字。
// 各端算法相同，所以同一个人在所有人那里颜色一样。右下角是心之钢层数角标，0 层不显示。

/// 指纹 → 色相。避开红色一段（330°–30°），免得和红色的层数角标混在一起。
export function avatarColor(fingerprint: string): string {
  let h = 2166136261;
  for (const ch of fingerprint) {
    h ^= ch.charCodeAt(0);
    h = Math.imul(h, 16777619);
  }
  const hue = 30 + ((h >>> 0) % 300);
  return `hsl(${hue} 38% 46%)`;
}

/// 昵称第一个字（按字符算，表情也算一个）；没昵称时用指纹前两位。
export function initialOf(name: string | null | undefined, fingerprint: string): string {
  const first = Array.from((name ?? "").trim())[0];
  return first ?? fingerprint.slice(0, 2).toUpperCase();
}

interface Props {
  fingerprint: string;
  name?: string | null;
  size?: number;
  stacks?: number;
  /// 变化时角标跳一下（实时叠层）。
  pulseKey?: number;
}

export function Avatar({ fingerprint, name, size = 36, stacks = 0, pulseKey }: Props) {
  return (
    <span
      className="avatar"
      style={{ width: size, height: size, background: avatarColor(fingerprint), fontSize: Math.round(size * 0.44) }}
      aria-hidden="true"
    >
      {initialOf(name, fingerprint)}
      {stacks > 0 && (
        <span key={pulseKey} className={pulseKey ? "badge pulse" : "badge"}>
          {stacks}
        </span>
      )}
    </span>
  );
}
