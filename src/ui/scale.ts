// 界面整体缩放：所有页面仍按 500×860 的设计尺寸排版，再整体放大 k 倍。
// k = 1.3 × 基准显示器缩放 / 当前显示器缩放，窗口逻辑尺寸同步变成 500k×860k，
// 于是窗口的物理大小不随显示器 DPI 变化；Rust 还会按屏幕可用区域收口，小屏上不会顶出屏幕。
// 基准缩放记在 localStorage：第一次打开的那块显示器（用户的 4K）就是「现在看着合适」的基准。
import { invoke } from "@tauri-apps/api/core";

export const DESIGN_W = 500;
export const DESIGN_H = 860;
const UI_SCALE = 1.3;
const REF_KEY = "ohg.refDpr";

function refDpr(): number {
  try {
    const saved = Number(localStorage.getItem(REF_KEY));
    if (saved >= 0.5 && saved <= 4) return saved;
    const now = window.devicePixelRatio || 1;
    localStorage.setItem(REF_KEY, String(now));
    return now;
  } catch {
    // 存储被禁用时退化为「以当前显示器为基准」，界面照常渲染
    return window.devicePixelRatio || 1;
  }
}

let lastK = 0;
let timer: ReturnType<typeof setTimeout> | null = null;

/** 启动时算一次；窗口被拖到另一块缩放不同的显示器（或系统缩放改了）时 resize 会再触发。 */
export function syncUiScale(): void {
  if (timer) clearTimeout(timer);
  timer = setTimeout(async () => {
    timer = null;
    let k = UI_SCALE;
    try {
      k = await invoke<number>("ui_sync_window", {
        dpr: refDpr(),
        w: DESIGN_W,
        h: DESIGN_H,
      });
    } catch {
      // 浏览器里预览（没有 Tauri）：按基准倍率直接渲染
    }
    if (!Number.isFinite(k)) k = UI_SCALE;
    k = Math.min(2.6, Math.max(0.7, k));
    if (Math.abs(k - lastK) < 0.001) return; // 幂等：zoom 引起的 resize 不再回写，避免抖动
    lastK = k;
    document.documentElement.style.zoom = String(k);
  }, 60);
}
