// 界面上显示的名字和时间文字。
import type { Member, MemberPath, Message } from "./api";

export const PATH_NAMES: Record<MemberPath, string> = { lan: "局域网", relay: "中继", offline: "离线" };

export function displayName(id: string, members: Member[]): string {
  const m = members.find((x) => x.memberId === id);
  return m?.nickname || m?.fingerprint.slice(0, 6) || id.slice(0, 6);
}

export function noticeText(m: Message, members: Member[]): string {
  const who = displayName(m.authorId, members);
  switch (m.notice) {
    case "join":
      return `${who} 加入了群聊`;
    case "leave":
      return `${who} 退出了群聊`;
    case "rename":
      return `${who} 把群名改为「${m.title ?? ""}」`;
    case "config":
      return `${who} 修改了心之钢规则`;
    case "dissolve":
      return `${who} 解散了群聊`;
    default:
      return "";
  }
}

const pad = (n: number) => String(n).padStart(2, "0");

/// 今天只显示时分；今年显示月日；更早带年份。
export function timeText(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const t = new Date(now);
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (d.toDateString() === t.toDateString()) return hm;
  if (d.getFullYear() === t.getFullYear()) return `${d.getMonth() + 1}月${d.getDate()}日 ${hm}`;
  return `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日 ${hm}`;
}

/// 房间列表右上角的时间：今天显示时分，其余只显示日期。
export function shortTime(ms: number, now = Date.now()): string {
  if (!ms) return "";
  const d = new Date(ms);
  const t = new Date(now);
  if (d.toDateString() === t.toDateString()) return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (d.getFullYear() === t.getFullYear()) return `${d.getMonth() + 1}/${d.getDate()}`;
  return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`;
}
