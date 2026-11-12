// 「队伍」页：我的资料、建队或进队、队伍码、队员和他们的状态、退出和解散、网络。
// 数据流：打开时取一次整份 TeamView；Rust 推 "team://changed" 时重取（中继连接、掉线这些，
// Rust 每 5 秒看一次，变了才推）。所有改动都返回新的 TeamView，直接换上。
import { useCallback, useEffect, useState } from "react";
import {
  onProtocolMismatch,
  onTeamChanged,
  teamCancel,
  teamCreate,
  teamDissolve,
  teamJoin,
  teamLeave,
  teamRename,
  teamView,
  type Member,
  type Relay,
  type Settings,
  type TeamView,
} from "../api";
import { Avatar } from "./Avatar";
import { Tip } from "./controls";
import { Icon } from "./Icon";
import { ProfileCard } from "./ProfileCard";

const STATUS_TEXT: Record<NonNullable<Member["status"]>, string> = { online: "在线", paused: "信号暂停", away: "离开电脑" };
const PATH_TEXT: Record<NonNullable<Member["path"]>, string> = { relay: "中继", offline: "离线" };
const PLATFORM_TEXT: Record<string, string> = { win: "Windows", mac: "macOS", linux: "Linux" };

function memberLine(m: Member): string {
  if (m.isMe) return "这台电脑";
  if (m.path === "offline" || m.path === null) return "离线";
  const parts = [m.status ? STATUS_TEXT[m.status] : "在线", PATH_TEXT[m.path]];
  if (m.platform && m.platform !== "win") parts.push(PLATFORM_TEXT[m.platform] ?? m.platform);
  return parts.join(" · ");
}

function dotOf(m: Member): "online" | "away" | "paused" | "offline" {
  if (m.isMe) return "online";
  if (m.path === "offline" || m.path === null) return "offline";
  return m.status === "away" ? "away" : m.status === "paused" ? "paused" : "online";
}

interface Props {
  settings: Settings;
  onSettings: (patch: Partial<Settings>) => void;
}

export function TeamPage(_props: Props) {
  const [view, setView] = useState<TeamView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [mismatch, setMismatch] = useState(false);

  const load = useCallback(() => {
    teamView().then(setView, (e) => setError(String(e)));
  }, []);

  useEffect(() => {
    load();
    const offChanged = onTeamChanged(load);
    const offMismatch = onProtocolMismatch(() => setMismatch(true));
    return () => {
      void offChanged.then((f) => f());
      void offMismatch.then((f) => f());
    };
  }, [load]);

  const run = (p: Promise<TeamView>) => {
    setBusy(true);
    p.then(
      (v) => {
        setView(v);
        setError(null);
      },
      (e) => setError(String(e)),
    ).finally(() => setBusy(false));
  };

  if (!view) return <div className="page">{error ? <p className="banner error">{error}</p> : <p className="loading">正在读取队伍…</p>}</div>;

  return (
    <div className="page">
      <h1>队伍</h1>
      {error && <p className="banner error">{error}</p>}
      {mismatch && <p className="banner warn">有队友的版本对不上（多半是 1.x），互相收不到，请都更新到最新版</p>}

      <ProfileCard me={view.me} onSaved={setView} />

      {!view.team && <NoTeam busy={busy} onCreate={(t) => run(teamCreate(t))} onJoin={(c) => run(teamJoin(c))} />}
      {view.team?.status === "joining" && (
        <section className="card">
          <h2>
            正在加入队伍…
            <Tip text="要等至少一位队友在线，同步完队伍信息才算进队。大家都连着云端服务就行" />
          </h2>
          <div className="actions">
            <button className="soft" disabled={busy} onClick={() => run(teamCancel())}>
              取消加入
            </button>
          </div>
        </section>
      )}
      {view.team?.status === "active" && <ActiveTeam view={view} busy={busy} run={run} />}

      <section className="card">
        <h2>网络</h2>
        <p className="net-status">
          {`云端连接：${view.relaysOk} / ${view.relaysTotal} 个中继已连接`}
          {view.relays.length > 0 && <Tip text={relayLines(view.relays)} />}
        </p>
      </section>
    </div>
  );
}

const RELAY_STATE: Record<Relay["state"], string> = { ok: "已连接", connecting: "连接中", cooling: "被限速", retired: "不再使用" };

