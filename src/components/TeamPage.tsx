// 「队伍」页：我的资料、建队或进队、队伍码、队员和他们的状态、对线模式和排行榜、退出和解散、网络。
// 数据流：打开时取一次整份 TeamView；Rust 推 "team://changed" 时重取；另外每 3 秒重取一次，
// 因为队友走局域网还是中继、掉没掉线，不会每次都推送。所有改动都返回新的 TeamView，直接换上。
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
  type Settings,
  type TeamView,
} from "../api";
import { hotkeyParts } from "../shared/keys";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";
import { KeyCapture } from "./KeyCapture";
import { ProfileCard } from "./ProfileCard";

const REFRESH_MS = 3000;

const STATUS_TEXT: Record<NonNullable<Member["status"]>, string> = { online: "在线", paused: "信号暂停", away: "离开电脑" };
const PATH_TEXT: Record<NonNullable<Member["path"]>, string> = { lan: "局域网", relay: "中继", offline: "离线" };
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
  laning: boolean;
  onLaning: (on: boolean) => void;
}

export function TeamPage({ settings, onSettings, laning, onLaning }: Props) {
  const [view, setView] = useState<TeamView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [mismatch, setMismatch] = useState(false);

  const load = useCallback(() => {
    teamView().then(setView, (e) => setError(String(e)));
  }, []);

  useEffect(() => {
    load();
    const timer = setInterval(load, REFRESH_MS);
    const offChanged = onTeamChanged(load);
    const offMismatch = onProtocolMismatch(() => setMismatch(true));
    return () => {
      clearInterval(timer);
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
      {mismatch && <p className="banner warn">有队友用的是别的版本（多半是 1.x），互相收不到。请大家都更新到同一个版本。</p>}

      <ProfileCard me={view.me} onSaved={setView} />

      {!view.team && <NoTeam busy={busy} onCreate={(t) => run(teamCreate(t))} onJoin={(c) => run(teamJoin(c))} />}
      {view.team?.status === "joining" && (
        <section className="card">
          <h2>正在加入队伍</h2>
          <p className="muted">要等至少一位队友在线，同步完队伍信息才算进队。队友在同一个局域网里，或者大家都开着「允许互联网」就行。</p>
          <div className="actions">
            <button className="soft" disabled={busy} onClick={() => run(teamCancel())}>
              取消加入
            </button>
          </div>
        </section>
      )}
      {view.team?.status === "active" && <ActiveTeam view={view} busy={busy} run={run} laning={laning} />}
      {view.team?.status === "active" && (
        <LaningCard view={view} laning={laning} onLaning={onLaning} settings={settings} onSettings={onSettings} />
      )}

      <section className="card">
        <h2>网络</h2>
        <div className="row">
          <div className="row-text">
            <span>允许互联网</span>
            <small className="hint">不在同一个局域网时经公共中继转发，内容加密，中继看不到</small>
          </div>
          <div className="row-control">
            <input
              type="checkbox"
              role="switch"
              aria-label="允许互联网"
              checked={settings.allowInternet}
              onChange={(e) => onSettings({ allowInternet: e.target.checked })}
            />
          </div>
        </div>
        <p className="net-status">
          {settings.allowInternet
            ? `中继：${view.relaysOk} / ${view.relaysTotal} 个已连接`
            : "只用局域网"}
          {view.lanDegraded && <span className="warn-text">　局域网端口被别的程序占了，只能经中继联系</span>}
        </p>
      </section>
    </div>
  );
}

function NoTeam({ busy, onCreate, onJoin }: { busy: boolean; onCreate: (t: string) => void; onJoin: (c: string) => void }) {
  const [title, setTitle] = useState("开黑");
  const [code, setCode] = useState("");
  return (
    <div className="split">
      <section className="card">
        <h2>建一个队伍</h2>
        <p className="muted">建好后把队伍码发给朋友，最多 16 人。</p>
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
        <h2>用队伍码进队</h2>
        <p className="muted">队伍码以 GT2 开头，向队长要。</p>
        <form
          className="inline-form"
          onSubmit={(e) => {
            e.preventDefault();
            onJoin(code);
          }}
        >
          <input value={code} onChange={(e) => setCode(e.target.value)} aria-label="队伍码" placeholder="GT2-…" spellCheck={false} />
          <button className="primary" disabled={busy || !code.trim()}>
            进队
          </button>
        </form>
      </section>
    </div>
  );
}

function ActiveTeam({ view, busy, run, laning }: { view: TeamView; busy: boolean; run: (p: Promise<TeamView>) => void; laning: boolean }) {
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
                {(m.isMe ? laning : m.laning) && <span className="chip lane">对线中</span>}
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
            <span>{confirm === "leave" ? "退出后可以用队伍码再进来。确定退出？" : "解散后所有人都不能再用这个队伍，不能撤销。确定解散？"}</span>
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

function LaningCard(props: {
  view: TeamView;
  laning: boolean;
  onLaning: (on: boolean) => void;
  settings: Settings;
  onSettings: (patch: Partial<Settings>) => void;
}) {
  const { view, laning, onLaning, settings, onSettings } = props;
  const board = [...view.members].sort((a, b) => b.stacks - a.stacks || a.taken - b.taken);
  return (
    <section className="card">
      <h2>对线模式</h2>
      <div className="row">
        <div className="row-text">
          <span>{laning ? "对线中" : "没开"}</span>
          <small className="hint">
            开着的人在别人屏幕上是一个跟着鼠标走的小英雄。贴近对手 3 秒充能，再按住信号触发键单击他，就叠一层心之钢；同一个人 30 秒内只能叠一次。
          </small>
        </div>
        <div className="row-control">
          <input type="checkbox" role="switch" aria-label="对线模式" checked={laning} onChange={(e) => onLaning(e.target.checked)} />
        </div>
      </div>
      <div className="row">
        <div className="row-text">
          <span>快捷键</span>
          <small className="hint">按一下开、再按一下关（现在是 {hotkeyParts(settings.laningHotkey).join(" + ")}）</small>
        </div>
        <div className="row-control">
          <KeyCapture kind="hotkey" value={settings.laningHotkey} onPick={(laningHotkey) => onSettings({ laningHotkey })} />
        </div>
      </div>
      <table className="board">
        <thead>
          <tr>
            <th>排行</th>
            <th>层数</th>
            <th>生命值</th>
            <th>被叠</th>
          </tr>
        </thead>
        <tbody>
          {board.map((m, i) => (
            <tr key={m.memberId} className={m.isMe ? "me" : undefined}>
              <td>
                <span className="rank">{i + 1}</span>
                {m.nickname || m.fingerprint}
              </td>
              <td className="num">{m.stacks}</td>
              <td className="num">{m.hp}</td>
              <td className="num">{m.taken}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
