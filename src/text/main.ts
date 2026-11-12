// 聊天框（3.0-M4，原来的输入框）：文字快捷键打开、再按一次或 Esc 收起。上面是这个队伍的历史记录
// （文字、语音、信号、表情，最新的在最下面），下面是输入框；回车发送，发完不关。
// 数据流：打开时取一次全部记录（history_list）、队员名字（team_view）和导入表情（settings_read）；
// 之后 Rust 每记一条推 "history:added"，这里接着加；回放语音交给 Rust（history_play / history_stop），
// 放到哪条由 "voice:playing" 告诉这里。说的话和指令由 Rust 的 panel.rs 处理（清理、认指令、发给队伍）。
// 输入部分参考 lolPing src/renderer/text/main.ts（MIT，© 2026 Danrui Meng）。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Settings, TeamView } from "../api";
import { el } from "../shared/dom";
import { emoteName, type CustomEmote } from "../shared/emotes";
import { pingById } from "../shared/pings";
import { emoteIconStyled, pingIcon, type IconStyle } from "../shared/vectorAssets";
import "./text.css";

const MAX = 120;
/** 剩最后 20 个字时显示计数。 */
const COUNT_FROM = MAX - 20;
/** V2：默认不显示任何说明；指令放在输入框的 title 里，只有出错时才亮一条反馈。 */
const HINT = "";
const ANSWERS: Record<string, string> = {
  unknown: "未知指令",
  votesOff: "投票未开启",
  busy: "已有投票",
};

/** 和 Rust 的 history::EntryDto 对应。 */
interface Entry {
  id: number;
  memberId: string;
  me: boolean;
  color: string;
  ts: number;
  kind: "text" | "voice" | "ping" | "emote";
  text: string;
  voiceMs: number;
}
type Filter = "all" | "text" | "voice" | "signal";

const log = document.getElementById("log") as HTMLOListElement;
const input = document.getElementById("message") as HTMLInputElement;
const count = document.getElementById("count") as HTMLElement;
const hint = document.getElementById("hint") as HTMLElement;
const tabs = [...document.querySelectorAll<HTMLButtonElement>(".tabs button")];

let entries: Entry[] = [];
let names = new Map<string, string>();
let custom: CustomEmote[] = [];
let iconStyle: IconStyle = "league";
let filter: Filter = "all";
let playing: number | null = null;

const shows = (e: Entry): boolean =>
  filter === "all" || (filter === "signal" ? e.kind === "ping" || e.kind === "emote" : e.kind === filter);

function time(ts: number): string {
  const d = new Date(ts);
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  return d.toDateString() === new Date().toDateString() ? hm : `${d.getMonth() + 1}-${d.getDate()} ${hm}`;
}

/** 信号、表情那一行：小图加一句「发了……」。 */
function signal(icon: string, words: string): HTMLElement {
  const s = el("span", "signal");
  const img = el("img", "");
  img.src = icon;
  img.alt = "";
  s.append(img, el("span", "", words));
  return s;
}

function label(b: HTMLButtonElement): void {
  const on = Number(b.dataset.id) === playing;
  b.classList.toggle("playing", on);
  b.setAttribute("aria-label", on ? `停止，${b.dataset.secs} 秒语音` : `播放 ${b.dataset.secs} 秒语音`);
}

function voiceButton(e: Entry): HTMLButtonElement {
  const secs = Math.max(1, Math.round(e.voiceMs / 1000));
  const b = el("button", "voice");
  b.type = "button";
  b.dataset.id = String(e.id);
  // 像微信语音：越长的条越宽
  b.style.setProperty("--len", `${Math.min(100, 30 + secs * 5)}%`);
  b.dataset.secs = String(secs);
  label(b);
  b.append(el("i", ""), el("span", "", `${secs}″`));
  b.addEventListener("click", () => {
    void (playing === e.id ? invoke("history_stop") : invoke("history_play", { id: e.id }));
  });
  return b;
}

