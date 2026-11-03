// 透明窗口的页面：只画，不算规则。Rust 告诉它开轮盘、高亮哪格、关轮盘、在哪放什么信号、弹什么提示、对线的对手在哪；
// 它在空闲时报告一声，让 Rust 把窗口挪开。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FALLBACK_TEXTURE, TICK_SOUND, allSoundNames, pingById, soundUrl, textureUrl } from "../shared/pings";
import heartsteelIcon from "../assets/heartsteel.png";
import heartsteelSound from "../assets/heartsteel.mp3";
import { IdleWatch } from "./idleWatch";
import { LaneStage, spawnHeartsteel, type LaneState } from "./lane";
import { spawnPing, type PingTag } from "./pingFx";
import { SoundBank } from "./soundBank";
import { Wheel } from "./wheel";

/** 和 Rust 的 settings2::OverlaySettings 对应。 */
interface OverlaySettings {
  wheelSizePx: number;
  pingSizePx: number;
  pingDurationS: number;
  /** 0 到 1，静音时是 0。 */
  volume: number;
  tickSound: boolean;
  wheel: string[];
  clickPingId: string;
}

const pingLayer = document.getElementById("pings") as HTMLElement;
const toastLayer = document.getElementById("toasts") as HTMLElement;
const wheelSvg = document.getElementById("wheel") as unknown as SVGSVGElement;
const laneLayer = document.getElementById("lane") as HTMLElement;
const lane = new LaneStage(laneLayer);

const sounds = new SoundBank();
let settings: OverlaySettings | null = null;

const iconOf = (id: string): string => textureUrl(pingById(id)?.icon ?? FALLBACK_TEXTURE);

const wheel = new Wheel(wheelSvg, () => {
  if (settings?.tickSound) sounds.play(TICK_SOUND, settings.volume * 0.5);
});

function apply(s: OverlaySettings): void {
  settings = s;
  wheel.setSize(s.wheelSizePx);
  wheel.setIcons(s.wheel.map(iconOf));
  wheel.setCenterIcon(iconOf(s.clickPingId));
}

const TOAST_MS = 2600;
function showToast(title: string, body: string): void {
  const t = document.createElement("div");
  t.className = "toast";
  const b = document.createElement("b");
  b.textContent = title;
  t.append(b);
  if (body) {
    const small = document.createElement("small");
    small.textContent = body;
    t.append(small);
  }
  toastLayer.replaceChildren(t);
  setTimeout(() => {
    t.classList.add("out");
    setTimeout(() => t.remove(), 200);
  }, TOAST_MS);
}

void listen<OverlaySettings>("overlay:settings", (e) => apply(e.payload));
void listen<{ x: number; y: number }>("wheel:open", (e) => wheel.open(e.payload.x, e.payload.y));
void listen<{ slot: number | null }>("wheel:hot", (e) => wheel.setHot(e.payload.slot ?? -1));
void listen("wheel:close", () => wheel.close());
void listen<{ id: string; x: number; y: number; tag?: PingTag }>("ping:spawn", ({ payload: p }) => {
  const def = pingById(p.id);
  if (!def || !settings) return;
  spawnPing(pingLayer, def, p.x, p.y, { sizePx: settings.pingSizePx, durationS: settings.pingDurationS, tag: p.tag });
  sounds.play(def.sound, settings.volume);
});
void listen<{ title: string; body: string }>("toast:show", (e) => showToast(e.payload.title, e.payload.body));
void listen<LaneState>("lane:state", (e) => {
  lane.render(e.payload);
  idle.check();
});
void listen<{ x: number; y: number; label: string }>("lane:hit", ({ payload: p }) => {
  spawnHeartsteel(pingLayer, heartsteelIcon, p.x, p.y, p.label);
  if (settings) sounds.play("heartsteel", settings.volume);
});

// 信号、心之钢、提示在这两层里，对线的东西在 lane 层里，轮盘开着时它的 class 有 open。
const idle = new IdleWatch(
  () => !wheel.isOpen && pingLayer.childElementCount === 0 && toastLayer.childElementCount === 0 && lane.isEmpty,
  (v) => void invoke("overlay_idle", { idle: v }),
);
const changes = new MutationObserver(() => idle.check());
for (const l of [pingLayer, toastLayer]) changes.observe(l, { childList: true });
changes.observe(wheelSvg, { attributes: true, attributeFilter: ["class"] });

void invoke<OverlaySettings>("overlay_hello").then((s) => {
  apply(s);
  idle.check();
});
void sounds.load([
  ...allSoundNames().map((name) => ({ key: name, url: soundUrl(name) })),
  { key: "heartsteel", url: heartsteelSound },
]);
