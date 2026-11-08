// 「功能」页：开机启动，以及队伍里的三个功能（面板、文字、投票）的开关和按键。改动交给 App 统一保存；
// 键撞了由 Rust 的 settings2::normalize 换掉（存好后界面显示的是换过的）。
import type { Settings } from "../api";
import { hotkeyParts, vkLabel } from "../shared/keys";
import { KeyCapture } from "./KeyCapture";
import { Row, Switch } from "./PingsPage";

interface Props {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
}

export function FeaturesPage({ settings: s, onChange }: Props) {
  return (
    <div className="page">
      <h1>功能</h1>

      <section className="card">
        <h2>启动</h2>
        <Row label="开机时自动启动" hint="开机后只在托盘里，不弹出控制面板">
          <Switch label="开机时自动启动" checked={s.launchAtStartup} onChange={(launchAtStartup) => onChange({ launchAtStartup })} />
        </Row>
      </section>

      <section className="card">
        <h2>队伍面板</h2>
        <Row label="按住显示队伍面板" hint={`在队伍里按住 ${vkLabel(s.panelKey)} 看队员、在线情况，也能发起投票。全屏游戏里照常交给游戏`}>
          <Switch label="队伍面板" checked={s.teamPanel} onChange={(teamPanel) => onChange({ teamPanel })} />
        </Row>
        {s.teamPanel && (
          <Row label="面板键" hint="单独一个键，不带 Ctrl、Alt；和别的快捷键撞了会自动换一个">
            <KeyCapture kind="plain" value={s.panelKey} onPick={(panelKey) => onChange({ panelKey })} />
          </Row>
        )}
      </section>

      <section className="card">
        <h2>队伍文字</h2>
        <Row label="快捷键打字给队友" hint={`在队伍里按 ${hotkeyParts(s.textHotkey).join(" + ")} 打开输入框，回车后这句话出现在队友屏幕上同样的位置`}>
          <Switch label="队伍文字" checked={s.teamText} onChange={(teamText) => onChange({ teamText })} />
        </Row>
        {s.teamText && (
          <>
            <Row label="快捷键">
              <KeyCapture kind="hotkey" value={s.textHotkey} onPick={(textHotkey) => onChange({ textHotkey })} />
            </Row>
            <Row label="队友的话怎么显示">
              <select value={s.textStyle} onChange={(e) => onChange({ textStyle: e.target.value as Settings["textStyle"] })}>
                <option value="spot">在说话的位置冒出来</option>
                <option value="danmaku">弹幕从屏幕上方飘过</option>
              </select>
            </Row>
          </>
        )}
      </section>

      <section className="card">
        <h2>投票</h2>
        <Row
          label="投降、准备确认、踢人"
          hint="投降要 70% 同意，准备确认要所有人接受，踢人要过半。被踢的人的程序会自己退出队伍。关掉后你不算投票人"
        >
          <Switch label="投票" checked={s.teamVotes} onChange={(teamVotes) => onChange({ teamVotes })} />
        </Row>
      </section>
    </div>
  );
}
