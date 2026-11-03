// 「表情」页：表情触发键、表情轮盘、导入自己的图、大小和声音、显示队友的表情。改动交给 App 统一保存；
// 导入和删除直接调 Rust（要写文件），返回的新设置交回 App。合法性（撞键、轮盘、导入列表）由 Rust 把关。
import { useEffect, useMemo, useState } from "react";
import { importEmote, previewEmote, removeEmote, type NamedTrigger, type Settings } from "../api";
import { prepareEmote, prepareSound, toBase64, type PreparedEmote } from "../emoteImport";
import { EMOTE_CATALOG } from "../shared/emoteCatalog";
import { FX_NAMES, MAX_CUSTOM_EMOTES, emoteIcon, emoteName, type CustomEmote } from "../shared/emotes";
import { Icon } from "./Icon";
import { KeyCapture } from "./KeyCapture";
import { Row, Slider, Switch, TRIGGERS, howTo } from "./PingsPage";
import { DIRECTIONS, placeOnSlot, slotPos } from "./WheelEditor";

const OFF = "off";
const CUSTOM = "custom";

interface Props {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  /** 导入、删除后 Rust 存好的设置。 */
  onSaved: (s: Settings) => void;
}

export function EmotesPage({ settings: s, onChange, onSaved }: Props) {
  const t = s.emoteTrigger;
  const off = t === OFF;
  return (
    <div className="page">
      <h1>表情</h1>

      <section className="card">
        <h2>触发</h2>
        <Row label="表情触发键" hint={off ? "表情关着" : `${howTo(t)}，打开表情轮盘。不能和信号触发键相同`}>
          <select
            value={off ? OFF : typeof t === "object" ? CUSTOM : t}
            onChange={(e) => {
              const v = e.target.value;
              if (v === CUSTOM) onChange({ emoteTrigger: { vk: 0x54 } }); // 先给个 T，用户再录
              else onChange({ emoteTrigger: v as NamedTrigger | "off" });
            }}
          >
            <option value={OFF}>不用表情</option>
            {TRIGGERS.map((x) => (
              <option key={x.value} value={x.value} disabled={x.value === s.trigger}>
                {x.label}
                {x.value === s.trigger ? "（信号在用）" : ""}
              </option>
            ))}
            <option value={CUSTOM}>自定义按键…</option>
          </select>
          {typeof t === "object" && <KeyCapture kind="key" value={t.vk} onPick={(vk) => onChange({ emoteTrigger: { vk } })} />}
        </Row>
        <Row label="单击也放表情" hint="关着时，表情触发键加单击照常交给下面的程序">
          <Switch label="单击也放表情" checked={s.emoteClick} onChange={(emoteClick) => onChange({ emoteClick })} />
          {s.emoteClick && (
            <select value={s.clickEmote} onChange={(e) => onChange({ clickEmote: e.target.value })}>
              {allRefs(s.customEmotes).map((ref) => (
                <option key={ref} value={ref}>
                  {emoteName(ref, s.customEmotes)}
                </option>
              ))}
            </select>
          )}
        </Row>
      </section>

      <section className="card">
        <h2>表情轮盘</h2>
        <EmoteWheelEditor wheel={s.emoteWheel} custom={s.customEmotes} onChange={(emoteWheel) => onChange({ emoteWheel })} />
      </section>

      <CustomEmotes settings={s} onChange={onChange} onSaved={onSaved} />

      <section className="card">
        <h2>大小和声音</h2>
        <Slider label="表情大小" value={s.emoteSizePx} min={80} max={300} step={1} unit=" 像素" onChange={(v) => onChange({ emoteSizePx: v })} />
        <Row label="表情音效" hint="音量跟信号页的音量走">
          <Switch label="表情音效" checked={s.emoteSound} onChange={(emoteSound) => onChange({ emoteSound })} />
        </Row>
        <Row label="显示队友导入的表情" hint="关掉后队友导入的表情显示成问号，你导入的也不发给队友">
          <Switch label="显示队友导入的表情" checked={s.friendEmotes} onChange={(friendEmotes) => onChange({ friendEmotes })} />
        </Row>
      </section>
    </div>
  );
}

/** 能放上轮盘的全部表情：内置的在前，导入的在后。 */
const allRefs = (custom: readonly CustomEmote[]): string[] => [...EMOTE_CATALOG.map((e) => e.slug), ...custom.map((c) => c.id)];