/** 每个中继一行：正在发给它的打 ●，状态、发出的事件有多少从它那里回来了、平均多久回来。 */
function relayLines(relays: Relay[]): string {
  const lines = relays.map((r) => {
    const echo = r.echoPct === null ? "还没发过" : `回显 ${r.echoPct}%${r.echoMs === null ? "" : `，平均 ${r.echoMs} 毫秒`}`;
    return `${r.sending ? "●" : "○"} ${r.host}：${RELAY_STATE[r.state]}，${echo}`;
  });
  return ["● 正在发给它（只发回显最好的 3 个，收照常全听）", "回显：发出的事件有多少从中继回到自己这里（最近 50 个）", ...lines].join("\n");
}

function NoTeam({ busy, onCreate, onJoin }: { busy: boolean; onCreate: (t: string) => void; onJoin: (c: string) => void }) {
  const [title, setTitle] = useState("开黑");
  const [code, setCode] = useState("");
  return (
    <div className="split">
      <section className="card">
        <h2>
          建队
          <Tip text="建好后把队伍码发给朋友，最多 16 人" />
        </h2>
        <form
          className="inline-form"
          onSubmit={(e) => {
            e.preventDefault();
            onCreate(title);
          }}
        >
          <input value={title} maxLength={64} onChange={(e) => setTitle(e.target.value)} aria-label="队名" placeholder="队名" />
          <button className="primary" disabled={busy || !title.trim()}>
            建队
          </button>
        </form>
      </section>
      <section className="card">
        <h2>
          进队
          <Tip text="队伍码是 6 位字母数字，向队长要" />
        </h2>
        <form
          className="inline-form"
          onSubmit={(e) => {
            e.preventDefault();
            onJoin(code);
          }}
        >
          <input value={code} onChange={(e) => setCode(e.target.value.toUpperCase().replace(/[^0-9A-Z-]/g, ""))} aria-label="队伍码" placeholder="6 位队伍码" spellCheck={false} />
          <button className="primary" disabled={busy || !code.trim()}>
            进队
          </button>
        </form>
      </section>
    </div>
  );
}

function ActiveTeam({ view, busy, run }: { view: TeamView; busy: boolean; run: (p: Promise<TeamView>) => void }) {
  const team = view.team!;
  const [copied, setCopied] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<"leave" | "dissolve" | null>(null);

  const copy = () => {
    void navigator.clipboard.writeText(team.code).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <section className="card">
      <div className="team-head">
        {renaming === null ? (
          <h2>
            {team.title ?? "队伍"}
            {team.isOwner && (
              <button className="icon-button small" aria-label="改队名" onClick={() => setRenaming(team.title ?? "")}>
                <Icon name="edit" size={15} />
              </button>
            )}
          </h2>
        ) : (
          <form
            className="inline-form"
            onSubmit={(e) => {
              e.preventDefault();
              run(teamRename(renaming));
              setRenaming(null);
            }}
          >
            <input autoFocus value={renaming} maxLength={64} onChange={(e) => setRenaming(e.target.value)} aria-label="新队名" />
            <button className="primary" disabled={busy || !renaming.trim()}>
              保存
            </button>
            <button type="button" className="soft" onClick={() => setRenaming(null)}>
              取消
            </button>
          </form>
        )}
        <span className="muted">{view.members.length} / 16 人</span>
      </div>
      <div className="code-row">
        <code className="team-code">{team.code}</code>
        <button className="soft small" onClick={copy}>
          <Icon name="copy" size={14} />
          {copied ? "已复制" : "复制队伍码"}
        </button>
      </div>
      <ul className="members">
        {view.members.map((m) => (
          <li key={m.memberId} className="member">
            <Avatar fingerprint={m.fingerprint} name={m.nickname} avatar={m.avatar} size={38} dot={dotOf(m)} />
            <div className="member-text">
              <span className="member-name">
                {m.nickname || m.fingerprint}
                {m.isMe && <span className="muted">（我）</span>}
                {m.isOwner && <span className="chip">队长</span>}
              </span>
              <small className="hint">{memberLine(m)}</small>
            </div>
          </li>
        ))}
      </ul>
      <div className="actions">
        {confirm === null ? (
          <>
            <button className="soft" disabled={busy} onClick={() => setConfirm("leave")}>
              退出队伍
            </button>
            {team.isOwner && (
              <button className="soft danger" disabled={busy} onClick={() => setConfirm("dissolve")}>
                解散队伍
              </button>
            )}
          </>
        ) : (
          <>
            <span>{confirm === "leave" ? "确定退出？" : "解散后不能撤销，确定？"}</span>
            <button
              className="primary danger"
              disabled={busy}
              onClick={() => {
                run(confirm === "leave" ? teamLeave() : teamDissolve());
                setConfirm(null);
              }}
            >
              {confirm === "leave" ? "退出" : "解散"}
            </button>
            <button className="soft" onClick={() => setConfirm(null)}>
              取消
            </button>
          </>
        )}
      </div>
    </section>
  );
}
