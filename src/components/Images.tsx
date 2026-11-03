// 聊天里的图片（V1.1）：各片收齐后向 Rust 取 data URL 显示，取过的缓存起来；点图片全屏看大图。
import { useEffect, useState } from "react";
import { api, type ImageInfo } from "../api";

/// eventId → data URL。图片内容不会变，取一次就够了。
const cache = new Map<string, string>();

/// 显示框：等比缩进 240×240，太细长的也不小于 60。
function box(info: ImageInfo): { width: number; height: number } {
  const k = Math.min(1, 240 / Math.max(info.width, info.height, 1));
  return { width: Math.max(60, Math.round(info.width * k)), height: Math.max(60, Math.round(info.height * k)) };
}

interface Props {
  roomId: string;
  eventId: string;
  info: ImageInfo;
  onOpen: (url: string) => void;
}

export function MessageImage({ roomId, eventId, info, onOpen }: Props) {
  const [url, setUrl] = useState<string | null>(cache.get(eventId) ?? null);
  const [broken, setBroken] = useState(false);

  useEffect(() => {
    if (url || !info.ready) return;
    let live = true;
    api
      .image(roomId, eventId)
      .then((u) => {
        if (!live) return;
        if (u) {
          cache.set(eventId, u);
          setUrl(u);
        } else setBroken(true);
      })
      .catch(() => live && setBroken(true));
    return () => {
      live = false;
    };
  }, [roomId, eventId, info.ready, url]);

  const size = box(info);
  if (url) {
    return (
      <button className="msg-image" style={size} onClick={() => onOpen(url)} title="点击看大图">
        <img src={url} alt="图片" draggable={false} />
      </button>
    );
  }
  return (
    <div className="msg-image placeholder" style={size}>
      {broken ? "图片损坏" : "图片接收中…"}
    </div>
  );
}

export function ImageViewer({ url, onClose }: { url: string; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="viewer" onMouseDown={onClose} role="dialog" aria-label="查看图片">
      <img src={url} alt="图片" />
    </div>
  );
}
