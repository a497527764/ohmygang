// 主页：一块电源主控（设置齿轮贴在其右侧内），一排快捷键键帽，下面整块给队伍。
// 窄高窗口：建队 / 进队表单在上，「加入过的队伍」竖排在下，历史吃满剩余高度并可滚动。
// 数据：队伍历史来自 Rust 的 team_recent，自动归队由 Rust 启动时做。
import { useCallback, useEffect, useState } from "react";
import {
  onTeamChanged, teamCancel, teamCreate, teamDissolve, teamForget, teamJoin, teamLeave,
  teamModerate, teamRecent, teamView, type Member, type RecentTeam, type RecentTeams,
  type Settings, type Status, type TeamView, type TriggerKey,
} from "../api";
import { hotkeyParts, vkLabel } from "../shared/keys";
import { Avatar } from "../components/Avatar";
import { Glyph, type GlyphName } from "./Glyph";
import { MicWave } from "./MicWave";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import type { SettingsTab } from "./Settings";

interface Props {
  settings: Settings;
  status: Status;
  onToggle: (on: boolean) => void;
  onOpenSettings: (tab: SettingsTab) => void;
}

const triggerLabel = (t: TriggerKey | "off"): string => {
  if (t === "off") return "关";
  if (typeof t === "object") return vkLabel(t.vk);
  return t === "alt" ? "ALT" : t === "ctrl" ? "CTRL" : t === "shift" ? "SHIFT" : t === "win" ? "WIN" : t === "capslock" ? "CAPS" : t === "mouse4" ? "侧键1" : t === "mouse5" ? "侧键2" : String(t).toUpperCase();
};

/** 成员列表「连接」列的状态：自己的 path 是 null（没经过中继），也算在线；离线才不是。 */
const linkState = (m: { path: "relay" | "offline" | null }): "on" | "off" =>
  m.path === "offline" ? "off" : "on";

/** 队名后面跟的队伍码：优先云端 6 位短码；老队伍拿 GT2 长码的头一段，保证同名队伍能区分开。 */
const codeLabel = (r: { shortCode: string | null; code: string }): string =>
  r.shortCode ?? r.code.split("-").slice(0, 2).join("-");

