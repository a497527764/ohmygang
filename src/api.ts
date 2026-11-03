// 控制面板和 Rust 通信的唯一入口。数据流：界面调用这里 → Tauri invoke 到 Rust 的 api.rs → 返回结果；
// 信号开关在别处变了（托盘、暂停快捷键）时 Rust 推 "pings://enabled"，界面用 onEnabled 订阅。
// 设置的合法性（范围、触发键、快捷键、轮盘）由 Rust 的 settings2::normalize 把关，这里只做类型声明和转发。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Hotkey } from "./shared/keys";
import type { PingId } from "./shared/pings";

export type NamedTrigger = "alt" | "ctrl" | "shift" | "win" | "capslock" | "mouse4" | "mouse5";
export type TriggerKey = NamedTrigger | { vk: number };
export type Theme = "system" | "light" | "dark";

/** 和 Rust 的 settings2::Settings 对应。 */
export interface Settings {
  trigger: TriggerKey;
  dragThresholdPx: number;
  clickPing: boolean;
  clickPingId: PingId;
  gameCheck: boolean;
  wheel: PingId[];
  wheelSizePx: number;
  toggleHotkey: Hotkey;
  pingSizePx: number;
  pingDurationS: number;
  volume: number;
  muted: boolean;
  tickSound: boolean;
  theme: Theme;
  /** 不在同一局域网时经公共中继转发。 */
  allowInternet: boolean;
  /** 自己填的中继；空表示用默认列表。 */
  relays: string[];
}

export interface Status {
  enabled: boolean;
  /** 键鼠监听没起来的原因；正常时为 null。 */
  inputError: string | null;
}

export const readSettings = (): Promise<Settings> => invoke("settings_read");
/** 存整份设置，返回 Rust 修正后的结果。 */
export const writeSettings = (settings: Settings): Promise<Settings> => invoke("settings_write", { settings });
export const getStatus = (): Promise<Status> => invoke("status_get");
export const setEnabled = (on: boolean): Promise<void> => invoke("pings_set_enabled", { on });
/** 录按键期间挂起拦截，键都交给控制面板。 */
export const suspendInput = (on: boolean): Promise<void> => invoke("input_suspend", { on });
export const previewPing = (id: PingId): Promise<void> => invoke("ping_preview", { id });

export const onEnabled = (cb: (on: boolean) => void): Promise<UnlistenFn> =>
  listen<boolean>("pings://enabled", (e) => cb(e.payload));

// ---- 队伍（和 Rust 的 team.rs 对应） ----

export type AvatarData = { kind: "preset"; icon: number; color: number } | { kind: "image"; dataUrl: string };
/** 改头像时传给 Rust 的：图片给 Base64（不超过 800 字节）。 */
export type AvatarInput = { kind: "preset"; icon: number; color: number } | { kind: "image"; data: string };

export interface Me {
  memberId: string;
  nickname: string;
  fingerprint: string;
  avatar: AvatarData | null;
}

export interface TeamInfo {
  title: string | null;
  status: "joining" | "active";
  isOwner: boolean;
  code: string;
}

export interface Member {
  memberId: string;
  nickname: string | null;
  fingerprint: string;
  isOwner: boolean;
  isMe: boolean;
  avatar: AvatarData | null;
  stacks: number;
  /** 自己是 null。 */
  path: "lan" | "relay" | "offline" | null;
  status: "online" | "paused" | "away" | null;
  laning: boolean;
  platform: string | null;
}

export interface TeamView {
  me: Me;
  team: TeamInfo | null;
  members: Member[];
  lanDegraded: boolean;
  relaysOk: number;
  relaysTotal: number;
}

export const teamView = (): Promise<TeamView> => invoke("team_view");
export const teamCreate = (title: string): Promise<TeamView> => invoke("team_create", { title });
export const teamJoin = (code: string): Promise<TeamView> => invoke("team_join", { code });
export const teamCancel = (): Promise<TeamView> => invoke("team_cancel");
export const teamLeave = (): Promise<TeamView> => invoke("team_leave");
export const teamDissolve = (): Promise<TeamView> => invoke("team_dissolve");
export const teamRename = (title: string): Promise<TeamView> => invoke("team_rename", { title });
export const profileSet = (nickname: string, avatar: AvatarInput | null): Promise<TeamView> =>
  invoke("profile_set", { nickname, avatar });

/** 队伍有变化（进队、退出、改名、资料、在线状态）。 */
export const onTeamChanged = (cb: () => void): Promise<UnlistenFn> => listen("team://changed", () => cb());
/** 队友用的是别的协议版本（多半是 1.x）。 */
export const onProtocolMismatch = (cb: (remote: number) => void): Promise<UnlistenFn> =>
  listen<number>("team://mismatch", (e) => cb(e.payload));
