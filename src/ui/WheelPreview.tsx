// 控制面板里的轮盘缩略图：同一物体的「战术台版」。
// 底层用公共 WheelDial（中性盘面 + 细青环 + 八向刻度），
// 颜色只留在八个信号图标本身；游戏内浮层才是那个唯一的高光瞬间。
import { previewPing, type IconStyle } from "../api";
import { pingIcon } from "../shared/vectorAssets";
import { pingById } from "../shared/pings";
import { Glyph } from "./Glyph";
import { WheelDial } from "./WheelDial";

const R = 140;
const ICON_R = 95;

export function WheelPreview({ wheel, iconStyle, size = 280 }: { wheel: string[]; iconStyle: IconStyle; size?: number }) {
  return (
    <svg className="v-wheel" width={size} height={size} viewBox={`0 0 ${R * 2} ${R * 2}`} aria-label="信号轮盘">
      <WheelDial />
      {wheel.slice(0, 8).map((id, i) => {
        const a = (-90 + i * 45) * (Math.PI / 180);
        const x = R + Math.cos(a) * ICON_R - 20;
        const y = R + Math.sin(a) * ICON_R - 20;
        return <image key={i} href={pingIcon(id, iconStyle)} x={x} y={y} width={40} height={40} />;
      })}
      <text x={R} y={R + 5} textAnchor="middle" className="v-wheel-key">ALT</text>
    </svg>
  );
}

export function WheelTry({ wheel }: { wheel: string[] }) {
  const first = pingById(wheel[0]);
  return (
    <button type="button" className="v-try" onClick={() => first && void previewPing(first.id)} title={first?.name}>
      <Glyph name="play" />
      <span>试一下</span>
    </button>
  );
}
