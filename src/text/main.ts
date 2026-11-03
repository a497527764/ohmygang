// 队伍文字的输入框：快捷键打开，回车发送，Esc 取消。说的话和指令由 Rust 的 panel.rs 处理（清理、认指令、发给队伍）。
// 做法参考 lolPing src/renderer/text/main.ts（MIT，© 2026 Danrui Meng）。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./text.css";

const MAX = 120;
/** 剩最后 20 个字时显示计数。 */
const COUNT_FROM = MAX - 20;
const HINT = "回车发送，Esc 取消。指令：/ff 投降，/noff 反对投降，/ready 准备确认";
const ANSWERS: Record<string, string> = {
  unknown: "不认识这个指令。可以用 /ff、/noff、/ready",
  votesOff: "投票关着，在控制面板的「功能」页打开",
  busy: "已经有一个投票在进行",
};

const input = document.getElementById("message") as HTMLInputElement;
const count = document.getElementById("count") as HTMLElement;
const hint = document.getElementById("hint") as HTMLElement;

function showHint(text: string, warn: boolean): void {
  hint.textContent = text;
  hint.classList.toggle("warn", warn);
}

/** 按字算（一个汉字、一个表情都算一个），不按 UTF-16。 */
function clamp(): void {
  const points = [...input.value];
  if (points.length > MAX) input.value = points.slice(0, MAX).join("");
  const used = Math.min(points.length, MAX);
  count.hidden = used < COUNT_FROM;
  count.textContent = `${used}/${MAX}`;
}

/** 按住回车会连发：一次只交一句，不然同一句会发两遍。 */
let sending = false;

async function send(): Promise<void> {
  if (sending) return;
  sending = true;
  try {
    const answer = await invoke<string>("text_send", { text: input.value });
    if (answer === "sent" || answer === "empty") {
      input.value = "";
      clamp();
      return;
    }
    showHint(ANSWERS[answer] ?? HINT, true);
  } catch (e) {
    showHint(String(e), true);
  } finally {
    sending = false;
  }
}

// 输入法正在拼字时不截断（会打断拼写），回车是确认候选字。
input.addEventListener("input", (e) => {
  if (!(e as InputEvent).isComposing) clamp();
  showHint(HINT, false);
});
input.addEventListener("compositionend", clamp);
input.addEventListener("keydown", (e) => {
  if (e.isComposing) return;
  if (e.key === "Enter") {
    e.preventDefault();
    void send();
  } else if (e.key === "Escape") {
    e.preventDefault();
    input.value = "";
    clamp();
    void invoke("text_cancel");
  } else if (e.key === "Tab") {
    e.preventDefault(); // 焦点离开输入框后回车和 Esc 就没人听了
  }
});
// 点了别处（输入框失去焦点）就收起，免得一直挡在屏幕上。
window.addEventListener("blur", () => void invoke("text_cancel"));

void listen("text:open", () => {
  showHint(HINT, false);
  requestAnimationFrame(() => input.focus());
});
// 第一次打开时页面还没加载完，"text:open" 已经发过了：加载完和窗口拿到焦点时也把光标放进输入框。
window.addEventListener("focus", () => input.focus());
showHint(HINT, false);
clamp();
requestAnimationFrame(() => input.focus());
