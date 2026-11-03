// 设置（需求 3.6 的本机部分，V1.1 第二版参考 WhatsApp）：在左栏里打开，不挡住聊天。
// 第一层是个人资料卡片和分组列表：通用（外观、声音、后台运行）、网络（中继、局域网）、聊天记录、备份与恢复、关于；
// 点一组进到这一组的设置，左上角箭头返回。每项改动立即保存；中继地址不合格时显示 Rust 返回的中文错误，列表不变。
import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import type { Identity, NetworkStatus, RelayHealth, Settings } from "../api";
import { Avatar } from "./Avatar";
import { BackupSection } from "./BackupSection";
import { Icon, type IconName } from "./Icon";

const STATE_TEXT: Record<RelayHealth["state"], string> = {
  ok: "已连接",
  connecting: "连接中",
  cooling: "限流冷却中",
  retired: "已停用",
};

const THEME_NAMES: Record<Settings["theme"], string> = { system: "跟随系统", light: "浅色", dark: "深色" };

type Tab = "general" | "network" | "history" | "backup" | "about";
const TABS: { id: Tab; name: string; sub: string; icon: IconName }[] = [
  { id: "general", name: "通用", sub: "主题、声音、后台运行", icon: "sun" },
  { id: "network", name: "网络", sub: "互联网中继、局域网", icon: "globe" },
  { id: "history", name: "聊天记录", sub: "新进群时同步多少消息", icon: "clock" },
  { id: "backup", name: "备份与恢复", sub: "换电脑前导出，换好后导入", icon: "archive" },
  { id: "about", name: "关于钢一下", sub: "版本和玩法", icon: "info" },
];

type Patch = Partial<Omit<Settings, "defaultRelays">>;

interface Props {
  me: Identity | null;
  settings: Settings;
  relays: RelayHealth[] | null | undefined;
  network: NetworkStatus | null;
  /// reject 时显示错误。
  onUpdate: (patch: Patch) => Promise<void>;
  onPreview: (volume: number) => void;
  onOpenProfile: () => void;
}

