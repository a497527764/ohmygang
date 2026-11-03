// 和 Rust 核心通信的唯一入口（接口文档第 12 节）。
// 数据流：界面调用这里的函数 → Tauri invoke 到 Rust 命令 → 节点任务里的 Core 执行，新事件由节点广播出去；
// 别人发来的消息、层数变化、房间状态变化由 Rust 推送 "hs://event" 事件，界面用 onEvent 订阅。
// 业务规则（叠层判定、加解密、同步）都在 Rust 侧，这里只做类型声明和转发。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/// 头像：内置的是图标和底色的下标（见 avatars.ts），上传的图片是 data URL。没有时界面按指纹自动生成。
export type Avatar = { kind: "preset"; icon: number; color: number } | { kind: "image"; dataUrl: string };

/// 改头像时传给 Rust 的：图片给 Base64（不超过 800 字节）。
export type AvatarInput = { kind: "preset"; icon: number; color: number } | { kind: "image"; data: string };

export interface Identity {
  memberId: string;
  nickname: string;
  fingerprint: string;
  avatar: Avatar | null;
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
  /// 最后一条消息（「乙：……」，自己发的不带名字）。
  preview: string | null;
  /// 未读里有人 @ 我。
  mentioned: boolean;
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
  avatar: Avatar | null;
}

export interface RoomRule {
  epoch: number;
  windowMs: number;
  threshold: number;
  cooldownMs: number;
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
  theme: "system" | "light" | "dark";
  /// 内置默认中继，只读。
  defaultRelays: string[];
}

export interface NetworkStatus {
  lanPort: number;
  /// 局域网端口被占，用的是随机端口，收不到别人的局域网广播。
  lanDegraded: boolean;
}

export interface Reply {
  eventId: string;
  excerpt: string;
  /// 原消息不在本机时为 null。
  authorId: string | null;
}

export interface ImageInfo {
  width: number;
  height: number;
  /// 各片都到齐了，可以用 api.image 取图。
  ready: boolean;
}

export interface Message {
  kind: "chat" | "notice";
  eventId: string;
  authorId: string;
  tsMs: number;
  displayTs: number;
  /// 文字消息的文字；图片和系统提示为 null。
  text: string | null;
  delivery: "pending" | "sent" | null;
  notice: "join" | "leave" | "rename" | "config" | "dissolve" | null;
  title: string | null;
  reply: Reply | null;
  /// 被 @ 的人：成员 ID 的前 16 位。
  mentions: string[];
  image: ImageInfo | null;
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
  | { type: "delivered"; roomId: string; eventIds: string[] }
  | { type: "imageReady"; roomId: string; eventId: string };

export const api = {
  identity: () => invoke<Identity>("identity_get"),
  /// avatar 为 null 表示用自动生成的头像。
  setProfile: (nickname: string, avatar: AvatarInput | null) =>
    invoke<Identity>("identity_set_profile", { nickname, avatar }),
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
  roomRule: (roomId: string) => invoke<RoomRule | null>("rooms_rule", { roomId }),
  renameRoom: (roomId: string, title: string) => invoke<void>("rooms_rename", { roomId, title }),
  setRoomRule: (roomId: string, rule: { windowMs: number; threshold: number; cooldownMs: number }) =>
    invoke<void>("rooms_set_rule", { roomId, ...rule }),
  leaveRoom: (roomId: string) => invoke<void>("rooms_leave", { roomId }),
  dissolveRoom: (roomId: string) => invoke<void>("rooms_dissolve", { roomId }),
  cancelJoin: (roomId: string) => invoke<void>("rooms_cancel_join", { roomId }),
  leaderboard: (roomId: string) => invoke<LeaderboardEntry[]>("heartsteel_leaderboard", { roomId }),
  /// replyTo：引用的消息 ID；mentions：被 @ 的成员 ID。
  send: (roomId: string, text: string, replyTo: string | null = null, mentions: string[] = []) =>
    invoke<Message>("chat_send", { roomId, text, replyTo, mentions }),
  /// data：压缩好的图片的 Base64。
  sendImage: (roomId: string, data: string, width: number, height: number) =>
    invoke<Message>("chat_send_image", { roomId, data, width, height }),
  /// 收齐时返回 data URL，否则 null。
  image: (roomId: string, eventId: string) => invoke<string | null>("chat_image", { roomId, eventId }),
  search: (roomId: string, query: string) => invoke<Message[]>("chat_search", { roomId, query }),
  /// 只清空本机的显示。
  clearHistory: (roomId: string) => invoke<Room>("rooms_clear_history", { roomId }),
  /// 删除聊天（只影响本机）：还在的群藏起来、来新消息再出现，已退出或已解散的删除本机记录。
  deleteChat: (roomId: string) => invoke<void>("rooms_delete_chat", { roomId }),
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
