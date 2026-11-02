// 和 Rust 核心通信的唯一入口（接口文档第 12 节）。
// 数据流：界面调用这里的函数 → Tauri invoke 到 Rust 命令 → 节点任务里的 Core 执行，新事件由节点广播出去；
// 别人发来的消息、层数变化、房间状态变化由 Rust 推送 "hs://event" 事件，界面用 onEvent 订阅。
// 业务规则（叠层判定、加解密、同步）都在 Rust 侧，这里只做类型声明和转发。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Identity {
  memberId: string;
  nickname: string;
  fingerprint: string;
}

export type RoomStatus = "joining" | "active" | "left" | "dissolved";

export interface Room {
  roomId: string;
  title: string | null;
  status: RoomStatus;
  isOwner: boolean;
  unread: number;
  lastActive: number;
}

/// 现在经哪条路径能联系上这位成员（架构 6.5）。
export type MemberPath = "lan" | "relay" | "offline";

export interface Member {
  memberId: string;
  fingerprint: string;
  nickname: string | null;
  stacks: number;
  isOwner: boolean;
  inRoom: boolean;
  path: MemberPath;
}

export interface RelayHealth {
  url: string;
  state: "connecting" | "ok" | "cooling" | "retired";
  lastEchoMs: number | null;
}

export interface Message {
  kind: "chat" | "notice";
  eventId: string;
  authorId: string;
  tsMs: number;
  displayTs: number;
  text: string | null;
  delivery: "pending" | "sent" | null;
  notice: "join" | "leave" | "rename" | "config" | "dissolve" | null;
  title: string | null;
}

export interface Cursor {
  displayTs: number;
  eventId: string;
}

export type HsEvent =
  | { type: "timeline"; roomId: string; message: Message; isLive: boolean }
  | { type: "stacks"; roomId: string; changes: [string, number][] }
  | { type: "live"; roomId: string; memberId: string; newStacks: number }
  | { type: "room"; roomId: string }
  | { type: "fork"; roomId: string; memberId: string }
  | { type: "status"; roomId: string; status: RoomStatus }
  | { type: "mismatch"; roomId: string; remoteProto: number };

export const api = {
  identity: () => invoke<Identity>("identity_get"),
  setNickname: (nickname: string) => invoke<Identity>("identity_set_nickname", { nickname }),
  rooms: () => invoke<Room[]>("rooms_list"),
  createRoom: (title: string) => invoke<{ room: Room; code: string }>("rooms_create", { title }),
  joinRoom: (code: string) => invoke<Room>("rooms_join", { code }),
  roomCode: (roomId: string) => invoke<string>("rooms_get_code", { roomId }),
  members: (roomId: string) => invoke<Member[]>("rooms_members", { roomId }),
  markRead: (roomId: string) => invoke<void>("rooms_mark_read", { roomId }),
  send: (roomId: string, text: string) => invoke<Message>("chat_send_text", { roomId, text }),
  messages: (roomId: string, before: Cursor | null, limit: number) =>
    invoke<Message[]>("chat_list", { roomId, before, limit }),
  /// 关闭「允许互联网」时为 null。
  relayStatus: () => invoke<RelayHealth[] | null>("relay_status"),
  onEvent: (cb: (e: HsEvent) => void): Promise<UnlistenFn> =>
    listen<HsEvent>("hs://event", (e) => cb(e.payload)),
};

/// 按显示时间、再按事件 ID 排序（与 Rust 侧分页顺序一致），并去重。
export function mergeMessages(list: Message[], incoming: Message[]): Message[] {
  const byId = new Map(list.map((m) => [m.eventId, m]));
  for (const m of incoming) byId.set(m.eventId, m);
  return [...byId.values()].sort((a, b) =>
    a.displayTs !== b.displayTs ? a.displayTs - b.displayTs : a.eventId < b.eventId ? -1 : 1,
  );
}
