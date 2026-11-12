// 3.3 设置页：唯一二级页，顶部一排 HUD tab，分区承载原来的信号 / 表情 / 队伍 / 按键 / 功能 / 关于。
// 轮盘编辑器（信号、表情各一个）从旧独立页搬进来；说明文字只保留在 title 里。
import { useEffect, useMemo, useState } from "react";
import {
  importEmote, onUpdate, onVoice, previewEmote, previewPing, removeEmote, textPreview,
  updateCheck, updateInstall, updateView, voiceTest, voiceView,
  type KeyClash, type Settings, type Status, type TextEffect, type UpdateView, type VoiceStatus,
} from "../api";
import { prepareCustomEmote, prepareSound, toBase64, type PreparedEmote } from "../emoteImport";
import { KeyCapture } from "../components/KeyCapture";
import { vkLabel } from "../shared/keys";
import { ALL_PINGS, pingById, type PingId } from "../shared/pings";
import { EMOTE_CATALOG } from "../shared/emoteCatalog";
import { FX_NAMES, MAX_CUSTOM_EMOTES, emoteName, type CustomEmote } from "../shared/emotes";
import { emoteIconStyled, pingIcon, type IconStyle } from "../shared/vectorAssets";
import { Glyph, type GlyphName } from "./Glyph";
import { WheelDial } from "./WheelDial";
import { MicWave } from "./MicWave";
import { THEME_PRESETS, type Theme } from "./themes";
import { Action, Choice, Fold, Range, Row, Toggle } from "./controls";

export type SettingsTab = "common" | "keys" | "pings" | "emotes" | "team" | "voice" | "about";
const TABS: { id: SettingsTab; label: string; icon: GlyphName }[] = [
  { id: "common", label: "常用", icon: "bolt" },
  { id: "keys", label: "按键", icon: "keyboard" },
  { id: "pings", label: "信号", icon: "target" },
  { id: "emotes", label: "表情", icon: "smile" },
  { id: "team", label: "文字", icon: "text" },
  { id: "voice", label: "语音", icon: "mic" },
  { id: "about", label: "关于", icon: "info" },
];

const TRIGGERS = [
  { value: "alt", label: "Alt" }, { value: "ctrl", label: "Ctrl" }, { value: "shift", label: "Shift" },
  { value: "win", label: "Win" }, { value: "capslock", label: "Caps" },
  { value: "mouse4", label: "侧键 1" }, { value: "mouse5", label: "侧键 2" },
] as const;
const EFFECTS: { value: TextEffect; label: string }[] = [
  { value: "outline", label: "描边" }, { value: "gold", label: "金色" }, { value: "neon", label: "霓虹" },
  { value: "bounce", label: "弹跳" }, { value: "typewriter", label: "打字" }, { value: "shake", label: "抖动" },
];
const DIRS = ["上", "右上", "右", "右下", "下", "左下", "左", "左上"];
const UPDATE_TEXT: Record<UpdateView["phase"], string> = {
  idle: "", checking: "检查中", upToDate: "最新", available: "有新版本",
  downloading: "下载中", installing: "安装中", failed: "失败",
};

function IconStylePick({ value, onChange }: { value: IconStyle; onChange: (v: IconStyle) => void }) {
  const options: { value: IconStyle; label: string; sub: string }[] = [
    { value: "league", label: "原版", sub: "英雄联盟原图" },
    { value: "vector", label: "矢量", sub: "加粗描边" },
  ];
  // 三个中性信号（路上 / 协助 / 敌人消失）最能看出两套图的差别，不用「危险」那张红三角，免得像报错。
  const samples = ["omw", "assist", "missing"] as const;
  return (
    <div className="v-style-pick" role="radiogroup" aria-label="图标风格">
      {options.map((o) => (
        <button key={o.value} type="button" role="radio" aria-checked={value === o.value}
          className={`v-style${value === o.value ? " on" : ""}`} title={o.sub} onClick={() => onChange(o.value)}>
          <span className="v-style-thumbs">
            {samples.map((id) => <img key={id} src={pingIcon(id, o.value)} alt="" />)}
          </span>
          <b>{o.label}</b>
        </button>
      ))}
    </div>
  );
}

