// 界面里用到的线条图标，直接写 SVG 路径，不引图标库。
const PATHS = {
  chat: "M4 5h16v11H9l-5 4z",
  chatPlus: "M4 5h16v11H9l-5 4zM12 8v5M9.5 10.5h5",
  gear: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM19 12l2-1.5-2-3.5-2.3.7a7 7 0 0 0-1.7-1L14.5 4h-5L9 6.7a7 7 0 0 0-1.7 1L5 7 3 10.5 5 12l-2 1.5L5 17l2.3-.7a7 7 0 0 0 1.7 1l.5 2.7h5l.5-2.7a7 7 0 0 0 1.7-1L19 17l2-3.5z",
  more: "M12 5h.01M12 12h.01M12 19h.01",
  close: "M6 6l12 12M18 6L6 18",
  bellOff: "M9 18h6M6 16h9M18 13V10a6 6 0 0 0-9.5-4.9M6 10v3l-1 3M3 3l18 18",
  pin: "M9 4h6l-1 6 3 3H7l3-3zM12 13v7",
  copy: "M9 9h10v11H9zM5 15V4h10",
  plus: "M12 5v14M5 12h14",
  search: "M11 5a6 6 0 1 0 0 12 6 6 0 0 0 0-12zM20 20l-4.5-4.5",
  speaker: "M4 9h4l5-4v14l-5-4H4zM17 9a4 4 0 0 1 0 6",
  smile: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM8.5 14.5a4.5 4.5 0 0 0 7 0M9 9.5h.01M15 9.5h.01",
  image: "M4 5h16v14H4zM4 16l4.5-4.5 4 4 2.5-2.5L20 18M15.5 9.5h.01",
  reply: "M10 8L5 12l5 4M5 12h9a5 5 0 0 1 5 5v1",
  at: "M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0zM16 12v1.5a2.5 2.5 0 0 0 5 0V12a9 9 0 1 0-3.5 7.1",
  trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
  logout: "M14 4h5v16h-5M10 8l-4 4 4 4M6 12h10",
  edit: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
  check: "M5 12.5l4.5 4.5L19 7",
  checks: "M2 12.5l4.5 4.5L16 7M11 15.5l1.5 1.5L22 7",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2",
  send: "M4 12l16-8-6 16-2.5-6.5z",
  arrowDown: "M12 5v14M6 13l6 6 6-6",
  arrowLeft: "M19 12H5M11 6l-6 6 6 6",
  chevronDown: "M6 9l6 6 6-6",
  chevronRight: "M9 6l6 6-6 6",
  globe: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c2.5 3 2.5 15 0 18M12 3c-2.5 3-2.5 15 0 18",
  archive: "M4 5h16v4H4zM5 9v10h14V9M10 13h4",
  sun: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
  info: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 11v6M12 7.5h.01",
  door: "M14 4h5v16h-5M4 12h11M11 8l4 4-4 4",
  users: "M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM2.5 20a6.5 6.5 0 0 1 13 0M16 4.5a3.5 3.5 0 0 1 0 6.5M18 14a5.5 5.5 0 0 1 3.5 6",
  camera: "M4 8h3l2-3h6l2 3h3v11H4zM12 17a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z",
  lock: "M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 7 0v3",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  rule: "M5 4h14v16H5zM9 9h6M9 13h6M9 17h3",
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={name === "more" ? 3 : 1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