export function Home({ settings: s, status, onToggle, onOpenSettings }: Props) {
  const [view, setView] = useState<TeamView | null>(null);
  const [recent, setRecent] = useState<RecentTeam[]>([]);
  // 当前队伍现在几个人在线（云端统计；问不到是 null）
  const [currentOnline, setCurrentOnline] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // 队名默认留空（输入框里不出现任何占位字），不填时后端用默认名建队。
  const [title, setTitle] = useState("");
  const [code, setCode] = useState("");
  const [confirm, setConfirm] = useState<"leave" | "dissolve" | null>(null);
  // 右键菜单：记下触发位置和菜单项，点外面自动关
  const [menu, setMenu] = useState<{ anchor: DOMRect; items: MenuItem[] } | null>(null);

  const refreshRecent = useCallback(() => {
    teamRecent().then(
      (r: RecentTeams) => { setRecent(r.rooms); setCurrentOnline(r.currentOnline); },
      () => { setRecent([]); setCurrentOnline(null); },
    );
  }, []);

  const load = useCallback(() => {
    teamView().then(setView, () => setView(null));
    refreshRecent();
  }, [refreshRecent]);
  useEffect(() => {
    load();
    const off = onTeamChanged(load);
    return () => void off.then((f) => f());
  }, [load]);

  const run = (p: Promise<TeamView>) => {
    setBusy(true);
    p.then((v) => { setView(v); setError(null); }, (e) => setError(String(e)))
      // 退出队伍后历史列表要立刻重拉一次：不然「加入过的队伍」还是进队时的快照，看不到刚退的这条
      .finally(() => { setBusy(false); refreshRecent(); });
  };

  /** 右键历史里的一行：删掉本机记录（队伍本身还在，别人照样能用队伍码进来）。 */
  const forget = (roomId: string) => {
    setBusy(true);
    teamForget(roomId).then(
      (r) => { setRecent(r.rooms); setCurrentOnline(r.currentOnline); setError(null); },
      (e) => setError(String(e)),
    ).finally(() => setBusy(false));
  };

  /** 右键成员：房主才能踢人 / 拉黑。 */
  const moderate = (m: Member, block: boolean) => {
    setError(null);
    run(teamModerate(m.memberId, block));
  };

  const on = status.enabled && !status.inputError;
  // 云端服务器（中继所在的服务器）的连接状态：
  // connecting = 刚启动还在连（白色「连接中」），ok = 连上了，down = 连不上（红色「未连接」）
  const cloudDown = status.cloudState === "down" && !status.inputError;
  const cloudConnecting = status.cloudState === "connecting" && !status.inputError && !cloudDown;
  // 电源区上的三个字：不可用 > 未连接 > 连接中 > 在线 > 离线
  const powerText = status.inputError
    ? "不可用"
    : cloudDown
      ? "未连接"
      : cloudConnecting
        ? "连接中"
        : on
          ? "在线"
          : "离线";
  // 阵容表固定留到 5 个位置，空位是虚线格，能把卡片高度摊满
  const slots = Math.max(0, 5 - (view?.members.length ?? 0));
  const team = view?.team ?? null;
  const code_ = team?.code ?? "";
  const copy = () => {
    if (!code_) return;
    void navigator.clipboard.writeText(code_).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    });
  };

  // 后端的历史列表刻意排除当前队伍，这里把它合成进第一行并着重显示；
  // 有队伍时历史区也常驻，否则「当前队伍」根本没地方可高亮。
  const current: (RecentTeam & { current: boolean }) | null = team
    ? {
        code: code_,
        // 当前队伍不能从历史里删（后端也会拦），这里只是占个位
        roomId: "",
        shortCode: null,
        title: team.title ?? null,
        isOwner: team.isOwner,
        // 在线人数：云端统计优先（问不到就按 15 秒内报过状态的队友数算）
        online: currentOnline ?? view?.members.filter((m) => m.isMe || m.status).length ?? 0,
        members: view?.members.length ?? 0,
        status: team.status,
        current: true,
      }
    : null;
  const history: (RecentTeam & { current: boolean })[] = [
    ...(current ? [current] : []),
    ...recent
      .filter((r) => r.code !== current?.code)
      .slice(0, 10)
      .map((r) => ({ ...r, current: false })),
  ];

  const keys: {
    id: string; icon: GlyphName; label: string; key: string; verb: string; tab: SettingsTab;
  }[] = [
    { id: "ping", icon: "target", label: "信号", key: triggerLabel(s.trigger), verb: "按住拖动", tab: "pings" },
    { id: "emote", icon: "smile", label: "表情", key: triggerLabel(s.emoteTrigger), verb: "按住拖动", tab: "emotes" },
    { id: "talk", icon: "mic", label: "语音", key: vkLabel(s.talkKey), verb: "按住说话", tab: "voice" },
    { id: "text", icon: "text", label: "打字", key: hotkeyParts(s.textHotkey).join("+"), verb: "按下", tab: "keys" },
  ];

  return (
    <div className="v-home">
      <section className={`v-power${on && !cloudDown && !cloudConnecting ? " on" : ""}${status.inputError || cloudDown ? " bad" : ""}`}>
        <button
          type="button"
          className="v-power-btn"
          aria-label={powerText}
          aria-pressed={on}
          onClick={() => onToggle(!on)}
        >
          <span className="v-dial">
            <Glyph name="power" size={40} stroke={2.2} />
          </span>
          <span className="v-power-copy">
            <b>{powerText}</b>
            {status.inputError && <span className="v-power-hint">{status.inputError}</span>}
            {!status.inputError && !cloudDown && status.relayMs != null && (
              <span className="v-power-hint">服务器 {status.relayMs} ms</span>
            )}
            {cloudDown && <span className="v-power-hint">连不上中继服务器，检查网络或梯子</span>}
          </span>
          <MicWave live={s.voice} icon bars={17} max={54} className="v-wave" />
        </button>

        <button type="button" className="v-top-gear" onClick={() => onOpenSettings("common")}
          title="设置 · Ctrl+E" aria-label="设置">
          <Glyph name="gear" size={22} />
        </button>
      </section>

      <section className="v-keys">
        {keys.map((k) => (
          <button type="button" key={k.id} className="v-keycap" onClick={() => onOpenSettings(k.tab)}>
            <span className="v-keycap-name"><Glyph name={k.icon} size={14} />{k.label}</span>
            <span className="v-keycap-foot">
              <kbd>{k.key}</kbd>
              <span className="v-keycap-verb">{k.verb}</span>
            </span>
          </button>
        ))}
      </section>

      <section className="v-squad">
        <div className="v-squad-head">
          <Glyph name="users" size={18} />
          <b>队伍</b>
          {team && <span className="v-squad-name">{team.title ?? code_}</span>}
          <span className="v-page-spacer" />
          {team && (
            <button type="button" className="v-code" onClick={copy} title="复制队伍码">
              <span className="v-code-text">{code_}</span>
              <Glyph name={copied ? "check" : "copy"} size={16} />
            </button>
          )}
        </div>

        <div className="v-squad-body">
          {error && <p className="v-error">{error}</p>}

          {team && (
            <>
              <div className="v-members">
                <div className="v-members-head" aria-hidden="true">
                  <span>成员</span>
                  <span>平台</span>
                  <span>连接</span>
                </div>
                {view?.members.map((m) => (
                  <div key={m.memberId}
                    className={`v-member${m.path === "relay" ? " p1" : ""}${m.path === "offline" ? " off" : ""}`}
                    title={team?.isOwner && !m.isMe
                      ? `${m.nickname || m.fingerprint}（右键：踢出 / 拉黑）`
                      : (m.nickname || m.fingerprint)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      // 只有房主能处置队员；对自己不行
                      if (!team?.isOwner || m.isMe) return;
                      setMenu({
                        anchor: e.currentTarget.getBoundingClientRect(),
                        items: [
                          {
                            key: "kick",
                            label: "踢出队伍",
                            danger: true,
                            onSelect: () => moderate(m, false),
                          },
                          {
                            key: "block",
                            label: "拉黑（之后加不进来）",
                            danger: true,
                            onSelect: () => moderate(m, true),
                          },
                        ],
                      });
                    }}>
                    <span className="v-member-main">
                      <Avatar fingerprint={m.fingerprint} name={m.nickname} avatar={m.avatar} size={28} />
                      <b>{m.isMe ? `${m.nickname || m.fingerprint}（我自己）` : (m.nickname || m.fingerprint)}</b>
                      {m.isOwner && <span className="v-chip">队长</span>}
                    </span>
                    <span className="v-member-os">{m.platform ?? "—"}</span>
                    <span className={`v-path ${linkState(m)}`}>{linkState(m) === "on" ? "在线" : "离线"}</span>
                  </div>
                ))}
                {Array.from({ length: slots }, (_, i) => (
                  <div key={`slot-${i}`} className="v-member empty" aria-hidden="true">
                    <span className="v-member-main">
                      <span className="v-slot-dot" />
                      <span className="v-member-gap">空位</span>
                    </span>
                    <span className="v-member-os">—</span>
                    <span className="v-path">—</span>
                  </div>
                ))}
              </div>
              <div className="v-squad-foot">
                {team.status === "joining" ? (
                  <>
                    <b className="v-squad-confirm">正在加入队伍…</b>
                    <button className="v-btn ghost" disabled={busy} onClick={() => run(teamCancel())}>
                      <Glyph name="close" size={16} />取消
                    </button>
                  </>
                ) : confirm === null ? (
                  <>
                    <button className="v-btn ghost" disabled={busy} onClick={() => setConfirm("leave")}>
                      <Glyph name="close" size={16} />退出
                    </button>
                    {team.isOwner && (
                      <button className="v-btn danger" disabled={busy} onClick={() => setConfirm("dissolve")}>
                        <Glyph name="trash" size={16} />解散
                      </button>
                    )}
                  </>
                ) : (
                  <>
                    <b className="v-squad-confirm">{confirm === "leave" ? "退出队伍？" : "解散队伍？"}</b>
                    <button className="v-btn" disabled={busy}
                      onClick={() => { run(confirm === "leave" ? teamLeave() : teamDissolve()); setConfirm(null); }}>
                      <Glyph name="check" size={16} />确定
                    </button>
                    <button className="v-btn ghost" onClick={() => setConfirm(null)}>
                      <Glyph name="close" size={16} />取消
                    </button>
                  </>
                )}
              </div>
            </>
          )}

          {!team && (
            <div className="v-squad-setup">
              <div className="v-squad-forms">
                <form className="v-squad-form" onSubmit={(e) => { e.preventDefault(); run(teamCreate(title.trim() || "开黑")); }}>
                  <label className="v-field-label" htmlFor="team-title">建立队伍</label>
                  <div className="v-field-row">
                    <input id="team-title" className="v-title-input" value={title} maxLength={64}
                      onChange={(e) => setTitle(e.target.value)} />
                    <button className="v-btn ghost" type="submit" disabled={busy}>
                      <Glyph name="plus" size={16} />建立
                    </button>
                  </div>
                </form>
                <form className="v-squad-form" onSubmit={(e) => { e.preventDefault(); run(teamJoin(code)); }}>
                  <label className="v-field-label" htmlFor="team-code">加入队伍</label>
                  <div className="v-field-row">
                    <input id="team-code" className="v-code-input" value={code} placeholder="6 位队伍码"
                      spellCheck={false} autoCapitalize="characters" maxLength={64}
                      onChange={(e) => setCode(e.target.value.toUpperCase().replace(/[^0-9A-Z-]/g, ""))} />
                    <button className="v-btn orange" type="submit" disabled={busy || !code.trim()}>
                      <Glyph name="chevron" size={16} />加入
                    </button>
                  </div>
                </form>
              </div>
            </div>
          )}

          {/* 历史常驻：当前队伍置顶并橙色着重显示，其余最多 10 条、可视 5 行 / 有队伍时 3 行 */}
          <div className={`v-recent${team ? " compact" : ""}`}>
            <div className="v-recent-title">
              <Glyph name="clock" size={16} />
              <b>加入过的队伍</b>
            </div>
            <div className="v-recent-list">
              {history.map((r) =>
                r.current ? (
                  <div key={r.code} className="v-recent-row current" aria-current="true">
                    <Glyph name="users" size={16} />
                    <span className="v-recent-main">
                      <b>{r.title ?? "队伍"}</b>
                      <span className="v-recent-code">{codeLabel(r)}</span>
                      {r.isOwner && <span className="v-chip">队长</span>}
                      <span className="v-recent-flag">{team?.status === "active" ? "当前" : "加入中"}</span>
                    </span>
                    <span className="v-recent-num" title="在线 / 总人数">{r.online ?? "—"}/{r.members}</span>
                    <span className="v-recent-go"><Glyph name="check" size={16} /></span>
                  </div>
                ) : (
                  <button type="button" key={r.code} className="v-recent-row" disabled={busy}
                    title="右键：删除这条记录"
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setMenu({
                        anchor: e.currentTarget.getBoundingClientRect(),
                        items: [
                          {
                            key: "forget",
                            label: "删除这条记录",
                            danger: true,
                            onSelect: () => forget(r.roomId),
                          },
                        ],
                      });
                    }}
                    onClick={() => run(teamJoin(r.code))}>
                    <Glyph name="users" size={16} />
                    <span className="v-recent-main">
                      <b>{r.title ?? "队伍"}</b>
                      <span className="v-recent-code">{codeLabel(r)}</span>
                      {r.isOwner && <span className="v-chip">队长</span>}
                    </span>
                    <span className="v-recent-num" title="在线 / 总人数">{r.online ?? "—"}/{r.members}</span>
                    <span className="v-recent-go"><Glyph name="chevron" size={16} /></span>
                  </button>
                ),
              )}
              {/* 不在队伍里且不足 5 条时补虚线格，把「预留 5 条」画出来而不是留一块黑 */}
              {!team && Array.from({ length: Math.max(0, 5 - Math.min(history.length, 10)) }, (_, i) => (
                <div key={`slot-${i}`} className="v-recent-slot" aria-hidden="true" />
              ))}
            </div>
          </div>
        </div>
      </section>

      {menu && (
        <ContextMenu anchor={menu.anchor} items={menu.items} onClose={() => setMenu(null)} />
      )}
    </div>
  );
}
