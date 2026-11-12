// 线性图标系统：手绘 SVG 路径，统一 24 网格、2.6 粗描边、圆头圆角，电竞厚度。
// 颜色继承文字角色，不带自己的颜色；需要强调的地方由外层 CSS 给 drop-shadow。
const PATHS = {
  home: "M3 11 L12 3 L21 11 M6 10 V21 H18 V10",
  target: "M12 2 A10 10 0 1 0 12 22 A10 10 0 1 0 12 2 M12 8 A4 4 0 1 0 12 16 A4 4 0 1 0 12 8 M12 11.5 V12.5",
  smile: "M12 2 A10 10 0 1 0 12 22 A10 10 0 1 0 12 2 M8 14 Q12 18 16 14 M8.5 9 H8.6 M15.5 9 H15.6",
  users: "M9 11 A3.5 3.5 0 1 0 9 4 A3.5 3.5 0 1 0 9 11 M2 20 A7 7 0 0 1 16 20 M17 5 A3.5 3.5 0 0 1 17 11 M18 14 A5.5 5.5 0 0 1 22 20",
  gear: "M10 2 H14 L15 5 L17 6 L20 5 L22 8 L20 10 V14 L22 16 L20 19 L17 18 L15 19 L14 22 H10 L9 19 L7 18 L4 19 L2 16 L4 14 V10 L2 8 L4 5 L7 6 L9 5 Z M12 9 A3 3 0 1 0 12 15 A3 3 0 1 0 12 9",
  power: "M12 2 V11 M6.5 5.5 A9 9 0 1 0 17.5 5.5",
  mic: "M12 2 A3 3 0 0 0 9 5 V11 A3 3 0 0 0 15 11 V5 A3 3 0 0 0 12 2 M5 10 A7 7 0 0 0 19 10 M12 18 V22 M8 22 H16",
  wave: "M3 12 H6 M9 6 V18 M13 3 V21 M17 8 V16 M21 11 H22",
  plus: "M12 4 V20 M4 12 H20",
  copy: "M8 8 H21 V21 H8 Z M4 16 V3 H17",
  check: "M4 12 L10 18 L20 5",
  close: "M4 4 L20 20 M20 4 L4 20",
  chevron: "M8 4 L16 12 L8 20",
  play: "M6 3 L20 12 L6 21 Z",
  volume: "M3 9 H7 L12 4 V20 L7 15 H3 Z M16 9 A4 4 0 0 1 16 15 M19 6 A8 8 0 0 1 19 18",
  keyboard: "M2 6 H22 V18 H2 Z M6 10 H6.1 M10 10 H10.1 M14 10 H14.1 M18 10 H18.1 M7 14 H17",
  monitor: "M2 4 H22 V17 H2 Z M8 21 H16 M12 17 V21",
  globe: "M12 2 A10 10 0 1 0 12 22 A10 10 0 1 0 12 2 M2 12 H22 M12 2 C8 6 8 18 12 22 C16 18 16 6 12 2",
  refresh: "M20 7 A9 9 0 1 0 21 13 M20 2 V7 H15",
  download: "M12 3 V15 M6 10 L12 16 L18 10 M4 20 H20",
  trash: "M4 6 H20 M9 6 V3 H15 V6 M6 6 L7 21 H17 L18 6 M10 10 V17 M14 10 V17",
  edit: "M4 20 H8 L20 8 L16 4 L4 16 Z M14 6 L18 10",
  wifi: "M2 8 A15 15 0 0 1 22 8 M5 12 A10 10 0 0 1 19 12 M8 16 A5 5 0 0 1 16 16 M12 20 H12.1",
  mouse: "M12 2 A7 7 0 0 0 5 9 V15 A7 7 0 0 0 19 15 V9 A7 7 0 0 0 12 2 M12 2 V10 M5 10 H19",
  eye: "M2 12 C6 6 18 6 22 12 C18 18 6 18 2 12 Z M12 9 A3 3 0 1 0 12 15 A3 3 0 1 0 12 9",
  vote: "M4 20 H20 M6 16 L10 10 L14 13 L19 5 M14 5 H19 V10",
  text: "M3 5 H21 M12 5 V20 M8 20 H16",
  brush: "M4 20 C4 14 10 18 10 12 C10 8 8 6 12 3 C17 3 20 7 19 11 C18 15 13 14 13 18 C13 20 11 20 10 20 Z",
  layers: "M12 2 L22 8 L12 14 L2 8 Z M2 13 L12 19 L22 13 M2 18 L12 24 L22 18",
  bolt: "M13 2 L4 14 H11 L10 22 L20 9 H13 Z",
  shield: "M12 2 L21 6 V12 C21 17 17 21 12 22 C7 21 3 17 3 12 V6 Z M8 12 L11 15 L16 9",
  info: "M12 2 A10 10 0 1 0 12 22 A10 10 0 1 0 12 2 M12 11 V17 M12 7 H12.1",
  image: "M2 4 H22 V20 H2 Z M2 16 L8 10 L13 15 L17 11 L22 16 M7 8 A1.5 1.5 0 1 0 7 11 A1.5 1.5 0 1 0 7 8",
  clock: "M12 2 A10 10 0 1 0 12 22 A10 10 0 1 0 12 2 M12 6 V12 L17 15",
} as const;

export type GlyphName = keyof typeof PATHS;

export function Glyph({ name, size = 22, stroke = 2.6 }: { name: GlyphName; size?: number; stroke?: number }) {
  return (
    <svg
      className="glyph"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