function row(e: Entry): HTMLLIElement {
  // 行的类名带 k- 前缀，免得和里面的按钮（.voice）撞名
  const li = el("li", `entry k-${e.kind}${e.me ? " me" : ""}`);
  // 队友身份色只落在行首细色条上，名字保持中性，避免一屏高饱和文字
  li.style.setProperty("--who", e.me ? "var(--cyan)" : e.color);
  const who = el("b", "who", e.me ? "我" : (names.get(e.memberId) ?? "队友"));
  const head = el("div", "head");
  head.append(who, el("time", "", time(e.ts)));
  li.append(head);
  if (e.kind === "text") li.append(el("p", "text", e.text));
  else if (e.kind === "voice") li.append(voiceButton(e));
  else if (e.kind === "ping") {
    const def = pingById(e.text);
    li.append(signal(pingIcon(e.text, iconStyle), def?.name ?? e.text));
  } else li.append(signal(emoteIconStyled(e.text, iconStyle), emoteName(e.text, custom)));
  return li;
}

function nearBottom(): boolean {
  return log.scrollHeight - log.scrollTop - log.clientHeight < 40;
}

function render(): void {
  const shown = entries.filter(shows);
  log.replaceChildren(...(shown.length ? shown.map(row) : [el("li", "empty", "还没有记录")]));
  log.scrollTop = log.scrollHeight;
}

async function reload(): Promise<void> {
  const [list, view, settings] = await Promise.all([
    invoke<Entry[]>("history_list"),
    invoke<TeamView>("team_view"),
    invoke<Settings>("settings_read"),
  ]);
  entries = list;
  names = new Map(view.members.map((m) => [m.memberId, m.nickname || m.fingerprint]));
  custom = settings.customEmotes;
  iconStyle = settings.iconStyle === "vector" ? "vector" : "league";
  render();
}

void listen<Entry>("history:added", ({ payload: e }) => {
  entries.push(e);
  if (!shows(e)) return;
  const stick = nearBottom();
  log.querySelector(".empty")?.remove();
  log.append(row(e));
  if (stick || e.me) log.scrollTop = log.scrollHeight;
});

void listen<number | null>("voice:playing", ({ payload }) => {
  playing = payload;
  for (const b of log.querySelectorAll<HTMLButtonElement>("button.voice")) label(b);
});

for (const t of tabs) {
  t.addEventListener("click", () => {
    filter = t.dataset.filter as Filter;
    for (const x of tabs) x.setAttribute("aria-selected", String(x === t));
    render();
    input.focus();
  });
}

// ---------- 输入 ----------

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
      showHint(HINT, false);
      return;
    }
    showHint(ANSWERS[answer] ?? "未生效", true);
  } catch (e) {
    showHint(String(e), true);
  } finally {
    sending = false;
  }
}

const close = (): void => {
  input.value = "";
  clamp();
  void invoke("text_cancel");
};

// 输入法正在拼字时不截断（会打断拼写），回车是确认候选字。
input.addEventListener("input", (e) => {
  if (!(e as InputEvent).isComposing) clamp();
  showHint(HINT, false);
});
input.addEventListener("compositionend", clamp);
// 表单从不提交（发送走回车的处理）：内容安全策略不让在 HTML 里写 onsubmit，在这里拦（3.1.1）
input.form?.addEventListener("submit", (e) => e.preventDefault());
input.addEventListener("keydown", (e) => {
  if (e.isComposing) return;
  if (e.key === "Enter") {
    e.preventDefault();
    void send();
  }
});
// Esc 在哪儿按都收起（焦点可能在筛选按钮或语音按钮上）
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !e.isComposing) {
    e.preventDefault();
    close();
  }
});
document.getElementById("close")?.addEventListener("click", close);
document.getElementById("send")?.addEventListener("click", () => void send());
// 点了别处（聊天框失去焦点）就收起，免得一直挡在屏幕上。
// 但刚弹出来时全屏游戏常把前台抢回去（Windows 不让后台进程久占焦点）：还没拿到过焦点、
// 或拿到不到 0.3 秒就被抢走的 blur 不算「点走」，不收起——否则盒子弹出即被关掉，
// 看上去就是打字快捷键按了没反应（3.0.1）。
let focusedAt = 0;
window.addEventListener("focus", () => {
  focusedAt = performance.now();
  input.focus();
});
window.addEventListener("blur", () => {
  if (!focusedAt || performance.now() - focusedAt < 300) {
    focusedAt = 0;
    return;
  }
  void invoke("text_cancel");
});

void listen("text:open", () => {
  showHint(HINT, false);
  void reload();
  requestAnimationFrame(() => input.focus());
});
// 第一次打开时页面还没加载完，"text:open" 已经发过了：加载完时取一次记录，窗口拿到焦点时把光标放进输入框。
showHint(HINT, false);
clamp();
void reload();
requestAnimationFrame(() => input.focus());
