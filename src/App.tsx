// 控制面板 3.3：单主页 + 设置页两态。侧栏取消，其余一切设置收进设置页；
// 设置入口是主页右上角的齿轮（页面内 Ctrl+E 也能切，托盘的旧页面名照旧能跳）。
// 数据流沿用旧版：启动读设置和状态 → 改设置先更新界面，停手 300ms 后整份写回 Rust（Rust 修正的值回填）；
// 信号开关在托盘或快捷键处变化时由 "pings://enabled" 推过来。
import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { DEFAULT_THEME, getStatus, onCloud, onEnabled, onNavigate, readSettings, setEnabled, writeSettings, type KeyClash, type Settings, type Status } from "./api";
import { PermissionCard } from "./components/PermissionCard";
import { Home } from "./ui/Home";
import { SettingsView, type SettingsTab } from "./ui/Settings";
import "./ui/theme.css";
import "./app.css";

type Page = "home" | "settings";

/** 托盘里的老页面名照旧能跳：队伍类回主页，其余进设置页的对应分区。 */
const NAV_FALLBACK: Record<string, { page: Page; tab: SettingsTab }> = {
  home: { page: "home", tab: "common" },
  settings: { page: "settings", tab: "common" },
  team: { page: "home", tab: "common" },
  pings: { page: "settings", tab: "pings" },
  signals: { page: "settings", tab: "pings" },
  emotes: { page: "settings", tab: "emotes" },
  shortcuts: { page: "settings", tab: "keys" },
  features: { page: "settings", tab: "team" },
  voice: { page: "settings", tab: "voice" },
  about: { page: "settings", tab: "about" },
};

const SAVE_DELAY_MS = 300;

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [clashes, setClashes] = useState<KeyClash[]>([]);
  const [page, setPage] = useState<Page>("home");
  const [tab, setTab] = useState<SettingsTab>("common");
  const [error, setError] = useState<string | null>(null);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latest = useRef<Settings | null>(null);

  useEffect(() => {
    readSettings().then((s) => { latest.current = s; setSettings(s); }, (e) => setError(String(e)));
    getStatus().then(setStatus, (e) => setError(String(e)));
    const off = onEnabled((on) => setStatus((s) => (s ? { ...s, enabled: on } : s)));
    // 云端服务器可达性：连不上时主页电源区显示红色「未连接」
    const offCloud = onCloud((ok) => setStatus((s) => (s ? { ...s, cloud: ok } : s)));
    const offNav = onNavigate((p) => {
      const to = NAV_FALLBACK[p];
      if (to) { setPage(to.page); setTab(to.tab); }
    });
    return () => { void off.then((f) => f()); void offNav.then((f) => f()); void offCloud.then((f) => f()); };
  }, []);

  // Ctrl+E：主页 ↔ 设置；Esc：设置页退回主页。窗口有焦点时有效（和页面里的输入框互不打架）。
  // 录快捷键时 KeyCapture 在捕获阶段就把 Esc 拦掉，不会误触发返回。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && !e.altKey && !e.shiftKey && (e.key === "e" || e.key === "E")) {
        e.preventDefault();
        setPage((p) => (p === "settings" ? "home" : "settings"));
      }
      if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.isComposing) {
        setPage((p) => (p === "settings" ? "home" : p));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 主题：六套配色由用户明确选择，不再跟随系统；切主题只改 html 上的 data-theme。
  useEffect(() => {
    document.documentElement.dataset.theme = settings?.theme ?? DEFAULT_THEME;
  }, [settings?.theme]);

  const change = useCallback((patch: Partial<Settings>) => {
    if (!latest.current) return;
    latest.current = { ...latest.current, ...patch };
    setSettings(latest.current);
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      saveTimer.current = null;
      if (!latest.current) return;
      writeSettings(latest.current).then((saved) => {
        if (saveTimer.current) return;
        latest.current = saved.settings;
        setSettings(saved.settings);
        setClashes(saved.clashes);
        setError(null);
      }, (e) => setError(String(e)));
    }, SAVE_DELAY_MS);
  }, []);

  const saved = useCallback((s: Settings) => {
    const pending = saveTimer.current !== null && latest.current;
    latest.current = pending
      ? { ...pending, customEmotes: s.customEmotes, emoteWheel: s.emoteWheel, clickEmote: s.clickEmote }
      : s;
    setSettings(latest.current);
  }, []);

  const toggle = (on: boolean) => {
    setStatus((s) => (s ? { ...s, enabled: on } : s));
    void setEnabled(on);
  };

  // 空白拖窗：在非交互元素上按住鼠标就能拖动窗口（替代标题栏）。
  const onDragStart = useCallback((e: React.MouseEvent) => {
    const t = e.target as HTMLElement;
    // 交互元素不触发拖窗：按钮、输入框、下拉、链接、标签、快捷键、tab、滑块、带 no-drag 标记的
    if (t.closest("button,input,select,textarea,a,label,kbd,[role=tab],[role=button],[data-no-drag]")) return;
    void getCurrentWindow().startDragging();
  }, []);

  return (
    <div className="v-shell" onMouseDown={onDragStart}>
      <main className="v-main">
        {error && <p className="v-error">{error}</p>}
        {status?.needsPermission && status.inputError && <PermissionCard reason={status.inputError} />}
        {settings && status ? (
          page === "home" ? (
            <Home
              settings={settings}
              status={status}
              onToggle={toggle}
              onOpenSettings={(t) => { setPage("settings"); setTab(t); }}
            />
          ) : (
            <SettingsView
              settings={settings}
              status={status}
              clashes={clashes}
              onChange={change}
              onSaved={saved}
              tab={tab}
              onTab={setTab}
              onBack={() => setPage("home")}
            />
          )
        ) : !error ? <p className="v-note">…</p> : null}
      </main>
    </div>
  );
}