/** 六套主题：深色一行、浅色一行，每格用三个色块预览底色 / 卡片 / 主强调色。 */
function ThemePick({ value, onChange }: { value: Theme; onChange: (v: Theme) => void }) {
  return (
    <div className="v-theme-pick" role="radiogroup" aria-label="主题">
      {(["dark", "light"] as const).map((group) => (
        <div className="v-theme-group" key={group}>
          <span className="v-theme-group-name">{group === "dark" ? "深色" : "浅色"}</span>
          <div className="v-theme-row">
            {THEME_PRESETS.filter((p) => p.group === group).map((p) => (
              <button key={p.id} type="button" role="radio" aria-checked={value === p.id}
                className={`v-theme-chip${value === p.id ? " on" : ""}`}
                title={p.label} onClick={() => onChange(p.id)}>
                <span className="v-theme-swatch" style={{ background: p.bg }}>
                  <i style={{ background: p.card }} />
                  <i style={{ background: p.accent }} />
                  <i style={{ background: p.orange }} />
                </span>
                <b>{p.label}</b>
              </button>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

/** 轮盘编辑器的小轮盘格子：绝对定位在圆周上，选中格高亮。 */
function MiniSlot({ i, on, icon, label, onClick }: {
  i: number; on: boolean; icon: string; label: string; onClick: () => void;
}) {
  const a = ((-90 + i * 45) * Math.PI) / 180;
  return (
    <button type="button"
      className={`v-mini-slot${on ? " on" : ""}`}
      style={{ left: `${50 + 34 * Math.cos(a)}%`, top: `${50 + 34 * Math.sin(a)}%` }}
      onClick={onClick}
      aria-label={`${DIRS[i]}：${label}`}
      aria-pressed={on}
      title={`${DIRS[i]} · ${label}`}>
      <img src={icon} alt="" />
    </button>
  );
}

interface Props {
  settings: Settings;
  status: Status;
  clashes: KeyClash[];
  onChange: (patch: Partial<Settings>) => void;
  onSaved: (s: Settings) => void;
  tab: SettingsTab;
  onTab: (t: SettingsTab) => void;
  onBack: () => void;
}

export function SettingsView({ settings: s, status, clashes, onChange, onSaved, tab, onTab, onBack }: Props) {
  const [mics, setMics] = useState<{ id: string; name: string }[]>([]);
  const [voice, setVoice] = useState<VoiceStatus>({ error: null, test: null });
  const [update, setUpdate] = useState<UpdateView | null>(null);

  useEffect(() => {
    void voiceView().then((v) => { setMics(v.mics); setVoice(v.status); });
    void updateView().then(setUpdate);
    const a = onVoice(setVoice);
    const b = onUpdate(setUpdate);
    return () => { void a.then((f) => f()); void b.then((f) => f()); };
  }, []);

  const clash = (key: string) => clashes.some((c) => c.key === key);
  const mic = mics.some((m) => m.id === s.mic) ? s.mic : "";

  // ---- 信号轮盘编辑 ----
  const pingSlot = useState(0);
  const setPing = (p: PingId, slot: number) => {
    const w = [...s.wheel];
    const from = w.indexOf(p);
    if (from >= 0) [w[from], w[slot]] = [w[slot], w[from]];
    else w[slot] = p;
    return w;
  };

  // ---- 表情轮盘编辑 ----
  const emoteSlot = useState(0);
  const emoteIcon = (ref: string) => emoteIconStyled(ref, s.iconStyle);
  const allEmoteRefs: string[] = useMemo(
    () => [...EMOTE_CATALOG.map((e) => e.slug), ...s.customEmotes.map((c) => c.id)],
    [s.customEmotes],
  );
  const placeEmote = (ref: string, slot: number) => {
    const next = [...s.emoteWheel];
    const from = next.indexOf(ref);
    if (from >= 0) [next[from], next[slot]] = [next[slot], next[from]];
    else next[slot] = ref;
    return next;
  };

  // ---- 表情导入 ----
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<PreparedEmote | null>(null);
  const [sound, setSound] = useState<Uint8Array | null>(null);
  const [name, setName] = useState("");
  const [fx, setFx] = useState(0);
  const [confirm, setConfirm] = useState<string | null>(null);
  const previewUrl = useMemo(
    () => (picked ? URL.createObjectURL(new Blob([picked.still.slice()], { type: "image/webp" })) : null),
    [picked],
  );
  useEffect(() => () => (previewUrl ? URL.revokeObjectURL(previewUrl) : undefined), [previewUrl]);
  const pickImage = async (file: File | undefined) => {
    if (!file) return;
    const r = await prepareCustomEmote(file);
    if (!r.ok) return setError(r.error);
    setPicked(r.value);
    setName(r.value.name.slice(0, 24));
    setError(null);
  };
  const pickSound = async (file: File | undefined) => {
    if (!file) return;
    const r = await prepareSound(file);
    if (r.ok) setSound(r.value);
    else setError(r.error);
  };
  const saveEmote = () => {
    if (!picked) return;
    const atLimit = s.customEmotes.length >= MAX_CUSTOM_EMOTES;
    const go = () => {
      setBusy(true);
      importEmote({
        still: toBase64(picked.still),
        anim: picked.anim ? toBase64(picked.anim) : null,
        sound: sound ? toBase64(sound) : null,
        name,
        fx,
      })
        .then((saved) => {
          onSaved(saved);
          setPicked(null);
          setSound(null);
          setName("");
          setFx(0);
          setError(null);
        }, (e) => setError(String(e)))
        .finally(() => setBusy(false));
    };
    // 到了上限：先问，再删最早导入的那一个，再导入新的
    if (atLimit) {
      if (!window.confirm(`自定义表情已有 ${MAX_CUSTOM_EMOTES} 个，继续导入会删除最早导入的那一个，确定吗？`)) return;
      const earliest = s.customEmotes[0]?.id;
      if (earliest) {
        removeEmote(earliest).then(go).catch((e) => setError(String(e)));
        return;
      }
    }
    go();
  };
  const [drag, setDrag] = useState(false);
  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDrag(false);
    const f = e.dataTransfer.files?.[0];
    if (f) void pickImage(f);
  };
  const onPaste = (e: React.ClipboardEvent) => {
    const item = Array.from(e.clipboardData.items).find((i) => i.type.startsWith("image/"));
    const f = item?.getAsFile();
    if (f) {
      e.preventDefault();
      void pickImage(f);
    }
  };

  return (
    <div className="v-page">
      <header className="v-page-head">
        <button type="button" className="v-back" onClick={onBack} title="返回主页 · Ctrl+E">
          <Glyph name="chevron" size={22} />
        </button>
        <h1>设置</h1>
        <span className="v-page-crumb">{TABS.find((t) => t.id === tab)?.label}</span>
        <span className="v-ctrl-e"><kbd>Ctrl</kbd>+<kbd>E</kbd></span>
      </header>

      <nav className="v-tabs" role="tablist">
        {TABS.map((t, i) => (
          <button key={t.id} type="button" role="tab" aria-selected={tab === t.id}
            className={`v-tab${tab === t.id ? " on" : ""}`} onClick={() => onTab(t.id)}>
            <em>{String(i + 1).padStart(2, "0")}</em>
            <Glyph name={t.icon} size={17} />
            <b>{t.label}</b>
          </button>
        ))}
      </nav>

      <div className="v-settings-main">
        <div className="v-tab-body">
          {tab === "common" && (
          <>
            <section className="v-tile v-common">
              <div className="v-common-item">
                <div className="v-common-name"><Glyph name="image" size={18} /><b>图标风格</b></div>
                <IconStylePick value={s.iconStyle} onChange={(iconStyle) => onChange({ iconStyle })} />
              </div>
              <div className="v-common-item">
                <div className="v-common-name"><Glyph name="monitor" size={18} /><b>主题</b></div>
                <ThemePick value={s.theme} onChange={(theme) => onChange({ theme })} />
              </div>
              <div className="v-common-item">
                <div className="v-common-name"><Glyph name="volume" size={18} /><b>音量</b></div>
                <div className="v-inline">
                  <Range label="音量" value={s.volume} min={0} max={100} onChange={(volume) => onChange({ volume })} />
                </div>
                <div className="v-common-switch">
                  <span>全部静音</span>
                  <Toggle label="全部静音" checked={s.muted} onChange={(muted) => onChange({ muted })} />
                </div>
              </div>
            </section>
            <section className="v-tile">
              <Row icon="power" label="自动上线" title="启动后直接开始拦手势；关掉就停在离线，用开关或快捷键恢复">
                <Toggle label="自动上线" checked={s.autoOnline} onChange={(autoOnline) => onChange({ autoOnline })} />
              </Row>
              <Row icon="users" label="自动归队" title="启动时自动回到上一次的队伍">
                <Toggle label="自动归队" checked={s.autoRejoin} onChange={(autoRejoin) => onChange({ autoRejoin })} />
              </Row>
              <Row icon="power" label="开机启动">
                <Toggle label="开机启动" checked={s.launchAtStartup} onChange={(launchAtStartup) => onChange({ launchAtStartup })} />
              </Row>
              <Row icon="refresh" label="自动查更新">
                <Toggle label="自动查更新" checked={s.autoUpdateCheck} onChange={(autoUpdateCheck) => onChange({ autoUpdateCheck })} />
              </Row>
              <Row icon="text" label="队伍文字">
                <Toggle label="队伍文字" checked={s.teamText} onChange={(teamText) => onChange({ teamText })} />
              </Row>
              <Row icon="vote" label="投票" title="投降、准备确认、踢人；关掉后不算投票人">
                <Toggle label="投票" checked={s.teamVotes} onChange={(teamVotes) => onChange({ teamVotes })} />
              </Row>
            </section>
          </>
        )}

        {tab === "keys" && (
          <>
          <section className="v-tile">
            <Row icon="mouse" label="信号触发" title="按住它拖动开信号轮盘">
              <select className="v-select" aria-label="信号触发键" value={typeof s.trigger === "object" ? "custom" : s.trigger}
                onChange={(e) => onChange({ trigger: e.target.value === "custom" ? { vk: 0x4a } : e.target.value as Settings["trigger"] })}>
                {TRIGGERS.map((t) => <option key={t.value} value={t.value} disabled={t.value === s.emoteTrigger}>{t.label}</option>)}
                <option value="custom">自定义…</option>
              </select>
              {typeof s.trigger === "object" && <KeyCapture kind="key" value={s.trigger.vk} onPick={(vk) => onChange({ trigger: { vk } })} />}
            </Row>
            <Row icon="smile" label="表情触发" title="按住它拖动开表情轮盘；off 表示不用表情">
              <select className="v-select" aria-label="表情触发键" value={s.emoteTrigger === "off" ? "off" : typeof s.emoteTrigger === "object" ? "custom" : s.emoteTrigger}
                onChange={(e) => onChange({ emoteTrigger: e.target.value === "off" ? "off" : e.target.value === "custom" ? { vk: 0x54 } : e.target.value as Settings["emoteTrigger"] })}>
                <option value="off">关</option>
                {TRIGGERS.map((t) => <option key={t.value} value={t.value} disabled={t.value === s.trigger}>{t.label}</option>)}
                <option value="custom">自定义…</option>
              </select>
              {typeof s.emoteTrigger === "object" && <KeyCapture kind="key" value={s.emoteTrigger.vk} onPick={(vk) => onChange({ emoteTrigger: { vk } })} />}
            </Row>
            <Row icon="power" label="信号开关" title={clash("toggle") ? "键冲突，已由后端换成备用键" : undefined}>
              <KeyCapture kind="hotkey" value={s.toggleHotkey} onPick={(toggleHotkey) => onChange({ toggleHotkey })} />
            </Row>
            <Row icon="text" label="打字" title={clash("text") ? "键冲突，已由后端换成备用键" : undefined}>
              <KeyCapture kind="hotkey" value={s.textHotkey} onPick={(textHotkey) => onChange({ textHotkey })} />
            </Row>
            <Row icon="mic" label="说话" title={clash("talk") ? "键冲突，已由后端换成备用键" : undefined}>
              <KeyCapture kind="key" value={s.talkKey} onPick={(talkKey) => onChange({ talkKey })} />
            </Row>
            <Row icon="users" label="队伍面板" title={clash("panel") ? "键冲突，已由后端换成备用键" : undefined}>
              <KeyCapture kind="hotkey" value={s.panelHotkey} onPick={(panelHotkey) => onChange({ panelHotkey })} />
            </Row>
          </section>
          <section className="v-tile v-keyhelp">
            <div className="v-keyhelp-head"><Glyph name="keyboard" size={16} /><b>键位规则</b></div>
            <div className="v-keyhelp-grid">
              <div><b>按住 / 按下</b><span>信号、表情按住拖动开轮盘；开关、打字按一下就触发</span></div>
              <div><b>冲突自动让开</b><span>两处用同一个键时，后改的那个会被换成备用键</span></div>
              <div><b>重新录制</b><span>点右侧键位块再按一次键；按 Esc 取消</span></div>
            </div>
          </section>
          </>
        )}

        {tab === "pings" && (
          <div className="v-signal-grid">
            <section className="v-tile v-editor">
              <div className="v-editor-head">
                <b>轮盘</b>
                <span className="v-page-spacer" />
                <Action icon="play" label="预览" tone="ghost" onClick={() => void previewPing(s.wheel[pingSlot[0]])} />
              </div>
              <div className="v-mini-wheel">
                <svg viewBox="0 0 280 280" aria-hidden="true"><WheelDial /></svg>
                {s.wheel.slice(0, 8).map((id, i) => (
                  <MiniSlot key={i} i={i} on={pingSlot[0] === i} icon={pingIcon(id, s.iconStyle)}
                    label={pingById(id)?.name ?? id} onClick={() => pingSlot[1](i)} />
                ))}
                <span className="v-mini-core">ALT</span>
              </div>
              <div className="v-editor-side">
                <div className="v-editor-cur">
                  <img src={pingIcon(s.wheel[pingSlot[0]], s.iconStyle)} alt="" />
                  <b>{pingById(s.wheel[pingSlot[0]])?.name}</b>
                  <span>{DIRS[pingSlot[0]]}</span>
                </div>
                <div className="v-icon-grid">
                  {ALL_PINGS.map((p) => (
                    <button key={p.id} className={`v-icon-cell${p.id === s.wheel[pingSlot[0]] ? " on" : ""}`}
                      title={p.name} onClick={() => onChange({ wheel: setPing(p.id, pingSlot[0]) })}>
                      <img src={pingIcon(p.id, s.iconStyle)} alt="" />
                    </button>
                  ))}
                </div>
              </div>
            </section>
            <div className="v-side-col">
              <section className="v-tile">
                <Row icon="target" label="轮盘大小">
                  <Range label="轮盘大小" value={s.wheelSizePx} min={300} max={900} step={10} unit="px" onChange={(wheelSizePx) => onChange({ wheelSizePx })} />
                </Row>
                <Row icon="target" label="信号大小">
                  <Range label="信号大小" value={s.pingSizePx} min={60} max={220} unit="px" onChange={(pingSizePx) => onChange({ pingSizePx })} />
                </Row>
                <Row icon="clock" label="停留">
                  <Range label="信号停留" value={s.pingDurationS} min={1} max={8} step={0.1} unit="s" onChange={(pingDurationS) => onChange({ pingDurationS })} />
                </Row>
                <Row icon="mouse" label="拖动距离" title="拖多远才打开轮盘">
                  <Range label="拖动距离" value={s.dragThresholdPx} min={2} max={40} unit="px" onChange={(dragThresholdPx) => onChange({ dragThresholdPx })} />
                </Row>
                <Row icon="volume" label="刻度声">
                  <Toggle label="刻度声" checked={s.tickSound} onChange={(tickSound) => onChange({ tickSound })} />
                </Row>
              </section>
              <section className="v-tile">
                <Row icon="mouse" label="单击信号" title="触发键加单击直接放这个信号">
                  <Toggle label="单击信号" checked={s.clickPing} onChange={(clickPing) => onChange({ clickPing })} />
                </Row>
                {s.clickPing && (
                  <div className="v-inline-pick">
                    {ALL_PINGS.slice(0, 8).map((p) => (
                      <button key={p.id} className={`v-icon-cell sm${s.clickPingId === p.id ? " on" : ""}`}
                        title={p.name} onClick={() => onChange({ clickPingId: p.id })}>
                        <img src={pingIcon(p.id, s.iconStyle)} alt="" />
                      </button>
                    ))}
                  </div>
                )}
                <Row icon="monitor" label="全屏不拦截" title="游戏占着整块屏幕时，触发键加点击原样交给游戏">
                  <Toggle label="全屏不拦截" checked={s.gameCheck} onChange={(gameCheck) => onChange({ gameCheck })} />
                </Row>
              </section>
            </div>
          </div>
        )}

        {tab === "emotes" && (
          <div className="v-signal-grid">
            <section className="v-tile v-editor">
              <div className="v-editor-head">
                <b>轮盘</b>
                <span className="v-page-spacer" />
                <Action icon="play" label="预览" tone="ghost" onClick={() => void previewEmote(s.emoteWheel[emoteSlot[0]])} />
              </div>
              <div className="v-mini-wheel emote">
                <svg viewBox="0 0 280 280" aria-hidden="true"><WheelDial /></svg>
                {s.emoteWheel.slice(0, 8).map((ref, i) => (
                  <MiniSlot key={i} i={i} on={emoteSlot[0] === i} icon={emoteIcon(ref)}
                    label={emoteName(ref, s.customEmotes)} onClick={() => emoteSlot[1](i)} />
                ))}
                <span className="v-mini-core">CTRL</span>
              </div>
              <div className="v-editor-side">
                <div className="v-editor-cur">
                  <img src={emoteIcon(s.emoteWheel[emoteSlot[0]])} alt="" />
                  <b>{emoteName(s.emoteWheel[emoteSlot[0]], s.customEmotes)}</b>
                  <span>{DIRS[emoteSlot[0]]}</span>
                </div>
                <div className="v-icon-grid emote">
                  {allEmoteRefs.map((ref) => (
                    <button key={ref} className={`v-icon-cell${ref === s.emoteWheel[emoteSlot[0]] ? " on" : ""}`}
                      title={emoteName(ref, s.customEmotes)}
                      onClick={() => onChange({ emoteWheel: placeEmote(ref, emoteSlot[0]) })}>
                      <img src={emoteIcon(ref)} alt="" />
                    </button>
                  ))}
                </div>
              </div>
            </section>
            <div className="v-side-col">
              <section className="v-tile">
                <Row icon="mouse" label="单击表情" title="表情触发键加单击直接放这个表情">
                  <Toggle label="单击表情" checked={s.emoteClick} onChange={(emoteClick) => onChange({ emoteClick })} />
                </Row>
                {s.emoteClick && (
                  <Row icon="smile" label="单击放">
                    <select className="v-select" aria-label="单击放的表情" value={s.clickEmote} onChange={(e) => onChange({ clickEmote: e.target.value })}>
                      {[...new Set([...s.emoteWheel, ...s.customEmotes.map((c) => c.id)])].map((r) => (
                        <option key={r} value={r}>{r.startsWith("c:") ? r.slice(2, 10) : r}</option>
                      ))}
                    </select>
                  </Row>
                )}
                <Row icon="target" label="表情大小">
                  <Range label="表情大小" value={s.emoteSizePx} min={80} max={300} unit="px" onChange={(emoteSizePx) => onChange({ emoteSizePx })} />
                </Row>
                <Row icon="volume" label="音效">
                  <Toggle label="表情音效" checked={s.emoteSound} onChange={(emoteSound) => onChange({ emoteSound })} />
                </Row>
                <Row icon="users" label="队友表情" title="关掉后队友导入的表情显示成问号，你导入的也不发给队友">
                  <Toggle label="队友表情" checked={s.friendEmotes} onChange={(friendEmotes) => onChange({ friendEmotes })} />
                </Row>
              </section>
              <Fold icon="smile" label="自定义表情" count={s.customEmotes.length}>
                {error && <p className="v-error">{error}</p>}
                {s.customEmotes.length > 0 && (
                  <div className="v-custom-list">
                    {s.customEmotes.map((c: CustomEmote) => (
                      <div className="v-custom" key={c.id}>
                        <img src={emoteIcon(c.id)} alt="" />
                        <input value={c.name} maxLength={24} aria-label="名字"
                          onChange={(e) => onChange({ customEmotes: s.customEmotes.map((x) => (x.id === c.id ? { ...x, name: e.target.value } : x)) })} />
                        <select value={c.fx} aria-label="出场效果"
                          onChange={(e) => onChange({ customEmotes: s.customEmotes.map((x) => (x.id === c.id ? { ...x, fx: Number(e.target.value) } : x)) })}>
                          {FX_NAMES.map((n, i) => <option key={n} value={i}>{n}</option>)}
                        </select>
                        {confirm === c.id ? (
                          <button className="v-icon-btn danger" title="确定删除" onClick={() => {
                            setConfirm(null);
                            removeEmote(c.id).then(onSaved, (e) => setError(String(e)));
                          }}><Glyph name="check" size={18} /></button>
                        ) : (
                          <button className="v-icon-btn" title="删除" onClick={() => setConfirm(c.id)}><Glyph name="trash" size={18} /></button>
                        )}
                      </div>
                    ))}
                  </div>
                )}
                <div
                  className={`v-dropzone${drag ? " drag" : ""}${picked ? " has" : ""}`}
                  onDragOver={(e) => { e.preventDefault(); setDrag(true); }}
                  onDragLeave={() => setDrag(false)}
                  onDrop={onDrop}
                  onPaste={onPaste}
                >
                  <label className="v-btn ghost">
                    <Glyph name="image" size={18} />
                    {picked ? "换图" : "选图 / 拖拽 / 粘贴"}
                    <input type="file" accept="image/png,image/jpeg,image/gif,image/webp" hidden onChange={(e) => void pickImage(e.target.files?.[0])} />
                  </label>
                  {picked && (
                    <>
                      {previewUrl && <img className="v-import-preview" src={previewUrl} alt="" />}
                      <input value={name} maxLength={24} aria-label="名字" placeholder="名字" onChange={(e) => setName(e.target.value)} />
                      <select value={fx} aria-label="出场效果" onChange={(e) => setFx(Number(e.target.value))}>
                        {FX_NAMES.map((n, i) => <option key={n} value={i}>{n}</option>)}
                      </select>
                      <label className="v-btn ghost">
                        <Glyph name="volume" size={18} />
                        {sound ? "已加音效" : "音效"}
                        <input type="file" accept="audio/ogg,audio/mpeg,audio/wav,.ogg,.mp3,.wav" hidden onChange={(e) => void pickSound(e.target.files?.[0])} />
                      </label>
                      <Action icon="check" label="导入" onClick={saveEmote} />
                    </>
                  )}
                  <span className="v-note">{s.customEmotes.length} / {MAX_CUSTOM_EMOTES} · 最多 1000×1000、1 MB</span>
                  {picked && (
                    <button type="button" className="v-icon-btn" disabled={busy} title="取消" onClick={() => { setPicked(null); setSound(null); }}>
                      <Glyph name="close" size={18} />
                    </button>
                  )}
                </div>
              </Fold>
            </div>
          </div>
        )}

        {tab === "team" && (
          <>
            <section className="v-tile v-tile-tight">
              <Row icon="text" label="队伍文字">
                <Toggle label="队伍文字" checked={s.teamText} onChange={(teamText) => onChange({ teamText })} />
              </Row>
              {s.teamText && (
                <>
                  <Row icon="monitor" label="显示">
                    <Choice label="文字显示" value={s.textStyle} options={[{ value: "spot", label: "原地" }, { value: "danmaku", label: "弹幕" }]}
                      onChange={(textStyle) => onChange({ textStyle })} />
                  </Row>
                  <Row icon="text" label="字号">
                    <Range label="字号" value={s.textSizePx} min={16} max={56} unit="px" onChange={(textSizePx) => onChange({ textSizePx })} />
                  </Row>
                  <Row icon="brush" label="特效">
                    <select className="v-select" aria-label="字幕特效" value={s.textEffect} onChange={(e) => onChange({ textEffect: e.target.value as TextEffect })}>
                      {EFFECTS.map((f) => <option key={f.value} value={f.value}>{f.label}</option>)}
                    </select>
                  </Row>
                  <Row icon="play" label="预览" title="在屏幕中间显示一段预览文字，看看当前特效的效果">
                    <Action icon="play" label="预览文字" onClick={() => void textPreview(s.textStyle, s.textSizePx, s.textEffect)} />
                  </Row>
                </>
              )}
            </section>
          </>
        )}

        {tab === "voice" && (
          <>
          <section className="v-tile">
            <Row icon="mic" label="语音" title="按住说话键说话，队友的话直接播放">
              <Toggle label="语音" checked={s.voice} onChange={(voice) => onChange({ voice })} />
            </Row>
            <Row icon="volume" label="队友音量">
              <Range label="队友音量" value={s.voiceVolume} min={0} max={100} onChange={(voiceVolume) => onChange({ voiceVolume })} />
            </Row>
            <Row icon="mic" label="麦克风">
              <select className="v-select" aria-label="麦克风" value={mic} onChange={(e) => onChange({ mic: e.target.value })}>
                <option value="">跟随系统</option>
                {mics.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
              </select>
              <button className="v-icon-btn" title={voice.test ?? "录 3 秒听听"} disabled={voice.test !== null} onClick={() => void voiceTest()}>
                <Glyph name="play" size={18} />
              </button>
            </Row>
            <Row icon="keyboard" label="说话键" title="按住说话">
              <KeyCapture kind="key" value={s.talkKey} onPick={(talkKey) => onChange({ talkKey })} />
            </Row>
            {voice.error && <p className="v-error">{voice.error}</p>}
          </section>
          <section className="v-tile v-mic-panel">
            <div className="v-mic-head">
              <Glyph name="mic" size={16} />
              <b>麦克风电平</b>
              <span className="v-page-spacer" />
              <span className="v-mic-hint">
                {s.voice ? `按住 ${vkLabel(s.talkKey)} 说话，队友听到的就是这里的音量` : "语音已关闭"}
              </span>
            </div>
            <MicWave live={s.voice} bars={56} max={72} className="v-mic-meter" />
          </section>
          </>
        )}

        {tab === "about" && (
          <div className="v-about">
            <section className="v-tile v-about-brand">
              <div className="v-about-bottom">
                <div className="v-about-ver">
                  <span>当前版本</span>
                  <b>{update?.current ?? "—"}</b>
                </div>
                {update && UPDATE_TEXT[update.phase] && <p className="v-note">{UPDATE_TEXT[update.phase]}</p>}
                {update?.error && <p className="v-error">{update.error}</p>}
                {update?.phase === "downloading" && update.progress !== null && (
                  <div className="v-progress"><i style={{ width: `${Math.round(update.progress * 100)}%` }} /></div>
                )}
                <div className="v-about-actions">
                  <Action icon="refresh" label="检查更新" tone="ghost" onClick={() => void updateCheck().then(setUpdate)} />
                  {update?.phase === "available" && <Action icon="download" label="下载安装" onClick={() => void updateInstall()} />}
                </div>
              </div>
            </section>
          </div>
        )}
        </div>

      </div>

      <footer className={`v-statusbar${status.enabled && !status.inputError ? " on" : ""}`}>
        <i className="v-state-dot" />
        <span>{status.inputError ? "输入设备不可用" : status.enabled ? "在线" : "离线"}</span>
        <span>·</span>
        <span>云端</span>
        <span className="v-sb-mid">改动即时保存</span>
        <span className="v-sb-right">{update?.current ? `v${update.current}` : ""}</span>
      </footer>
    </div>
  );
}
