// 建网页元素的两个小工具，透明窗口、队伍面板、投票、聊天框共用（3.1.1 以前每个文件各写一个）。

/** 建一个元素：类名（空的不设），可选的文字。文字只当文字放：内容可能来自队友的电脑。 */
export function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

export const div = (className: string, text?: string): HTMLDivElement => el("div", className, text);
