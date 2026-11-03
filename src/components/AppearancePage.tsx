// 「外观」页：控制面板的主题。透明窗口上的轮盘和信号是英雄联盟的样子，不跟主题变。
import type { Settings, Theme } from "../api";

const THEMES: { value: Theme; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];

export function AppearancePage({ settings, onChange }: { settings: Settings; onChange: (p: Partial<Settings>) => void }) {
  return (
    <div className="page">
      <h1>外观</h1>
      <section className="card">
        <h2>主题</h2>
        <div className="choices" role="radiogroup" aria-label="主题">
          {THEMES.map((t) => (
            <label key={t.value} className={`choice${settings.theme === t.value ? " active" : ""}`}>
              <input type="radio" name="theme" checked={settings.theme === t.value} onChange={() => onChange({ theme: t.value })} />
              {t.label}
            </label>
          ))}
        </div>
      </section>
    </div>
  );
}
