// 主页：一块电源主控（设置齿轮贴在其右侧内），一排快捷键键帽，下面整块给队伍。
// 窄高窗口：建队 / 进队表单在上，「加入过的队伍」竖排在下，历史吃满剩余高度并可滚动。
// 数据：队伍历史来自 Rust 的 team_recent，自动归队由 Rust 启动时做。
import { useCallback, useEffect, useState } from "react";
import {
  onTeamChanged, teamCancel, teamCreate, teamDissolve, teamJoin, teamLeave,
  teamRecent, teamView, type RecentTeam, type Settings, type Status, type TeamView,
  type TriggerKey,
} from "../api";
import { hotkeyParts, vkLabel } from "../shared/keys";
import { Avatar } from "../components/Avatar";
import { Glyph, type GlyphName } from "./Glyph";
import { MicWave } from "./MicWave";
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

const PATH_TEXT = { relay: "在线", offline: "离线" } as const;

/** 队名后面跟的队伍码：优先云端 6 位短码；老队伍拿 GT2 长码的头一段，保证同名队伍能区分开。 */
const codeLabel = (r: { shortCode: string | null; code: string }): string =>
  r.shortCode ?? r.code.split("-").slice(0, 2).join("-");

export function Home({ settings: s, status, onToggle, onOpenSettings }: Props) {
  const [view, setView] = useState<TeamView | null>(null);
  const [recent, setRecent] = useState<RecentTeam[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // 队名默认留空（输入框里不出现任何占位字），不填时后端用默认名建队。
  const [title, setTitle] = useState("");
  const [code, setCode] = useState("");
  const [confirm, setConfirm] = useState<"leave" | "dissolve" | null>(null);

  const load = useCallback(() => {
    teamView().then(setView, () => setView(null));
    teamRecent().then(setRecent, () => setRecent([]));
  }, []);
  useEffect(() => {
    load();
    const off = onTeamChanged(load);
    return () => void off.then((f) => f());
  }, [load]);

  const run = (p: Promise<TeamView>) => {
    setBusy(true);
    p.then((v) => { setView(v); setError(null); }, (e) => setError(String(e))).finally(() => setBusy(false));
  };

  const on = status.enabled && !status.inputError;
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
        shortCode: null,
        title: team.title ?? null,
        isOwner: team.isOwner,
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
      <section className={`v-power${on ? " on" : ""}${status.inputError ? " bad" : ""}`}>
        <button
          type="button"
          className="v-power-btn"
          aria-label={on ? "下线" : "上线"}
          aria-pressed={on}
          onClick={() => onToggle(!on)}
        >
          <span className="v-dial">
            <Glyph name="power" size={40} stroke={2.2} />
          </span>
          <span className="v-power-copy">
            <b>{status.inputError ? "不可用" : on ? "在线" : "离线"}</b>
            {status.inputError && <span className="v-power-hint">{status.inputError}</span>}
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
          {team?.status === "active" && (
            <button type="button" className="v-code" onClick={copy} title="复制队伍码">
              <span className="v-code-text">{code_}</span>
              <Glyph name={copied ? "check" : "copy"} size={16} />
            </button>
          )}
        </div>

        <div className="v-squad-body">
          {error && <p className="v-error">{error}</p>}

          {team?.status === "active" && (
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
                    title={m.nickname || m.fingerprint}>
                    <span className="v-member-main">
                      <Avatar fingerprint={m.fingerprint} name={m.nickname} avatar={m.avatar} size={28} />
                      <b>{m.isMe ? "我" : (m.nickname || m.fingerprint)}</b>
                      {m.isOwner && <span className="v-chip">队长</span>}
                    </span>
                    <span className="v-member-os">{m.platform ?? "—"}</span>
                    <span className={`v-path${m.path ? ` ${m.path}` : ""}`}>{m.path ? PATH_TEXT[m.path] : "在线"}</span>
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
                {confirm === null ? (
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

          {team?.status === "joining" && (
            <div className="v-squad-foot">
              <b className="v-squad-confirm">加入中 · {team.title ?? code_}</b>
              <button className="v-btn ghost" disabled={busy} onClick={() => run(teamCancel())}>
                <Glyph name="close" size={16} />取消
              </button>
            </div>
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
                    <span className="v-recent-num">{r.members} 人</span>
                    <span className="v-recent-go"><Glyph name="check" size={16} /></span>
                  </div>
                ) : (
                  <button type="button" key={r.code} className="v-recent-row" disabled={busy}
                    onClick={() => run(teamJoin(r.code))}>
                    <Glyph name="users" size={16} />
                    <span className="v-recent-main">
                      <b>{r.title ?? "队伍"}</b>
                      <span className="v-recent-code">{codeLabel(r)}</span>
                      {r.isOwner && <span className="v-chip">队长</span>}
                    </span>
                    <span className="v-recent-num">{r.members} 人</span>
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
    </div>
  );
}
