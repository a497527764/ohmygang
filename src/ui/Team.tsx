// V2 队伍页：建队 / 进队 / 复制队伍码 / 看队员 / 退队，一屏放完。
// 资料、网络开关、中继状态保留，说明文字全部收进 title。
import { useCallback, useEffect, useState } from "react";
import {
  onProtocolMismatch, onTeamChanged, profileSet, teamCancel, teamCreate, teamDissolve,
  teamJoin, teamLeave, teamRename, teamView, type Settings, type TeamView,
} from "../api";
import { Avatar } from "../components/Avatar";
import { Glyph } from "./Glyph";
import { Action, Fold, Row } from "./controls";

const STATUS: Record<string, string> = { online: "在线", paused: "暂停", away: "离开" };
const PATH: Record<string, string> = { relay: "中继", offline: "离线" };

export function Team({ settings: s, onChange }: { settings: Settings; onChange: (patch: Partial<Settings>) => void }) {
  const [view, setView] = useState<TeamView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [mismatch, setMismatch] = useState(false);
  const [copied, setCopied] = useState(false);
  const [title, setTitle] = useState("开黑");
  const [code, setCode] = useState("");
  const [confirm, setConfirm] = useState<"leave" | "dissolve" | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);

  const load = useCallback(() => {
    teamView().then(setView, (e) => setError(String(e)));
  }, []);
  useEffect(() => {
    load();
    const a = onTeamChanged(load);
    const b = onProtocolMismatch(() => setMismatch(true));
    return () => { void a.then((f) => f()); void b.then((f) => f()); };
  }, [load]);

  const run = (p: Promise<TeamView>) => {
    setBusy(true);
    p.then((v) => { setView(v); setError(null); }, (e) => setError(String(e))).finally(() => setBusy(false));
  };
  const copy = () => {
    if (!view?.team?.code) return;
    void navigator.clipboard.writeText(view.team.code).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    });
  };

  if (!view) return <div className="v-page"><p className="v-note">{error ?? "…"}</p></div>;
  const team = view.team;

  return (
    <div className="v-page">
      <div className="v-page-head">
        <Glyph name="users" size={26} />
        <h1>队伍</h1>
        <span className="v-big-num">{view.members.length}<i>/16</i></span>
      </div>
      {error && <p className="v-error">{error}</p>}
      {mismatch && <p className="v-error">版本不一致 · 请都更新</p>}

      <div className="v-team-grid">
        <section className="v-tile">
          <div className="v-tile-head"><Glyph name="shield" size={20} /><b>我</b></div>
          <div className="v-profile">
            <Avatar fingerprint={view.me.fingerprint} name={view.me.nickname} avatar={view.me.avatar} size={52} />
            <input
              value={view.me.nickname}
              maxLength={24}
              aria-label="昵称"
              onBlur={(e) => e.target.value !== view.me.nickname && void profileSet(e.target.value, null).then(setView)}
              onChange={(e) => setView({ ...view, me: { ...view.me, nickname: e.target.value } })}
            />
          </div>
        </section>

        {!team && (
          <section className="v-tile v-team-actions">
            <div className="v-tile-head"><Glyph name="plus" size={20} /><b>建队 / 进队</b></div>
            <form className="v-inline" onSubmit={(e) => { e.preventDefault(); run(teamCreate(title)); }}>
              <input value={title} maxLength={64} aria-label="队名" placeholder="队名" onChange={(e) => setTitle(e.target.value)} />
              <Action icon="plus" label="建队" onClick={() => run(teamCreate(title))} />
            </form>
            <form className="v-inline" onSubmit={(e) => { e.preventDefault(); run(teamJoin(code)); }}>
              <input value={code} aria-label="队伍码" placeholder="6 位队伍码" spellCheck={false} maxLength={64}
                onChange={(e) => setCode(e.target.value.toUpperCase().replace(/[^0-9A-Z-]/g, ""))} />
              <Action icon="chevron" label="进队" tone="ghost" onClick={() => run(teamJoin(code))} />
            </form>
          </section>
        )}

        {team?.status === "joining" && (
          <section className="v-tile">
            <div className="v-tile-head"><Glyph name="clock" size={20} /><b>加入中</b></div>
            <Action icon="close" label="取消" tone="ghost" onClick={() => run(teamCancel())} />
          </section>
        )}

        {team?.status === "active" && (
          <section className="v-tile v-team-active">
            <div className="v-tile-head">
              <Glyph name="users" size={20} />
              {renaming === null ? (
                <>
                  <b>{team.title ?? "队伍"}</b>
                  {team.isOwner && (
                    <button className="v-icon-btn" title="改名" onClick={() => setRenaming(team.title ?? "")}>
                      <Glyph name="edit" size={18} />
                    </button>
                  )}
                </>
              ) : (
                <form className="v-inline" onSubmit={(e) => { e.preventDefault(); run(teamRename(renaming)); setRenaming(null); }}>
                  <input autoFocus value={renaming} maxLength={64} aria-label="新队名" onChange={(e) => setRenaming(e.target.value)} />
                  <button className="v-icon-btn" title="保存" type="submit"><Glyph name="check" size={18} /></button>
                </form>
              )}
              <span className="v-page-spacer" />
              <button className="v-code sm" onClick={copy} title="复制队伍码">
                <Glyph name={copied ? "check" : "copy"} size={18} />
                <span>{copied ? "已复制" : "队伍码"}</span>
              </button>
            </div>
            <div className="v-members">
              {view.members.map((m) => (
                <div key={m.memberId} className={`v-member${m.path === "offline" ? " off" : ""}`}
                  title={`${STATUS[m.status ?? "online"] ?? "在线"} · ${PATH[m.path ?? "offline"] ?? "离线"}`}>
                  <Avatar fingerprint={m.fingerprint} name={m.nickname} avatar={m.avatar} size={36} />
                  <b>{m.nickname || m.fingerprint}</b>
                  {m.isOwner && <span className="v-chip">队长</span>}
                </div>
              ))}
            </div>
            <div className="v-team-foot">
              {confirm === null ? (
                <>
                  <Action icon="close" label="退出" tone="ghost" disabled={busy} onClick={() => setConfirm("leave")} />
                  {team.isOwner && <Action icon="trash" label="解散" tone="danger" disabled={busy} onClick={() => setConfirm("dissolve")} />}
                </>
              ) : (
                <>
                  <b>{confirm === "leave" ? "退出？" : "解散？"}</b>
                  <Action icon="check" label="确定" onClick={() => { run(confirm === "leave" ? teamLeave() : teamDissolve()); setConfirm(null); }} />
                  <Action icon="close" label="取消" tone="ghost" onClick={() => setConfirm(null)} />
                </>
              )}
            </div>
          </section>
        )}
      </div>

      <Fold icon="globe" label="网络">
        <Row icon="wifi" label="中继">
          <span className="v-value">{view.relaysOk}/{view.relaysTotal}</span>
        </Row>
        {s.relays.map((r, i) => (
          <Row key={i} icon="globe" label={`中继 ${i + 1}`}>
            <input className="v-input" value={r} maxLength={120} aria-label={`中继 ${i + 1}`}
              onChange={(e) => onChange({ relays: s.relays.map((x, j) => (j === i ? e.target.value : x)) })} />
            <button className="v-icon-btn" title="移除" onClick={() => onChange({ relays: s.relays.filter((_, j) => j !== i) })}>
              <Glyph name="close" size={18} />
            </button>
          </Row>
        ))}
        {s.relays.length < 16 && (
          <Action icon="plus" label="中继" tone="ghost" onClick={() => onChange({ relays: [...s.relays, "wss://"] })} />
        )}
      </Fold>
    </div>
  );
}
