// 控制面板（2.0）：左边是分页，右边是当前页：「队伍」「信号」「外观」「关于」。
// 数据流：启动时读设置和状态 → 改设置时先更新界面，停手 300 毫秒后整份交给 Rust 存（Rust 修正后的值再回填）；
// 信号开关在托盘或暂停快捷键那边变了，Rust 推送过来，这里跟着变。
import { useCallback, useEffect, useRef, useState } from "react";
import { getStatus, onEnabled, readSettings, setEnabled, writeSettings, type Settings, type Status } from "./api";
import { AboutPage } from "./components/AboutPage";
import { AppearancePage } from "./components/AppearancePage";
import { Icon, type IconName } from "./components/Icon";
import { PingsPage } from "./components/PingsPage";
import { TeamPage } from "./components/TeamPage";
import { textureUrl } from "./shared/pings";
import "./app.css";

type Page = "team" | "pings" | "appearance" | "about";
const PAGES: { id: Page; label: string; icon: IconName }[] = [
  { id: "team", label: "队伍", icon: "users" },
  { id: "pings", label: "信号", icon: "target" },
  { id: "appearance", label: "外观", icon: "sun" },
  { id: "about", label: "关于", icon: "info" },
];

const SAVE_DELAY_MS = 300;

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [page, setPage] = useState<Page>("team");
  const [error, setError] = useState<string | null>(null);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    readSettings().then(setSettings, (e) => setError(String(e)));
    getStatus().then(setStatus, (e) => setError(String(e)));
    const off = onEnabled((on) => setStatus((s) => (s ? { ...s, enabled: on } : s)));
    return () => void off.then((f) => f());
  }, []);

  // 主题：选了浅色或深色就写到 <html data-theme>，跟随系统时去掉，由 CSS 按系统设置切换。
  useEffect(() => {
    const root = document.documentElement;
    if (!settings || settings.theme === "system") delete root.dataset.theme;
    else root.dataset.theme = settings.theme;
  }, [settings?.theme]);

  const change = useCallback((patch: Partial<Settings>) => {
    setSettings((s) => {
      if (!s) return s;
      const next = { ...s, ...patch };
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        writeSettings(next).then(
          (saved) => {
            setSettings(saved);
            setError(null);
          },
          (e) => setError(`设置没存上：${String(e)}`),
        );
      }, SAVE_DELAY_MS);
      return next;
    });
  }, []);

  const toggle = (on: boolean) => {
    setStatus((s) => (s ? { ...s, enabled: on } : s));
    void setEnabled(on);
  };

  return (
    <div className="shell">
      <nav className="side">
        <div className="brand">
          <img src={textureUrl("generic_ping")} alt="" width={28} height={28} />
          <span>钢一下</span>
        </div>
        {PAGES.map((p) => (
          <button
            key={p.id}
            className={`nav-item${page === p.id ? " active" : ""}`}
            onClick={() => setPage(p.id)}
            aria-current={page === p.id ? "page" : undefined}
          >
            <Icon name={p.icon} />
            {p.label}
          </button>
        ))}
        {status && (
          <p className={`side-status${status.enabled && !status.inputError ? " on" : ""}`}>
            <span className="dot" />
            {status.inputError ? "信号不可用" : status.enabled ? "信号开启中" : "信号已暂停"}
          </p>
        )}
      </nav>
      <main className="content">
        {error && <p className="banner error">{error}</p>}
        {settings && status ? (
          <>
            {page === "team" && <TeamPage settings={settings} onSettings={change} />}
            {page === "pings" && (
              <PingsPage settings={settings} status={status} onChange={change} onToggle={toggle} />
            )}
            {page === "appearance" && <AppearancePage settings={settings} onChange={change} />}
            {page === "about" && <AboutPage />}
          </>
        ) : (
          !error && <p className="loading">正在读取设置…</p>
        )}
      </main>
    </div>
  );
}
