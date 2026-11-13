// 右键菜单：点哪儿就在哪儿弹一小块，点外面 / 按 Esc / 滚轮就关。
// 定位说明：界面整体按 zoom 缩放过（html 上的 zoom），getBoundingClientRect 给的是缩放后的坐标，
// 而 fixed 定位的 left/top 用的是缩放前的本地坐标，两者差一个 zoom 系数，这里统一换算，
// 免得菜单飘到别的地方去。
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export interface MenuItem {
  key: string;
  label: string;
  /** 危险操作（删除、拉黑）用红字。 */
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

interface Props {
  /** 触发菜单的元素的位置（getBoundingClientRect 的结果）。 */
  anchor: DOMRect;
  items: MenuItem[];
  onClose: () => void;
}

/** html 上的 zoom 系数：取不到就按 1 算（浏览器里没缩放时是 1）。 */
function zoomFactor(): number {
  const z = parseFloat(document.documentElement.style.zoom);
  return Number.isFinite(z) && z > 0 ? z : 1;
}

export function ContextMenu({ anchor, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number }>({
    left: anchor.left / zoomFactor(),
    top: anchor.bottom / zoomFactor() + 4,
  });

  // 贴着触发行出现；菜单比屏幕还高就往上翻，宽了就往左收。
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const k = zoomFactor();
    const r = el.getBoundingClientRect();
    const vw = window.innerWidth / k;
    const vh = window.innerHeight / k;
    const w = r.width / k;
    const h = r.height / k;
    let left = anchor.left / k;
    let top = anchor.bottom / k + 4;
    if (left + w > vw - 8) left = Math.max(8, vw - 8 - w);
    if (top + h > vh - 8) top = Math.max(8, vh - 8 - h);
    setPos({ left, top });
  }, [anchor]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", onClose);
    window.addEventListener("wheel", onClose, { passive: true });
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("wheel", onClose);
    };
  }, [onClose]);

  return createPortal(
    <div ref={ref} className="v-menu" style={{ left: pos.left, top: pos.top }} role="menu">
      {items.map((it) => (
        <button
          key={it.key}
          type="button"
          role="menuitem"
          className={`v-menu-item${it.danger ? " danger" : ""}`}
          disabled={it.disabled}
          onClick={() => {
            if (it.disabled) return;
            it.onSelect();
            onClose();
          }}
        >
          {it.label}
        </button>
      ))}
    </div>,
    document.body,
  );
}
