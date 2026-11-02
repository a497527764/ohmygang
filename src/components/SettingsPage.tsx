// 设置页（需求 3.6 的本机部分）：昵称、心之钢音效、网络（允许互联网、中继列表）、聊天记录条数、备份与恢复、局域网提示。
// 每项改动立即保存；中继地址不合格时显示 Rust 返回的中文错误，列表不变。
import { useState } from "react";
import type { Identity, NetworkStatus, RelayHealth, Settings } from "../api";
import { Avatar } from "./Avatar";
import { BackupSection } from "./BackupSection";
import { Icon } from "./Icon";

const STATE_TEXT: Record<RelayHealth["state"], string> = {
  ok: "已连接",
  connecting: "连接中",
  cooling: "限流冷却中",
  retired: "已停用",
};

interface Props {
  me: Identity | null;
  settings: Settings;
  relays: RelayHealth[] | null | undefined;
  network: NetworkStatus | null;
  onClose: () => void;
  onEditNickname: () => void;
  /// reject 时显示错误。
  onUpdate: (patch: Partial<Omit<Settings, "defaultRelays">>) => Promise<void>;
  onPreview: (volume: number) => void;
}

export function SettingsPage(p: Props) {
  const { settings: s } = p;
  const [newRelay, setNewRelay] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [volume, setVolume] = useState(s.volume);
  const [limits, setLimits] = useState({ lan: String(s.historyLimitLan), relay: String(s.historyLimitRelay) });

  async function update(patch: Partial<Omit<Settings, "defaultRelays">>): Promise<boolean> {
    setError(null);
    try {
      await p.onUpdate(patch);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  }

  async function addRelay() {
    const url = newRelay.trim();
    if (!url) return;
    if (await update({ relays: [...s.relays, url] })) setNewRelay("");
  }

  const health = (url: string) => p.relays?.find((r) => r.url === url);
  const isDefault =
    s.relays.length === s.defaultRelays.length && s.relays.every((u, i) => u === s.defaultRelays[i]);

  return (
    <div className="overlay" onMouseDown={p.onClose}>
      <div
        className="settings"
        role="dialog"
        aria-label="设置"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") p.onClose();
        }}
      >
        <header className="settings-head">
          <h2>设置</h2>
          <button className="icon-button" onClick={p.onClose} aria-label="关闭" autoFocus>
            <Icon name="close" />
          </button>
        </header>

        <section className="settings-section">
          <h3>我的资料</h3>
          {p.me && (
            <div className="profile">
              <Avatar fingerprint={p.me.fingerprint} name={p.me.nickname} size={48} />
              <div className="profile-text">
                <span className="profile-name">{p.me.nickname || "还没设置昵称"}</span>
                <span className="profile-fp">公钥指纹 {p.me.fingerprint}</span>
              </div>
              <button className="soft" onClick={p.onEditNickname}>
                改昵称
              </button>
            </div>
          )}
          <p className="settings-note">昵称可以和别人重名，群友核对身份时请以公钥指纹为准。</p>
        </section>

        <section className="settings-section">
          <h3>声音</h3>
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
            <button className="soft" onClick={() => p.onPreview(volume)} disabled={!s.soundEnabled}>
              <Icon name="speaker" size={14} />
              试听
            </button>
          </div>
        </section>

        <section className="settings-section">
          <h3>网络</h3>
          <label className="switch-row">
            <span>允许经互联网中继联系群友</span>
            <input
              type="checkbox"
              role="switch"
              checked={s.internetAllowed}
              onChange={(e) => update({ internetAllowed: e.currentTarget.checked })}
            />
          </label>
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
          {error && <p className="field-error">{error}</p>}
        </section>

        <section className="settings-section">
          <h3>聊天记录</h3>
          <p className="settings-note">新进一个群时，最多从群友那里同步最近多少条聊天。层数和群成员总是完整同步，不受这个数影响。</p>
          <div className="limit-row">
            <label>
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
            <label>
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
        </section>

        <BackupSection />

        <section className="settings-section">
          <h3>局域网</h3>
          {p.network?.lanDegraded ? (
            <p className="settings-warn">
              局域网端口被其他软件占用，现在用的是临时端口 {p.network.lanPort}，收不到同一局域网群友的消息，只能经中继联系。关掉占用端口
              47480 的软件后重开本程序即可恢复。
            </p>
          ) : (
            <p className="settings-note">
              局域网端口 {p.network?.lanPort ?? "…"}。第一次运行时 Windows 防火墙会询问，请勾选「专用网络」并允许，否则同一局域网的群友找不到你。
            </p>
          )}
        </section>
      </div>
    </div>
  );
}
