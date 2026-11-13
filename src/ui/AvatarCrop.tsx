// 头像裁剪：任意图片放进方形取景框，拖动选位置、滚轮/滑杆缩放；
// 确认时把框里的区域裁成 200×200、自动压好体积（见 avatarImport.ts）。
import { useEffect, useRef, useState } from "react";
import { Action, Range } from "./controls";
import { cropAvatar, loadAvatarImage } from "../avatarImport";

/** 取景框边长（CSS 像素）。 */
const VIEW = 284;

interface Pos {
  /** 图片左上角在取景框里的位置。 */
  x: number;
  y: number;
  /** 缩放倍数（1 = 原始像素）。 */
  s: number;
}

export function AvatarCrop({ file, onCancel, onDone }: {
  file: File;
  onCancel: () => void;
  onDone: (r: { data: string; dataUrl: string }) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [bitmap, setBitmap] = useState<ImageBitmap | null>(null);
  const [pos, setPos] = useState<Pos | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const drag = useRef<{ px: number; py: number; ox: number; oy: number } | null>(null);

  const minScale = bitmap ? Math.max(VIEW / bitmap.width, VIEW / bitmap.height) : 1;
  const maxScale = Math.min(minScale * 12, 16);

  // 载入选中的图：解不出来直接报错（没有形状/尺寸要求，任意图都进得来）
  useEffect(() => {
    let closed = false;
    setError(null);
    setBitmap(null);
    setPos(null);
    void loadAvatarImage(file).then((r) => {
      if (closed) {
        if (r.ok) r.value.close();
        return;
      }
      if (!r.ok) {
        setError(r.error);
        return;
      }
      const b = r.value;
      const s = Math.max(VIEW / b.width, VIEW / b.height);
      setBitmap(b);
      setPos({ s, x: (VIEW - b.width * s) / 2, y: (VIEW - b.height * s) / 2 });
    });
    return () => {
      closed = true;
    };
  }, [file]);

  // 换图或退出时释放位图
  useEffect(() => () => { bitmap?.close(); }, [bitmap]);

  // 画布跟随位置重画（带 DPR，缩放时依然清晰）
  useEffect(() => {
    const cv = canvasRef.current;
    if (!cv || !bitmap || !pos) return;
    const dpr = window.devicePixelRatio || 1;
    if (cv.width !== Math.round(VIEW * dpr)) {
      cv.width = Math.round(VIEW * dpr);
      cv.height = Math.round(VIEW * dpr);
    }
    const ctx = cv.getContext("2d");
    if (!ctx) return;
    ctx.save();
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, VIEW, VIEW);
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(bitmap, pos.x, pos.y, bitmap.width * pos.s, bitmap.height * pos.s);
    ctx.restore();
  }, [bitmap, pos]);

  // 取景框始终留在图里：位置和缩放都夹住
  const clamp = (p: Pos): Pos => {
    if (!bitmap) return p;
    const s = Math.min(Math.max(p.s, minScale), maxScale);
    return {
      s,
      x: Math.min(Math.max(p.x, VIEW - bitmap.width * s), 0),
      y: Math.min(Math.max(p.y, VIEW - bitmap.height * s), 0),
    };
  };

  const zoomTo = (ns: number) =>
    setPos((p) => {
      if (!p) return p;
      // 围绕取景框中心缩放，框里的画面不跳
      const cx = VIEW / 2;
      const cy = VIEW / 2;
      const k = ns / p.s;
      return clamp({ s: ns, x: cx - (cx - p.x) * k, y: cy - (cy - p.y) * k });
    });

  // 滚轮缩放要 preventDefault，React 的 onWheel 是被动的，用原生监听
  useEffect(() => {
    const cv = canvasRef.current;
    if (!cv || !bitmap) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      zoomTo(pos ? pos.s * (e.deltaY < 0 ? 1.15 : 1 / 1.15) : minScale);
    };
    cv.addEventListener("wheel", onWheel, { passive: false });
    return () => cv.removeEventListener("wheel", onWheel);
  }, [bitmap, pos, minScale]);

  const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (!pos) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { px: e.clientX, py: e.clientY, ox: pos.x, oy: pos.y };
  };
  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const d = drag.current;
    if (!d || !pos) return;
    setPos(clamp({ ...pos, x: d.ox + (e.clientX - d.px), y: d.oy + (e.clientY - d.py) }));
  };
  const onPointerUp = () => { drag.current = null; };

  const reset = () => {
    if (!bitmap) return;
    const s = minScale;
    setPos({ s, x: (VIEW - bitmap.width * s) / 2, y: (VIEW - bitmap.height * s) / 2 });
  };

  const confirm = async () => {
    if (!bitmap || !pos || busy) return;
    setBusy(true);
    setError(null);
    const r = await cropAvatar(bitmap, {
      x: -pos.x / pos.s,
      y: -pos.y / pos.s,
      w: VIEW / pos.s,
      h: VIEW / pos.s,
    });
    setBusy(false);
    if (!r.ok) {
      setError(r.error);
      return;
    }
    onDone(r.value);
  };

  if (error && !bitmap) {
    return (
      <div className="v-crop">
        <p className="v-error">{error}</p>
        <div className="v-crop-actions">
          <Action icon="close" label="取消" tone="ghost" onClick={onCancel} />
        </div>
      </div>
    );
  }

  return (
    <div className="v-crop">
      <canvas ref={canvasRef} className="v-crop-canvas" style={{ width: VIEW, height: VIEW, touchAction: "none" }}
        aria-label="头像裁剪预览"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      />
      <div className="v-crop-zoom">
        <Range label="缩放" value={Math.round((pos?.s ?? minScale) * 1000) / 1000}
          min={Math.round(minScale * 1000) / 1000} max={Math.round(maxScale * 1000) / 1000} step={0.001}
          unit="×" onChange={zoomTo} />
      </div>
      <div className="v-crop-actions">
        <Action icon="close" label="取消" tone="ghost" onClick={onCancel} disabled={busy} />
        <Action icon="refresh" label="重置" tone="ghost" onClick={reset} disabled={busy || !pos} />
        <Action icon="check" label={busy ? "处理中…" : "就这样"} onClick={() => void confirm()} disabled={busy || !bitmap} />
      </div>
      {error && <p className="v-error">{error}</p>}
      <span className="v-note">拖动选位置，滚轮或滑杆缩放；确认后自动裁成 200×200 并压好体积</span>
    </div>
  );
}
