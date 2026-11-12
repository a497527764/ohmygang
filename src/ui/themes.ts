// 六套主题：3 深色 + 3 浅色，由用户明确选择（不再跟随系统）。
// id 必须和 theme.css 的 [data-theme="..."] 以及 Rust settings2::THEMES 完全一致。
export type Theme =
  | "dark-ice" | "dark-carbon" | "dark-plum"
  | "light-paper" | "light-sand" | "light-mint";

export interface ThemePreset {
  id: Theme;
  label: string;
  group: "dark" | "light";
  /** 色卡用的三个代表色：背景、卡片、主强调。 */
  bg: string;
  card: string;
  accent: string;
  orange: string;
}

export const THEME_PRESETS: ThemePreset[] = [
  { id: "dark-ice",    label: "深空蓝", group: "dark",  bg: "#070a11", card: "#0b131f", accent: "#00e7ff", orange: "#ff6b1a" },
  { id: "dark-carbon", label: "石墨绿", group: "dark",  bg: "#08090b", card: "#101317", accent: "#b8ff45", orange: "#ff7a1a" },
  { id: "dark-plum",   label: "电光紫", group: "dark",  bg: "#0a0710", card: "#140e22", accent: "#b98cff", orange: "#ff8a3d" },
  { id: "light-paper", label: "冷白纸", group: "light", bg: "#e8edf4", card: "#ffffff", accent: "#0093ac", orange: "#d9540a" },
  { id: "light-sand",  label: "暖砂棕", group: "light", bg: "#efe7da", card: "#fffaf1", accent: "#9a4b00", orange: "#d2460a" },
  { id: "light-mint",  label: "薄荷绿", group: "light", bg: "#e3efeb", card: "#fbfffd", accent: "#0b7563", orange: "#dc5208" },
];

export const DEFAULT_THEME: Theme = "dark-ice";
export const isLightTheme = (t: string): boolean => t.startsWith("light-");
