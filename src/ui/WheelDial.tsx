// 轮盘的公共底层：主页缩略图、信号编辑页、表情编辑页共用。
// 中性盘面 + 一圈细青环 + 八向短刻度，颜色只留给叠在上层的信号/表情图标。
// 颜色走 CSS 变量，深浅主题自动跟随；这里不写死任何色值。
const R = 140;

function polar(r: number, deg: number): [number, number] {
  return [r * Math.cos((deg * Math.PI) / 180), r * Math.sin((deg * Math.PI) / 180)];
}

export function WheelDial() {
  const dividers = Array.from({ length: 8 }, (_, i) => {
    const a = -90 + i * 45 - 22.5;
    const [x0, y0] = polar(58, a);
    const [x1, y1] = polar(129, a);
    return <line key={`d${i}`} className="wp-div" x1={R + x0} y1={R + y0} x2={R + x1} y2={R + y1} />;
  });
  // 八向短刻度是产品的签名细节，只出现在这里，不铺到别处
  const ticks = Array.from({ length: 8 }, (_, i) => {
    const a = (-90 + i * 45) * (Math.PI / 180);
    const [x0, y0] = [R + Math.cos(a) * 132, R + Math.sin(a) * 132];
    const [x1, y1] = [R + Math.cos(a) * 140, R + Math.sin(a) * 140];
    return <line key={`t${i}`} className="wp-tick" x1={x0} y1={y0} x2={x1} y2={y1} />;
  });

  return (
    <>
      <circle className="wp-fill" cx={R} cy={R} r={129} />
      <circle className="wp-ring" cx={R} cy={R} r={129} />
      {dividers}
      <circle className="wp-core" cx={R} cy={R} r={56} />
      <circle className="wp-inner" cx={R} cy={R} r={42} />
      {ticks}
    </>
  );
}
