// V2 表情页：轮盘 + 一整屏图标网格，点一下就换进选中的格子。
// 导入能力保留，但默认收起；44 个内置表情与 24 个导入名额都还在。
import { useEffect, useMemo, useState } from "react";
import { importEmote, previewEmote, removeEmote, type Settings } from "../api";
import { prepareEmote, prepareSound, toBase64, type PreparedEmote } from "../emoteImport";
import { EMOTE_CATALOG } from "../shared/emoteCatalog";
import { FX_NAMES, MAX_CUSTOM_EMOTES, emoteName, type CustomEmote } from "../shared/emotes";
import { emoteIconStyled } from "../shared/vectorAssets";
import { Glyph } from "./Glyph";
import { WheelDial } from "./WheelDial";
import { Action, Fold, Row, Toggle } from "./controls";

const DIRS = ["上", "右上", "右", "右下", "下", "左下", "左", "左上"];

const allRefs = (custom: readonly CustomEmote[]): string[] => [
  ...EMOTE_CATALOG.map((e) => e.slug),
  ...custom.map((c) => c.id),
];

function place(wheel: readonly string[], ref: string, slot: number): string[] {
  const next = [...wheel];
  const from = next.indexOf(ref);
  if (from >= 0) [next[from], next[slot]] = [next[slot], next[from]];
  else next[slot] = ref;
  return next;
}

export function Emotes({
  settings: s,
  onChange,
  onSaved,
}: {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onSaved: (s: Settings) => void;
}) {
  const [slot, setSlot] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<PreparedEmote | null>(null);
  const [sound, setSound] = useState<Uint8Array | null>(null);
  const [name, setName] = useState("");
  const [fx, setFx] = useState(0);
  const [confirm, setConfirm] = useState<string | null>(null);
  const full = s.customEmotes.length >= MAX_CUSTOM_EMOTES;
  const previewUrl = useMemo(
    () => (picked ? URL.createObjectURL(new Blob([picked.still.slice()], { type: "image/webp" })) : null),
    [picked],
  );
  useEffect(() => () => (previewUrl ? URL.revokeObjectURL(previewUrl) : undefined), [previewUrl]);
  const icon = (ref: string) => emoteIconStyled(ref, s.iconStyle);

  const pickImage = async (file: File | undefined) => {
    if (!file) return;
    const r = await prepareEmote(file);
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
  const save = () => {
    if (!picked) return;
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

  return (
    <div className="v-page">
      <div className="v-page-head">
        <Glyph name="smile" size={26} />
        <h1>表情</h1>
        <span className="v-page-spacer" />
        <Action icon="play" label="预览" tone="ghost" onClick={() => void previewEmote(s.emoteWheel[slot])} />
      </div>

      <div className="v-signal-grid">
        <section className="v-tile v-editor">
          <div className="v-mini-wheel emote">
            <svg viewBox="0 0 280 280" aria-hidden="true"><WheelDial /></svg>
            {s.emoteWheel.slice(0, 8).map((ref, i) => {
              const a = ((-90 + i * 45) * Math.PI) / 180;
              return (
                <button key={i} className={`v-mini-slot${i === slot ? " on" : ""}`}
                  style={{ left: `${50 + 34 * Math.cos(a)}%`, top: `${50 + 34 * Math.sin(a)}%` }}
                  onClick={() => setSlot(i)} aria-label={`${DIRS[i]}：${emoteName(ref, s.customEmotes)}`} aria-pressed={i === slot}>
                  <img src={icon(ref)} alt="" />
                </button>
              );
            })}
            <span className="v-mini-core">CTRL</span>
          </div>
          <div className="v-editor-side">
            <div className="v-editor-cur">
              <img src={icon(s.emoteWheel[slot])} alt="" />
              <b>{emoteName(s.emoteWheel[slot], s.customEmotes)}</b>
              <span>{DIRS[slot]}</span>
            </div>
            <div className="v-icon-grid emote">
              {allRefs(s.customEmotes).map((ref) => (
                <button key={ref} className={`v-icon-cell${ref === s.emoteWheel[slot] ? " on" : ""}`}
                  title={emoteName(ref, s.customEmotes)} onClick={() => onChange({ emoteWheel: place(s.emoteWheel, ref, slot) })}>
                  <img src={icon(ref)} alt="" />
                </button>
              ))}
            </div>
          </div>
        </section>

        <div className="v-side-col">
          <section className="v-tile">
            <Row icon="mouse" label="单击表情">
              <Toggle label="单击表情" checked={s.emoteClick} onChange={(emoteClick) => onChange({ emoteClick })} />
            </Row>
            <Row icon="volume" label="音效">
              <Toggle label="表情音效" checked={s.emoteSound} onChange={(emoteSound) => onChange({ emoteSound })} />
            </Row>
            <Row icon="users" label="队友表情" title="关掉后队友导入的表情显示成问号，你导入的也不发给队友">
              <Toggle label="队友表情" checked={s.friendEmotes} onChange={(friendEmotes) => onChange({ friendEmotes })} />
            </Row>
            <Row icon="smile" label="轮盘">
              <span className="v-value">{s.emoteTrigger === "off" ? "关" : "开"}</span>
            </Row>
          </section>

          <Fold icon="plus" label="导入" count={s.customEmotes.length}>
            {error && <p className="v-error">{error}</p>}
            {s.customEmotes.length > 0 && (
              <div className="v-custom-list">
                {s.customEmotes.map((c) => (
                  <div className="v-custom" key={c.id}>
                    <img src={icon(c.id)} alt="" />
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
            {full ? (
              <p className="v-note">24 / 24</p>
            ) : (
              <div className="v-import">
                <label className="v-btn ghost">
                  <Glyph name="image" size={18} />
                  {picked ? "换图" : "选图"}
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
                    <Action icon="check" label="导入" onClick={save} />
                  </>
                )}
                <span className="v-note">{s.customEmotes.length} / {MAX_CUSTOM_EMOTES}</span>
                <button type="button" className="v-icon-btn" disabled={busy} title="取消" onClick={() => { setPicked(null); setSound(null); }}>
                  <Glyph name="close" size={18} />
                </button>
              </div>
            )}
          </Fold>
        </div>
      </div>
    </div>
  );
}
