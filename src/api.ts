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
  muted: boolean;
  pinned: boolean;
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

export interface LeaderboardEntry {
  memberId: string;
  stacks: number;
  /// 从 1 开始，同层同名次。
  rank: number;
}

export interface RelayHealth {
  url: string;
  state: "connecting" | "ok" | "cooling" | "retired";
  lastEchoMs: number | null;
}

export interface Settings {
  internetAllowed: boolean;
  relays: string[];
  soundEnabled: boolean;
  /// 0–100。
  volume: number;
  /// 新进群时最多同步最近多少条聊天：经局域网、经中继。
  historyLimitLan: number;
  historyLimitRelay: number;
  /// 内置默认中继，只读。
  defaultRelays: string[];
}

export interface NetworkStatus {
  lanPort: number;
  /// 局域网端口被占，用的是随机端口，收不到别人的局域网广播。
  lanDegraded: boolean;
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
  | { type: "mismatch"; roomId: string; remoteProto: number }
  | { type: "delivered"; roomId: string; eventIds: string[] };

export const api = {
  identity: () => invoke<Identity>("identity_get"),
  setNickname: (nickname: string) => invoke<Identity>("identity_set_nickname", { nickname }),
  rooms: () => invoke<Room[]>("rooms_list"),
  createRoom: (title: string) => invoke<{ room: Room; code: string }>("rooms_create", { title }),
  joinRoom: (code: string) => invoke<Room>("rooms_join", { code }),
  roomCode: (roomId: string) => invoke<string>("rooms_get_code", { roomId }),
  members: (roomId: string) => invoke<Member[]>("rooms_members", { roomId }),
  markRead: (roomId: string) => invoke<void>("rooms_mark_read", { roomId }),
  /// 没给的项不改。
  setRoomPrefs: (roomId: string, prefs: { muted?: boolean; pinned?: boolean }) =>
    invoke<Room>("rooms_set_prefs", { roomId, muted: prefs.muted ?? null, pinned: prefs.pinned ?? null }),
  /// 界面当前打开的房间，Rust 据此决定哪个房间同步得更勤。
  setActiveRoom: (roomId: string | null) => invoke<void>("rooms_set_active", { roomId }),
  leaderboard: (roomId: string) => invoke<LeaderboardEntry[]>("heartsteel_leaderboard", { roomId }),
  send: (roomId: string, text: string) => invoke<Message>("chat_send_text", { roomId, text }),
  messages: (roomId: string, before: Cursor | null, limit: number) =>
    invoke<Message[]>("chat_list", { roomId, before, limit }),
  settings: () => invoke<Settings>("settings_get"),
  /// 没给的项不改。
  updateSettings: (patch: Partial<Omit<Settings, "defaultRelays">>) => invoke<Settings>("settings_update", { patch }),
  /// 关闭「允许互联网」时为 null。
  relayStatus: () => invoke<RelayHealth[] | null>("relay_status"),
  /// 返回备份文件的完整路径。stamp：本地时间，如 20261103-1530。
  exportBackup: (kind: "identity" | "full", passphrase: string, stamp: string) =>
    invoke<string>("backup_export", { kind, passphrase, stamp }),
  /// data：文件内容的 Base64。本机已有房间时要 overwrite = true。
  importBackup: (data: string, passphrase: string, overwrite: boolean) =>
    invoke<Identity>("backup_import", { data, passphrase, overwrite }),
  networkStatus: () => invoke<NetworkStatus>("network_status"),
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
