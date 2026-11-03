// 录一个按键或快捷键：点一下进入录制，按下要的键就记下，按 Esc 或点别处取消。
// 录制期间让 Rust 挂起拦截，否则触发键、暂停快捷键会在到达控制面板之前被拦下。
import { useEffect, useRef, useState } from "react";
import { suspendInput } from "../api";
import { UNSAFE_TRIGGER_VKS, codeToVk, eventMods, hotkeyParts, isModifierCode, MOD, vkLabel, type Hotkey } from "../shared/keys";

type Props =
  | { kind: "key"; value: number | null; onPick: (vk: number) => void }
  | { kind: "hotkey"; value: Hotkey; onPick: (h: Hotkey) => void };

const PRIMARY_MODS = MOD.ctrl | MOD.alt | MOD.win;

export function KeyCapture(props: Props) {
  const [recording, setRecording] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  // 录制期间的按键处理读最新的 props（父组件每次渲染都给新对象）
  const latest = useRef(props);
  latest.current = props;

  useEffect(() => {
    if (!recording) return;
    void suspendInput(true);
    const stop = () => {
      setRecording(false);
      void suspendInput(false);
    };
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") return stop();
      if (isModifierCode(e.code)) return; // 等真正的键
      const vk = codeToVk(e.code);
      if (vk === null) return setHint("这个键认不出来，换一个");
      const p = latest.current;
      if (p.kind === "key") {
        if (UNSAFE_TRIGGER_VKS.includes(vk)) return setHint(`${vkLabel(vk)} 会影响正常打字，换一个`);
        p.onPick(vk);
      } else {
        const mods = eventMods(e);
        if ((mods & PRIMARY_MODS) === 0) return setHint("要带上 Ctrl、Alt 或 Win");
        p.onPick({ mods, vk });
      }
      setHint(null);
      stop();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", stop);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", stop);
      void suspendInput(false);
    };
  }, [recording]);

  const parts =
    props.kind === "hotkey" ? hotkeyParts(props.value) : props.value === null ? [] : [vkLabel(props.value)];

  return (
    <span className="key-capture">
      <button
        className={`keys${recording ? " recording" : ""}`}
        onClick={() => {
          setHint(null);
          setRecording((r) => !r);
        }}
      >
        {recording ? (
          props.kind === "hotkey" ? "按下新的快捷键…" : "按下一个键…"
        ) : parts.length > 0 ? (
          parts.map((p) => <kbd key={p}>{p}</kbd>)
        ) : (
          "点这里录一个键"
        )}
      </button>
      {hint && <small className="hint warn">{hint}</small>}
    </span>
  );
}