export function SettingsPanel(p: Props) {
  const [tab, setTab] = useState<Tab | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function update(patch: Patch): Promise<boolean> {
    setError(null);
    try {
      await p.onUpdate(patch);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  }

  const current = TABS.find((t) => t.id === tab);
  if (current) {
    return (
      <>
        <header className="side-head">
          <button
            className="icon-button"
            onClick={() => {
              setTab(null);
              setError(null);
            }}
            aria-label="返回"
            title="返回"
          >
            <Icon name="arrowLeft" size={20} />
          </button>
          <h1 className="small">{current.name}</h1>
        </header>
        <div className="side-scroll settings-detail">
          {tab === "general" && <General {...p} update={update} />}
          {tab === "network" && <Network {...p} update={update} />}
          {tab === "history" && <History {...p} update={update} />}
          {tab === "backup" && <BackupSection />}
          {tab === "about" && <About />}
          {error && <p className="field-error">{error}</p>}
        </div>
      </>
    );
  }

  return (
    <>
      <header className="side-head">
        <h1>设置</h1>
      </header>
      <div className="side-scroll">
        {p.me && (
          <button className="profile-card" onClick={p.onOpenProfile}>
            <Avatar fingerprint={p.me.fingerprint} name={p.me.nickname} avatar={p.me.avatar} size={64} />
            <span className="profile-card-text">
              <span className="profile-card-name">{p.me.nickname || "还没有昵称"}</span>
              <span className="profile-card-sub">编辑头像和昵称</span>
            </span>
          </button>
        )}
        <nav className="settings-list" aria-label="设置分组">
          {TABS.map((t) => (
            <button key={t.id} className="settings-entry" onClick={() => setTab(t.id)}>
              <Icon name={t.icon} size={21} />
              <span className="entry-text">
                <span>{t.name}</span>
                <small>{t.sub}</small>
              </span>
            </button>
          ))}
        </nav>
      </div>
    </>
  );
}

type SectionProps = Props & { update: (patch: Patch) => Promise<boolean> };

function General({ settings: s, update, onPreview }: SectionProps) {
  const [volume, setVolume] = useState(s.volume);
  return (
    <>
      <section className="settings-section">
        <h2>外观</h2>
        <div className="segmented" role="radiogroup" aria-label="主题">
          {(Object.keys(THEME_NAMES) as Settings["theme"][]).map((t) => (
            <button
              key={t}
              role="radio"
              aria-checked={s.theme === t}
              className={s.theme === t ? "selected" : undefined}
              onClick={() => update({ theme: t })}
            >
              {THEME_NAMES[t]}
            </button>
          ))}
        </div>
      </section>

      <section className="settings-section">
        <h2>声音</h2>
        <div className="settings-card">
          <label className="switch-row">
            <span>叠层时播放心之钢音效</span>
            <input
              type="checkbox"
              role="switch"
              checked={s.soundEnabled}
              onChange={(e) => update({ soundEnabled: e.currentTarget.checked })}
            />
          </label>
          <div className="volume-row">
            <span>音量</span>
            <input
              type="range"
              min={0}
              max={100}
              value={volume}
              disabled={!s.soundEnabled}
              onChange={(e) => setVolume(Number(e.currentTarget.value))}
              onPointerUp={() => update({ volume })}
              onKeyUp={() => update({ volume })}
              aria-label="音量"
            />
            <span className="volume-value">{volume}</span>
            <button className="soft small" onClick={() => onPreview(volume)} disabled={!s.soundEnabled}>
              <Icon name="speaker" size={14} />
              试听
            </button>
          </div>
        </div>
      </section>

      <section className="settings-section">
        <h2>后台运行</h2>
        <p className="settings-note">
          点窗口右上角的关闭按钮只会把窗口收进右下角的托盘，程序继续收消息、叠层照常响。窗口不在眼前时来了新消息，托盘图标会出现红点，任务栏按钮会闪烁。要彻底退出，请右键托盘里的钢一下图标，选「退出」。
        </p>
      </section>
    </>
  );
}

function Network({ settings: s, relays, network, update }: SectionProps) {
  const [newRelay, setNewRelay] = useState("");
  const health = (url: string) => relays?.find((r) => r.url === url);
  const isDefault =
    s.relays.length === s.defaultRelays.length && s.relays.every((u, i) => u === s.defaultRelays[i]);

  async function addRelay() {
    const url = newRelay.trim();
    if (!url) return;
    if (await update({ relays: [...s.relays, url] })) setNewRelay("");
  }

  return (
    <>
      <section className="settings-section">
        <h2>互联网中继</h2>
        <div className="settings-card">
          <label className="switch-row">
            <span>允许经互联网中继联系群友</span>
            <input
              type="checkbox"
              role="switch"
              checked={s.internetAllowed}
              onChange={(e) => update({ internetAllowed: e.currentTarget.checked })}
            />
          </label>
        </div>
        <p className="settings-note">关闭后只能和同一局域网（同一个路由器）里的群友联系。</p>

        <ul className="relay-list" aria-label="中继列表">
          {s.relays.map((url) => {
            const h = health(url);
            const state = !s.internetAllowed ? "off" : (h?.state ?? "connecting");
            return (
              <li key={url}>
                <span className={`relay-dot ${state}`} />
                <span className="relay-url">{url}</span>
                <span className="relay-state">
                  {state === "off" ? "未启用" : STATE_TEXT[state]}
                  {state === "ok" && h?.lastEchoMs != null && ` ${h.lastEchoMs} ms`}
                </span>
                <button
                  className="icon-button small"
                  onClick={() => update({ relays: s.relays.filter((u) => u !== url) })}
                  aria-label={`删除 ${url}`}
                  title="删除"
                >
                  <Icon name="close" size={14} />
                </button>
              </li>
            );
          })}
          {s.relays.length === 0 && <li className="relay-empty">没有中继，只能经局域网联系。</li>}
        </ul>
        <form
          className="relay-add"
          onSubmit={(e) => {
            e.preventDefault();
            addRelay();
          }}
        >
          <input
            value={newRelay}
            onChange={(e) => setNewRelay(e.currentTarget.value)}
            placeholder="wss://中继地址"
            aria-label="新中继地址"
          />
          <button type="submit" className="soft" disabled={!newRelay.trim()}>
            添加
          </button>
          <button type="button" className="ghost" disabled={isDefault} onClick={() => update({ relays: s.defaultRelays })}>
            恢复默认
          </button>
        </form>
      </section>

      <section className="settings-section">
        <h2>局域网</h2>
        {network?.lanDegraded ? (
          <p className="settings-warn">
            局域网端口被其他软件占用，现在用的是临时端口 {network.lanPort}，收不到同一局域网群友的消息，只能经中继联系。关掉占用端口
            47480 的软件后重开本程序即可恢复。
          </p>
        ) : (
          <p className="settings-note">
            局域网端口 {network?.lanPort ?? "…"}。第一次运行时 Windows 防火墙会询问，请勾选「专用网络」并允许，否则同一局域网的群友找不到你。
          </p>
        )}
      </section>
    </>
  );
}

function History({ settings: s, update }: SectionProps) {
  const [limits, setLimits] = useState({ lan: String(s.historyLimitLan), relay: String(s.historyLimitRelay) });
  return (
    <section className="settings-section">
      <h2>新进群时同步多少聊天</h2>
      <p className="settings-note">
        新进一个群时，最多从群友那里同步最近多少条消息。层数和群成员总是完整同步，不受这个数影响。中继每秒只能传几条，所以经中继的少一些。
      </p>
      <div className="settings-card limit-rows">
        <label className="limit-row">
          <span>经局域网</span>
          <input
            type="number"
            min={0}
            max={100000}
            value={limits.lan}
            onChange={(e) => setLimits({ ...limits, lan: e.currentTarget.value })}
            onBlur={() => update({ historyLimitLan: Number(limits.lan) || 0 })}
          />
          <span>条</span>
        </label>
        <label className="limit-row">
          <span>经中继</span>
          <input
            type="number"
            min={0}
            max={100000}
            value={limits.relay}
            onChange={(e) => setLimits({ ...limits, relay: e.currentTarget.value })}
            onBlur={() => update({ historyLimitRelay: Number(limits.relay) || 0 })}
          />
          <span>条</span>
        </label>
      </div>
      <p className="settings-note">要清空某个群的聊天记录，到那个群的「群资料」里操作。</p>
    </section>
  );
}

function About() {
  // 版本号由 Rust 侧读安装包里的配置（tauri.conf.json 的 version），不在界面里另写一份
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);
  return (
    <section className="settings-section">
      <h2>关于钢一下</h2>
      <p className="settings-note">
        版本 {version || "…"}。不用服务器的开黑群聊：同一局域网直接互联，跨网经公共中继转发，消息全程加密，只有拿到房间码的人能看。
      </p>
      <p className="settings-note">在群里 5 秒内连发 3 条消息叠一层心之钢（规则群主可改），头像右下角是层数。</p>
    </section>
  );
}
