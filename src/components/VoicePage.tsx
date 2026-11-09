// 「语音」页：开关、说话键（在「快捷键」页改）、麦克风、队友语音的音量、试音。
// 数据流：设置交给 App 统一保存；麦克风列表和状态（打不开的原因、试音进行到哪）打开时取一次，之后跟着 "voice://changed" 变。
import { useEffect, useState } from "react";
import { onVoice, voiceTest, voiceView, type Mic, type Settings, type VoiceStatus } from "../api";
import { vkLabel } from "../shared/keys";
import { Row, Slider, Switch } from "./controls";

interface Props {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onShortcuts: () => void;
}

const TEST_TEXT: Record<NonNullable<VoiceStatus["test"]>, string> = { recording: "正在录 3 秒…", playing: "正在回放…" };

export function VoicePage({ settings: s, onChange, onShortcuts }: Props) {
  const [mics, setMics] = useState<Mic[]>([]);
  const [status, setStatus] = useState<VoiceStatus>({ error: null, test: null });

  useEffect(() => {
    void voiceView().then((v) => {
      setMics(v.mics);
      setStatus(v.status);
    });
    const off = onVoice(setStatus);
    return () => void off.then((f) => f());
  }, []);

  // 保存的麦克风拔掉了：列表里没有它，就显示成「跟随系统」（Rust 那边也是这么用的）
  const mic = mics.some((m) => m.id === s.mic) ? s.mic : "";

  return (
    <div className="page">
      <h1>语音</h1>
      {status.error && <p className="banner error">{status.error}</p>}

      <section className={`hero${s.voice ? " on" : ""}`}>
        <div>
          <b>{s.voice ? "语音开着" : "语音关着"}</b>
          <p>
            按住 <kbd>{vkLabel(s.talkKey)}</kbd> 说话
          </p>
        </div>
        <Switch label="语音开关" checked={s.voice} onChange={(voice) => onChange({ voice })} />
      </section>

      <section className="card">
        <h2>设置</h2>
        <Row label="说话键" tip="按住说话，松开发给队友。点一下照常打出这个字">
          <kbd>{vkLabel(s.talkKey)}</kbd>
          <button className="soft small" onClick={onShortcuts}>
            去快捷键页改
          </button>
        </Row>
        <Row label="麦克风">
          <select aria-label="麦克风" value={mic} onChange={(e) => onChange({ mic: e.target.value })}>
            <option value="">跟随系统</option>
            {mics.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        </Row>
        <Slider label="队友语音的音量" value={s.voiceVolume} min={0} max={100} step={1} unit="" onChange={(v) => onChange({ voiceVolume: v })} />
        <Row label="试音" tip="录 3 秒，在本机编码再解码放给你听，不发给任何人" hint={status.test && TEST_TEXT[status.test]}>
          <button className="soft" disabled={status.test !== null} onClick={() => void voiceTest()}>
            录 3 秒听听
          </button>
        </Row>
      </section>
    </div>
  );
}
