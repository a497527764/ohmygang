// 界面里用到的几个线条图标，直接写 SVG，不引图标库。
const PATHS = {
  chat: "M4 5h16v11H9l-5 4z",
  gear: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM19 12l2-1.5-2-3.5-2.3.7a7 7 0 0 0-1.7-1L14.5 4h-5L9 6.7a7 7 0 0 0-1.7 1L5 7 3 10.5 5 12l-2 1.5L5 17l2.3-.7a7 7 0 0 0 1.7 1l.5 2.7h5l.5-2.7a7 7 0 0 0 1.7-1L19 17l2-3.5z",
  more: "M5 12h.01M12 12h.01M19 12h.01",
  close: "M6 6l12 12M18 6L6 18",
  bellOff: "M9 18h6M6 16h9M18 13V10a6 6 0 0 0-9.5-4.9M6 10v3l-1 3M3 3l18 18",
  pin: "M9 4h6l-1 6 3 3H7l3-3zM12 13v7",
  copy: "M9 9h10v11H9zM5 15V4h10",
  plus: "M12 5v14M5 12h14",
  search: "M11 5a6 6 0 1 0 0 12 6 6 0 0 0 0-12zM20 20l-4.5-4.5",
  speaker: "M4 9h4l5-4v14l-5-4H4zM17 9a4 4 0 0 1 0 6",
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
      strokeWidth={name === "more" ? 3 : 1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