function EmoteWheelEditor({ wheel, custom, onChange }: { wheel: string[]; custom: CustomEmote[]; onChange: (w: string[]) => void }) {
  const [slot, setSlot] = useState(0);
  return (
    <div className="wheel-editor emote">
      <div className="mini-wheel" role="group" aria-label="表情轮盘的 8 格">
        <div className="mini-ring" />
        <div className="mini-core">EMOTE</div>
        {wheel.map((ref, i) => (
          <button
            key={i}
            className={`mini-slot${i === slot ? " selected" : ""}`}
            style={slotPos(i)}
            onClick={() => setSlot(i)}
            aria-label={`${DIRECTIONS[i]}：${emoteName(ref, custom)}`}
            aria-pressed={i === slot}
          >
            <img src={emoteIcon(ref)} alt="" />
          </button>
        ))}
      </div>
      <div className="wheel-side">
        <p className="wheel-pick">
          <span>
            向{DIRECTIONS[slot]}拖：<b>{emoteName(wheel[slot], custom)}</b>
          </span>
          <button className="soft small" onClick={() => void previewEmote(wheel[slot])}>
            <Icon name="play" size={14} />
            预览
          </button>
        </p>
        <p className="muted">换成：</p>
        <div className="palette emote-palette">
          {allRefs(custom).map((ref) => (
            <button
              key={ref}
              className={`palette-item${ref === wheel[slot] ? " current" : ""}`}
              onClick={() => onChange(placeOnSlot(wheel, ref, slot))}
              title={emoteName(ref, custom)}
            >
              <img src={emoteIcon(ref)} alt="" />
              <span>{emoteName(ref, custom)}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function CustomEmotes({ settings: s, onChange, onSaved }: Props) {
  const [picked, setPicked] = useState<PreparedEmote | null>(null);
  const [sound, setSound] = useState<{ name: string; bytes: Uint8Array } | null>(null);
  const [name, setName] = useState("");
  const [fx, setFx] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const full = s.customEmotes.length >= MAX_CUSTOM_EMOTES;
  const previewUrl = useMemo(() => (picked ? URL.createObjectURL(new Blob([picked.still.slice()], { type: "image/webp" })) : null), [picked]);
  useEffect(() => () => (previewUrl ? URL.revokeObjectURL(previewUrl) : undefined), [previewUrl]);

  const pickImage = async (file: File | undefined) => {
    if (!file) return;
    setError(null);
    const r = await prepareEmote(file);
    if (!r.ok) {
      setError(r.error);
      return;
    }
    setPicked(r.value);
    setName(r.value.name.slice(0, 24));
  };

  const pickSound = async (file: File | undefined) => {
    if (!file) return;
    setError(null);
    const r = await prepareSound(file);
    if (r.ok) setSound({ name: file.name, bytes: r.value });
    else setError(r.error);
  };

  const save = () => {
    if (!picked) return;
    setBusy(true);
    importEmote({
      still: toBase64(picked.still),
      anim: picked.anim ? toBase64(picked.anim) : null,
      sound: sound ? toBase64(sound.bytes) : null,
      name,
      fx,
    })
      .then(
        (saved) => {
          onSaved(saved);
          setPicked(null);
          setSound(null);
          setName("");
          setFx(0);
          setError(null);
        },
        (e) => setError(String(e)),
      )
      .finally(() => setBusy(false));
  };

  const update = (id: string, patch: Partial<CustomEmote>) =>
    onChange({ customEmotes: s.customEmotes.map((c) => (c.id === id ? { ...c, ...patch } : c)) });

  return (
    <section className="card">
      <h2>导入的表情</h2>
      <p className="muted">
        用自己的图（PNG、JPEG、GIF、WebP）做表情，最多 {MAX_CUSTOM_EMOTES} 个。队友收到的是缩小的静态图；动图和音效只在你这边放。
      </p>
      {error && <p className="banner error">{error}</p>}

      {s.customEmotes.length > 0 && (
        <ul className="custom-emotes">
          {s.customEmotes.map((c) => (
            <li key={c.id} className="custom-emote">
              <img src={emoteIcon(c.id)} alt="" />
              <input
                value={c.name}
                maxLength={24}
                aria-label="表情名字"
                onChange={(e) => update(c.id, { name: e.target.value })}
              />
              <select value={c.fx} aria-label="出场效果" onChange={(e) => update(c.id, { fx: Number(e.target.value) })}>
                {FX_NAMES.map((n, i) => (
                  <option key={n} value={i}>
                    {n}
                  </option>
                ))}
              </select>
              <span className="hint">
                {[c.animated && "动图", c.sound && "有音效"].filter(Boolean).join("、")}
              </span>
              <button className="soft small" onClick={() => void previewEmote(c.id)}>
                <Icon name="play" size={14} />
                预览
              </button>
              {confirm === c.id ? (
                <>
                  <button
                    className="primary danger small"
                    onClick={() => {
                      setConfirm(null);
                      removeEmote(c.id).then(onSaved, (e) => setError(String(e)));
                    }}
                  >
                    删除
                  </button>
                  <button className="soft small" onClick={() => setConfirm(null)}>
                    取消
                  </button>
                </>
              ) : (
                <button className="soft small danger" onClick={() => setConfirm(c.id)}>
                  删除
                </button>
              )}
            </li>
          ))}
        </ul>
      )}

      {full ? (
        <p className="muted">已经导入了 {MAX_CUSTOM_EMOTES} 个，删掉一个才能再导入。</p>
      ) : (
        <div className="emote-import">
          <label className="soft file-pick">
            {picked ? "换一张图" : "选一张图"}
            <input type="file" accept="image/png,image/jpeg,image/gif,image/webp" onChange={(e) => void pickImage(e.target.files?.[0])} />
          </label>
          {picked && (
            <>
              {previewUrl && <img className="import-preview" src={previewUrl} alt="缩好的静态图" />}
              <input value={name} maxLength={24} aria-label="名字" placeholder="名字" onChange={(e) => setName(e.target.value)} />
              <select value={fx} aria-label="出场效果" onChange={(e) => setFx(Number(e.target.value))}>
                {FX_NAMES.map((n, i) => (
                  <option key={n} value={i}>
                    {n}
                  </option>
                ))}
              </select>
              <label className="soft file-pick">
                {sound ? `音效：${sound.name}` : "加音效（可选）"}
                <input type="file" accept="audio/ogg,audio/mpeg,audio/wav,.ogg,.mp3,.wav" onChange={(e) => void pickSound(e.target.files?.[0])} />
              </label>
              <button className="primary" disabled={busy} onClick={save}>
                导入
              </button>
              {picked.anim && <span className="hint">动图，你这边会放原动画</span>}
            </>
          )}
        </div>
      )}
    </section>
  );
}
