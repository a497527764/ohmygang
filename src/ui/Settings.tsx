// 3.3 设置页：唯一二级页，顶部一排 HUD tab，分区承载原来的信号 / 表情 / 队伍 / 按键 / 功能 / 关于。
// 轮盘编辑器（信号、表情各一个）从旧独立页搬进来；说明文字只保留在 title 里。
import { useCallback, useEffect, useState } from "react";
import {
  importEmote, onUpdate, onVoice, previewEmote, previewPing, profileSet, removeEmote, teamView,
  textPreview, updateCheck, updateInstall, updateView, voiceTest, voiceView,
  type AvatarInput, type KeyClash, type Me, type Settings, type Status, type TextEffect,
  type UpdateView, type VoiceStatus,
} from "../api";
import { base64OfDataUrl, randomPreset } from "../avatarImport";
import { prepareCustomEmote, toBase64 } from "../emoteImport";
import { Avatar } from "../components/Avatar";
import { AvatarCrop } from "./AvatarCrop";
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
/** 云端连接状态的说法（和主页电源区一致）。 */
const CLOUD_TEXT: Record<Status["cloudState"], string> = {
  connecting: "连接中",
  ok: "已连接",
  down: "未连接",
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
  const placeEmote = (ref: string, slot: number) => {
    const next = [...s.emoteWheel];
    const from = next.indexOf(ref);
    if (from >= 0) [next[from], next[slot]] = [next[slot], next[from]];
    else next[slot] = ref;
    return next;
  };

  // ---- 表情导入：即拖即导，无暂存区 ----
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [dragTarget, setDragTarget] = useState<string | null>(null); // "new" = 拖到空白处，否则 = 拖到某个表情上

  const doImport = useCallback(async (file: File | undefined, replaceId?: string) => {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      const r = await prepareCustomEmote(file);
      if (!r.ok) { setError(r.error); return; }
      const saved = await importEmote({
        still: toBase64(r.value.still),
        anim: r.value.anim ? toBase64(r.value.anim) : null,
        sound: null,
        replaceId: replaceId ?? null,
      });
      onSaved(saved);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [onSaved]);

  const onDrop = useCallback((e: React.DragEvent, replaceId?: string) => {
    e.preventDefault();
    e.stopPropagation();
    setDragTarget(null);
    const f = e.dataTransfer.files?.[0];
    if (f) void doImport(f, replaceId);
  }, [doImport]);

  // 粘贴导入挂到 window 上：dropzone 是普通 div 收不到 paste；正在输入的框里粘贴不抢
  useEffect(() => {
    if (tab !== "emotes") return;
    const onPaste = (e: ClipboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      const item = Array.from(e.clipboardData?.items ?? []).find((i) => i.type.startsWith("image/"));
      const f = item?.getAsFile();
      if (f) {
        e.preventDefault();
        void doImport(f);
      }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [tab, doImport]);

  // ---- 个人资料：昵称 + 头像（改动即时保存，见状态栏） ----
  const [me, setMe] = useState<Me | null>(null);
  const [nick, setNick] = useState("");
  const [profileErr, setProfileErr] = useState<string | null>(null);
  const [profileBusy, setProfileBusy] = useState(false);

  useEffect(() => {
    if (tab !== "common") return;
    void teamView().then((v) => {
      setMe(v.me);
      setNick(v.me.nickname);
      setProfileErr(null);
    });
  }, [tab]);

  // 现在的头像拆回 profile_set 的两个参数：内置头像给 avatar；图片头像给 avatarImage（200×200 大图通道）
  const avatarArgs = (): { avatar: AvatarInput | null; avatarImage: string | null } => {
    const a = me?.avatar;
    if (a?.kind === "preset") return { avatar: { kind: "preset", icon: a.icon, color: a.color }, avatarImage: null };
    if (a?.kind === "image") return { avatar: null, avatarImage: base64OfDataUrl(a.dataUrl) };
    return { avatar: null, avatarImage: null };
  };

  const saveProfile = useCallback(async (nickname: string, avatar: AvatarInput | null, avatarImage: string | null) => {
    setProfileBusy(true);
    setProfileErr(null);
    try {
      const v = await profileSet(nickname, avatar, avatarImage);
      setMe(v.me);
      setNick(v.me.nickname);
    } catch (e) {
      setProfileErr(String(e));
    } finally {
      setProfileBusy(false);
    }
  }, []);

  const saveNickname = () => {
    if (!me || profileBusy) return;
    const n = nick.trim();
    if (n === me.nickname) return;
    const { avatar, avatarImage } = avatarArgs();
    void saveProfile(n, avatar, avatarImage);
  };

  const pickRandomPreset = useCallback(() => {
    void saveProfile(nick.trim(), randomPreset(), null);
  }, [nick, saveProfile]);

  // 选了图先进裁剪框（任意图片都行），裁好确认才保存
  const [cropFile, setCropFile] = useState<File | null>(null);
  const onCropDone = useCallback((r: { data: string; dataUrl: string }) => {
    setCropFile(null);
    void saveProfile(nick.trim(), null, r.data);
  }, [nick, saveProfile]);

  return (
    <div className="v-page">
      <header className="v-page-head">
        <button type="button" className="v-back" onClick={onBack} title="返回主页 · Ctrl+E / Esc">
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
          <section className="v-tile v-profile">
            {cropFile ? (
              <AvatarCrop file={cropFile} onCancel={() => setCropFile(null)} onDone={onCropDone} />
            ) : (
            <div className="v-profile-face">
              <Avatar fingerprint={me?.fingerprint ?? ""} name={nick || me?.nickname || null} avatar={me?.avatar ?? null} size={88} />
              <div className="v-profile-side">
                <div className="v-profile-actions">
                  <label className={`v-btn ghost${profileBusy ? " disabled" : ""}`}>
                    <Glyph name="image" size={18} />
                    {profileBusy ? "保存中…" : "选头像"}
                    <input type="file" accept="image/png,image/jpeg,image/gif,image/webp" hidden disabled={profileBusy}
                      onChange={(e) => { setCropFile(e.target.files?.[0] ?? null); e.currentTarget.value = ""; }} />
                  </label>
                  <Action icon="smile" label="随机头像" tone="ghost" disabled={profileBusy} onClick={pickRandomPreset} />
                </div>
                <span className="v-note">任意图片都能选，裁剪框选好位置自动压成 200×200 同步给队友</span>
              </div>
            </div>
            )}
            <Row icon="users" label="昵称" title="进队后显示在队友的成员列表里；不设就显示编号">
              <input className="v-input v-profile-nick" value={nick} maxLength={32} placeholder="不设就显示编号"
                onChange={(e) => setNick(e.target.value)}
                onBlur={saveNickname}
                onKeyDown={(e) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur(); }} />
            </Row>
            {profileErr && <p className="v-error">{profileErr}</p>}
          </section>

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
                <div className="v-emote-sec-title">默认表情</div>
                <div className="v-icon-grid emote">
                  {EMOTE_CATALOG.map((e) => (
                    <button key={e.slug} className={`v-icon-cell${e.slug === s.emoteWheel[emoteSlot[0]] ? " on" : ""}`}
                      title={emoteName(e.slug, s.customEmotes)}
                      onClick={() => onChange({ emoteWheel: placeEmote(e.slug, emoteSlot[0]) })}>
                      <img src={emoteIcon(e.slug)} alt="" />
                    </button>
                  ))}
                </div>
                <Fold icon="smile" label="自定义表情" count={s.customEmotes.length} defaultOpen>
                  {error && <p className="v-error">{error}</p>}
                  {s.customEmotes.length > 0 && (
                    <div className="v-icon-grid emote">
                      {s.customEmotes.map((c: CustomEmote) => (
                        <div key={c.id} className="v-icon-wrap"
                          onDragOver={(e) => { e.preventDefault(); e.stopPropagation(); setDragTarget(c.id); }}
                          onDragLeave={() => setDragTarget(null)}
                          onDrop={(e) => onDrop(e, c.id)}>
                          <button className={`v-icon-cell${c.id === s.emoteWheel[emoteSlot[0]] ? " on" : ""}${dragTarget === c.id ? " drag" : ""}`}
                            title={emoteName(c.id, s.customEmotes)}
                            onClick={() => onChange({ emoteWheel: placeEmote(c.id, emoteSlot[0]) })}>
                            <img src={emoteIcon(c.id)} alt="" />
                          </button>
                          {confirm === c.id ? (
                            <button className="v-cell-del danger" title="确定删除"
                              onClick={(e) => { e.stopPropagation(); setConfirm(null); removeEmote(c.id).then(onSaved, (e2) => setError(String(e2))); }}>
                              <Glyph name="check" size={11} />
                            </button>
                          ) : (
                            <button className="v-cell-del" title="删除"
                              onClick={(e) => { e.stopPropagation(); setConfirm(c.id); }}>
                              <Glyph name="close" size={11} />
                            </button>
                          )}
                        </div>
                      ))}
                    </div>
                  )}
                  <div
                    className={`v-dropzone${dragTarget === "new" ? " drag" : ""}`}
                    onDragOver={(e) => { e.preventDefault(); setDragTarget("new"); }}
                    onDragLeave={() => setDragTarget(null)}
                    onDrop={(e) => onDrop(e)}
                  >
                    <label className={`v-btn ghost${busy ? " disabled" : ""}`}>
                      <Glyph name="image" size={18} />
                      {busy ? "导入中…" : "选图 / 拖拽 / 粘贴"}
                      <input type="file" accept="image/png,image/jpeg,image/gif,image/webp" hidden disabled={busy} onChange={(e) => void doImport(e.target.files?.[0])} />
                    </label>
                    <span className="v-note">{s.customEmotes.length} / {MAX_CUSTOM_EMOTES} · 拖到表情上替换 · 最多 1000×1000、1 MB</span>
                  </div>
                </Fold>
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
                      {(() => {
                        const options = [...new Set([...s.emoteWheel, ...s.customEmotes.map((c) => c.id)])];
                        const label = (r: string) => (r.startsWith("c:") ? r.slice(2, 10) : r);
                        return (
                          <>
                            {/* 引用的表情已被删：给个兜底项，别让下拉显示成空白 */}
                            {!options.includes(s.clickEmote) && (
                              <option value={s.clickEmote}>{label(s.clickEmote)}（已删除）</option>
                            )}
                            {options.map((r) => <option key={r} value={r}>{label(r)}</option>)}
                          </>
                        );
                      })()}
                    </select>
                  </Row>
                )}
                <Row icon="target" label="表情大小">
                  <Range label="表情大小" value={s.emoteSizePx} min={80} max={300} unit="px" onChange={(emoteSizePx) => onChange({ emoteSizePx })} />
                </Row>
                <Row icon="bolt" label="弹出方式" title="导入表情的出场效果，全部统一用这一个">
                  <select className="v-select" aria-label="弹出方式" value={s.emoteFx} onChange={(e) => onChange({ emoteFx: Number(e.target.value) })}>
                    {FX_NAMES.map((n, i) => <option key={n} value={i}>{n}</option>)}
                  </select>
                </Row>
                <Row icon="volume" label="音效">
                  <Toggle label="表情音效" checked={s.emoteSound} onChange={(emoteSound) => onChange({ emoteSound })} />
                </Row>
                <Row icon="users" label="队友表情" title="关掉后队友导入的表情显示成问号，你导入的也不发给队友">
                  <Toggle label="队友表情" checked={s.friendEmotes} onChange={(friendEmotes) => onChange({ friendEmotes })} />
                </Row>
              </section>
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
                    <Range label="字号" value={s.textSizePx} min={16} max={100} unit="px" onChange={(textSizePx) => onChange({ textSizePx })} />
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
                {update?.note && <p className="v-note">{update.note}</p>}
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
            <section className="v-tile v-net">
              <div className="v-net-head">
                <Glyph name="bolt" size={16} />
                <b>网络</b>
                <span className="v-page-spacer" />
                <span className={`v-net-state ${status.cloudState}`}>
                  {CLOUD_TEXT[status.cloudState]}
                  {status.relayMs != null ? ` · ${status.relayMs} ms` : ""}
                </span>
              </div>
              <Row icon="download" label="更新下载直连" title="检查更新和下载安装包不走系统代理（梯子常把 GitHub 绕远）">
                <Toggle label="更新下载直连" checked={s.netDirect} onChange={(netDirect) => onChange({ netDirect })} />
              </Row>
              <p className="v-note">
                语音、表情、队伍消息、云端心跳都是直连服务器 IP（14.103.234.253），本来就不走系统代理——
                这里的开关只影响更新下载。用全局模式（TUN）的梯子会在网络层接管所有流量，软件绕不过去，
                请在梯子客户端里把服务器 IP 设成直连；上面的延迟是到服务器的往返时间，忽高忽低多半是梯子在绕路。
              </p>
              <p className="v-note">
                中继连接每 15 秒有一次心跳，空闲时也不会被 NAT 或梯子悄悄掐断；
                如果「云端未连接」频繁出现，先把服务器 IP 加进梯子的直连规则，再看上面的延迟。
              </p>
            </section>
          </div>
        )}
        </div>

      </div>

      <footer className={`v-statusbar${status.enabled && !status.inputError ? " on" : ""}`}>
        <i className="v-state-dot" />
        <span>{status.inputError ? "输入设备不可用" : status.enabled ? "在线" : "离线"}</span>
        <span>·</span>
        <span className={`v-sb-cloud ${status.cloudState}`}>云端{CLOUD_TEXT[status.cloudState]}</span>
        <span className="v-sb-mid">改动即时保存</span>
        <span className="v-sb-right">{update?.current ? `v${update.current}` : ""}</span>
      </footer>
    </div>
  );
}
