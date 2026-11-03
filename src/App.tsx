// 主界面（V1.1 第二版，参考 WhatsApp 的布局）：最左边窄栏（上面是聊天，下面是设置和我的头像），
// 左栏按窄栏的选择显示聊天列表、设置或个人资料，中间始终是聊天区，右侧按需打开群资料或搜索。
// 数据都从 api.ts 取；收到 hs://event 时只刷新受影响的部分。
// 成员路径、中继和局域网状态没有推送，每 5 秒刷新一次。音效在这里决定播不播：本机音效开关、房间免打扰、音量。
// 主题：设置里选了浅色或深色就写到 <html data-theme>，跟随系统时去掉这个属性，由 CSS 按系统设置切换。
import { useCallback, useEffect, useRef, useState } from "react";
import heartsteel from "./assets/heartsteel.png";
import {
  api,
  mergeMessages,
  type HsEvent,
  type Identity,
  type Member,
  type Message,
  type NetworkStatus,
  type RelayHealth,
  type Room,
  type RoomRule,
  type Settings,
} from "./api";
import { Avatar } from "./components/Avatar";
import { ChatView, type Flash, type Pulse } from "./components/ChatView";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { Dialog } from "./components/Dialog";
import { GroupDrawer } from "./components/GroupDrawer";
import { Icon } from "./components/Icon";
import { ProfilePanel } from "./components/ProfileDialog";
import { RoomList } from "./components/RoomList";
import { RuleDialog } from "./components/RuleDialog";
import { SearchPanel } from "./components/SearchPanel";
import { SettingsPanel } from "./components/SettingsPage";
import { compressForChat } from "./image";
import { displayName } from "./names";
import { playHeartsteel } from "./sound";
import "./app.css";

const PAGE = 100;
/// 跳到一条旧消息时最多往前翻多少页。
const MAX_JUMP_PAGES = 60;

type DialogKind = "create" | "join" | "rename";
type ConfirmKind = "leave" | "dissolve" | "cancelJoin" | "clearHistory";
type Panel = "drawer" | "search" | null;

function App() {
  const [me, setMe] = useState<Identity | null>(null);
  const [rooms, setRooms] = useState<Room[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [hasOlder, setHasOlder] = useState(false);
  const [members, setMembers] = useState<Member[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [relays, setRelays] = useState<RelayHealth[] | null | undefined>(undefined);
  const [network, setNetwork] = useState<NetworkStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [left, setLeft] = useState<"chats" | "settings" | "profile">("chats");
  const [dialog, setDialog] = useState<DialogKind | null>(null);
  const [confirmKind, setConfirmKind] = useState<ConfirmKind | null>(null);
  const [ruleEdit, setRuleEdit] = useState<RoomRule | null>(null);
  const [panel, setPanel] = useState<Panel>(null);
  const [pulse, setPulse] = useState<Pulse | null>(null);
  const [flash, setFlash] = useState<Flash | null>(null);
  const [highlight, setHighlight] = useState<{ eventId: string; key: number } | null>(null);
  const [boardKey, setBoardKey] = useState(0);
  // 事件回调里要读最新值，用 ref 存一份
  const activeRef = useRef<string | null>(null);
  const roomsRef = useRef<Room[]>([]);
  const settingsRef = useRef<Settings | null>(null);
  const membersRef = useRef<Member[]>([]);
  const messagesRef = useRef<Message[]>([]);
  roomsRef.current = rooms;
  settingsRef.current = settings;
  membersRef.current = members;
  messagesRef.current = messages;

  const fail = useCallback((e: unknown) => setError(e instanceof Error ? e.message : String(e)), []);
  // 补齐时会一下子来很多条：标记已读和刷新房间列表攒一下，300 毫秒内只做一次
  const readTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const loadRooms = useCallback(() => api.rooms().then(setRooms).catch(fail), [fail]);
  const loadMembers = useCallback(
    (roomId: string) =>
      api
        .members(roomId)
        .then((list) => {
          if (activeRef.current === roomId) setMembers(list);
        })
        .catch(fail),
    [fail],
  );

  const openRoom = useCallback(
    async (roomId: string) => {
      activeRef.current = roomId;
      setActive(roomId);
      setLeft("chats");
      api.setActiveRoom(roomId).catch(fail);
      setMessages([]);
      setMembers([]);
      setPanel(null);
      setHighlight(null);
      try {
        const [list, people] = await Promise.all([api.messages(roomId, null, PAGE), api.members(roomId)]);
        if (activeRef.current !== roomId) return;
        setMessages(list);
        setHasOlder(list.length >= PAGE);
        setMembers(people);
        await api.markRead(roomId);
        await loadRooms();
      } catch (e) {
        fail(e);
      }
    },
    [loadRooms, fail],
  );

  useEffect(() => {
    api.identity().then(setMe).catch(fail);
    api.settings().then(setSettings).catch(fail);
    loadRooms();
    const off = api.onEvent((e: HsEvent) => {
      const current = activeRef.current;
      if (e.type === "live") {
        const s = settingsRef.current;
        const room = roomsRef.current.find((r) => r.roomId === e.roomId);
        if (s?.soundEnabled && !room?.muted) playHeartsteel(s.volume);
        if (e.roomId === current) {
          const key = Date.now();
          setPulse({ memberId: e.memberId, key });
          setFlash({ text: `${displayName(e.memberId, membersRef.current)} 叠到 ${e.newStacks} 层心之钢`, key });
        }
      }
      if (e.roomId === current) {
        if (e.type === "delivered") {
          const ids = new Set(e.eventIds);
          setMessages((list) => list.map((m) => (ids.has(m.eventId) ? { ...m, delivery: "sent" } : m)));
        }
        if (e.type === "imageReady") {
          setMessages((list) =>
            list.map((m) => (m.eventId === e.eventId && m.image ? { ...m, image: { ...m.image, ready: true } } : m)),
          );
        }
        if (e.type === "timeline") {
          setMessages((list) => mergeMessages(list, [e.message]));
          if (!readTimer.current) {
            const roomId = e.roomId;
            readTimer.current = setTimeout(() => {
              readTimer.current = null;
              api.markRead(roomId).then(loadRooms).catch(fail);
            }, 300);
          }
        }
        if (e.type === "stacks" || e.type === "room" || e.type === "status" || e.type === "live") {
          loadMembers(e.roomId);
          setBoardKey((k) => k + 1);
        }
      }
      if (e.type === "mismatch") setError("有群友的版本和你不同，请双方更新到同一版本");
      if ((e.type !== "timeline" && e.type !== "delivered" && e.type !== "imageReady") || e.roomId !== current) {
        loadRooms();
      }
    });
    return () => {
      off.then((f) => f());
    };
  }, [loadRooms, loadMembers, fail]);

  // 主题：跟随系统时去掉 data-theme，由 CSS 的 prefers-color-scheme 决定
  useEffect(() => {
    const root = document.documentElement;
    if (!settings || settings.theme === "system") delete root.dataset.theme;
    else root.dataset.theme = settings.theme;
  }, [settings]);

  // 成员路径、中继和局域网状态没有推送，定时刷新
  useEffect(() => {
    const refresh = () => {
      api.relayStatus().then(setRelays).catch(fail);
      api.networkStatus().then(setNetwork).catch(fail);
      if (active) loadMembers(active);
    };
    refresh();
    const t = setInterval(refresh, 5000);
    return () => clearInterval(t);
  }, [active, loadMembers, fail]);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 2500);
    return () => clearTimeout(t);
  }, [toast]);
  useEffect(() => {
    if (!pulse) return;
    const t = setTimeout(() => setPulse(null), 1200);
    return () => clearTimeout(t);
  }, [pulse]);
  useEffect(() => {
    if (!flash) return;
    const t = setTimeout(() => setFlash(null), 1800);
    return () => clearTimeout(t);
  }, [flash]);
  useEffect(() => {
    if (!error) return;
    const t = setTimeout(() => setError(null), 8000);
    return () => clearTimeout(t);
  }, [error]);

  const room = rooms.find((r) => r.roomId === active) ?? null;

  async function copyCode() {
    if (!active) return;
    try {
      await navigator.clipboard.writeText(await api.roomCode(active));
      setToast("房间码已复制，发给群友就能进群");
    } catch (e) {
      fail(e);
    }
  }

  async function confirmDialog(kind: DialogKind, value: string) {
    if (kind === "create") {
      const created = await api.createRoom(value.trim());
      await loadRooms();
      await openRoom(created.room.roomId);
      await navigator.clipboard.writeText(created.code).catch(() => {});
      setToast("建好了，房间码已复制，发给群友就能进群");
    } else if (kind === "join") {
      const joined = await api.joinRoom(value.trim());
      await loadRooms();
      await openRoom(joined.roomId);
    } else {
      if (!active) return;
      await api.renameRoom(active, value);
      await afterManage(active);
    }
    setDialog(null);
  }

  /// 群管理操作之后：刷新房间列表、当前房间的消息和成员、群资料。
  /// 不用 openRoom：那会关掉群资料，改完群名或规则还要接着看。
  async function afterManage(roomId: string) {
    await loadRooms();
    if (activeRef.current === roomId) {
      const [list, people] = await Promise.all([api.messages(roomId, null, PAGE), api.members(roomId)]);
      setMessages((old) => mergeMessages(old, list));
      setMembers(people);
    }
    setBoardKey((k) => k + 1);
  }

  async function confirmManage(kind: ConfirmKind) {
    if (!active) return;
    const roomId = active;
    if (kind === "leave") {
      await api.leaveRoom(roomId);
      await afterManage(roomId);
    } else if (kind === "dissolve") {
      await api.dissolveRoom(roomId);
      await afterManage(roomId);
    } else if (kind === "clearHistory") {
      await api.clearHistory(roomId);
      setMessages([]);
      setHasOlder(false);
      await loadRooms();
      setToast("本机的聊天记录已清空");
    } else {
      await api.cancelJoin(roomId);
      activeRef.current = null;
      setActive(null);
      setPanel(null);
      api.setActiveRoom(null).catch(fail);
      await loadRooms();
    }
    setConfirmKind(null);
  }

  async function send(text: string, replyTo: string | null, mentions: string[]): Promise<boolean> {
    if (!active) return false;
    try {
      const sent = await api.send(active, text, replyTo, mentions);
      setMessages((list) => mergeMessages(list, [sent]));
      loadMembers(active);
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  }

  async function sendImage(file: Blob): Promise<boolean> {
    if (!active) return false;
    const roomId = active;
    try {
      const c = await compressForChat(file);
      const sent = await api.sendImage(roomId, c.data, c.width, c.height);
      if (activeRef.current === roomId) setMessages((list) => mergeMessages(list, [sent]));
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  }

  async function loadOlder() {
    const roomId = activeRef.current;
    const first = messagesRef.current[0];
    if (!roomId || !first) return;
    try {
      const older = await api.messages(roomId, { displayTs: first.displayTs, eventId: first.eventId }, PAGE);
      if (activeRef.current !== roomId) return;
      setHasOlder(older.length >= PAGE);
      setMessages((list) => mergeMessages(list, older));
    } catch (e) {
      fail(e);
    }
  }

  /// 跳到一条消息：不在已加载的范围里就往前翻页，直到找到为止。
  async function jumpTo(eventId: string) {
    const roomId = activeRef.current;
    if (!roomId) return;
    let list = messagesRef.current;
    let more = hasOlder;
    try {
      for (let page = 0; !list.some((m) => m.eventId === eventId) && more && page < MAX_JUMP_PAGES; page++) {
        const first = list[0];
        if (!first) break;
        const older = await api.messages(roomId, { displayTs: first.displayTs, eventId: first.eventId }, PAGE);
        if (activeRef.current !== roomId) return;
        more = older.length >= PAGE;
        list = mergeMessages(list, older);
      }
    } catch (e) {
      fail(e);
      return;
    }
    setMessages(list);
    setHasOlder(more);
    if (list.some((m) => m.eventId === eventId)) setHighlight({ eventId, key: Date.now() });
    else setToast("原消息不在本机（可能早于同步范围或已清空）");
  }

  async function setPrefs(prefs: { muted?: boolean; pinned?: boolean }) {
    if (!active) return;
    try {
      await api.setRoomPrefs(active, prefs);
      await loadRooms();
    } catch (e) {
      fail(e);
    }
  }

  async function updateSettings(patch: Partial<Omit<Settings, "defaultRelays">>) {
    setSettings(await api.updateSettings(patch));
    if (patch.internetAllowed !== undefined || patch.relays !== undefined) {
      setRelays(await api.relayStatus());
    }
  }

  const dialogProps = {
    create: { title: "建一个群", confirm: "建群", placeholder: "群名称，比如：周五开黑", maxLength: 64 },
    join: { title: "用房间码进群", confirm: "进群", placeholder: "粘贴群友发来的房间码（HS1- 开头）" },
    rename: { title: "修改群名称", confirm: "保存", placeholder: "最多 64 个字", maxLength: 64, initial: room?.title ?? "" },
  };
  const roomTitle = room?.title ?? "这个群";
  const confirmProps: Record<ConfirmKind, { title: string; message: string; confirm: string; danger?: boolean }> = {
    leave: {
      title: "退出群聊",
      message: `退出「${roomTitle}」后不再收发这个群的消息。聊天记录保留在本机只读，你的心之钢层数保留，以后用房间码可以重新进群。`,
      confirm: "退出",
      danger: true,
    },
    dissolve: {
      title: "解散群聊",
      message: `解散「${roomTitle}」后所有人都不能再发言，也不能撤销。聊天记录各自保留只读。暂时不在线的群友下次遇到知道的人时才会得知。`,
      confirm: "解散",
      danger: true,
    },
    cancelJoin: {
      title: "取消加入",
      message: `不再等待加入「${roomTitle}」。以后想进还可以再用房间码进群。`,
      confirm: "取消加入",
    },
    clearHistory: {
      title: "清空聊天记录",
      message: `清空后，这台电脑上不再显示「${roomTitle}」现在的聊天记录。只影响你自己：群友那里的记录不变，心之钢层数不变，之后的新消息照常显示。`,
      confirm: "清空",
      danger: true,
    },
  };

  const anyUnread = rooms.some((r) => r.unread > 0 && (!r.muted || r.mentioned));

  return (
    <div className="app">
      <nav className="rail" aria-label="主导航">
        <button
          className={left === "chats" ? "rail-item current" : "rail-item"}
          onClick={() => setLeft("chats")}
          title="聊天"
          aria-label="聊天"
          aria-current={left === "chats" ? "page" : undefined}
        >
          <Icon name="chat" size={23} />
          {anyUnread && <span className="rail-dot" />}
        </button>
        <span className="rail-spacer" />
        <button
          className={left === "settings" ? "rail-item current" : "rail-item"}
          onClick={() => setLeft("settings")}
          title="设置"
          aria-label="设置"
          aria-current={left === "settings" ? "page" : undefined}
        >
          <Icon name="gear" size={23} />
        </button>
        <button
          className={left === "profile" ? "rail-me current" : "rail-me"}
          onClick={() => setLeft("profile")}
          title="个人资料"
          aria-label="个人资料"
        >
          {me ? (
            <Avatar fingerprint={me.fingerprint} name={me.nickname} avatar={me.avatar} size={30} />
          ) : (
            <span className="avatar" style={{ width: 30, height: 30 }} />
          )}
        </button>
      </nav>

      <aside className="sidebar">
        {left === "chats" && (
          <RoomList
            rooms={rooms}
            active={active}
            onOpen={openRoom}
            onCreate={() => setDialog("create")}
            onJoin={() => setDialog("join")}
          />
        )}
        {left === "settings" && settings && (
          <SettingsPanel
            me={me}
            settings={settings}
            relays={relays}
            network={network}
            onUpdate={updateSettings}
            onPreview={playHeartsteel}
            onOpenProfile={() => setLeft("profile")}
          />
        )}
        {left === "profile" && me && (
          <ProfilePanel
            me={me}
            onSaved={(m) => {
              setMe(m);
              if (active) loadMembers(active);
            }}
            onToast={setToast}
            onError={fail}
          />
        )}
      </aside>

      <div className="main">
        {network?.lanDegraded && (
          <div className="banner warn">局域网端口被其他软件占用，现在只能经中继联系群友。详情见「设置 → 网络」。</div>
        )}
        {room ? (
          <ChatView
            room={room}
            me={me}
            members={members}
            messages={messages}
            relays={relays}
            canLoadOlder={hasOlder}
            pulse={pulse}
            flash={flash}
            highlight={highlight}
            onLoadOlder={loadOlder}
            onSend={send}
            onSendImage={sendImage}
            onJump={jumpTo}
            onOpenDrawer={() => setPanel((p) => (p === "drawer" ? null : "drawer"))}
            onOpenSearch={() => setPanel((p) => (p === "search" ? null : "search"))}
            onCancelJoin={() => setConfirmKind("cancelJoin")}
            onToast={setToast}
          />
        ) : (
          <div className="welcome">
            <img src={heartsteel} alt="" width={120} height={120} />
            <h2>钢一下</h2>
            <p>不用服务器的开黑群聊。在群里 5 秒内连发 3 条消息，叠一层心之钢。</p>
            <div className="welcome-actions">
              <button className="primary" onClick={() => setDialog("create")}>
                建一个群
              </button>
              <button className="outline" onClick={() => setDialog("join")}>
                用房间码进群
              </button>
            </div>
            <p className="welcome-foot">
              <Icon name="lock" size={14} />
              消息全程加密，只有拿到房间码的人能看到
            </p>
          </div>
        )}
      </div>

      {room && panel === "drawer" && (
        <GroupDrawer
          room={room}
          me={me}
          members={members}
          refreshKey={boardKey}
          pulse={pulse}
          onClose={() => setPanel(null)}
          onCopyCode={copyCode}
          onSearch={() => setPanel("search")}
          onPrefs={setPrefs}
          onRename={() => setDialog("rename")}
          onEditRule={setRuleEdit}
          onClearHistory={() => setConfirmKind("clearHistory")}
          onLeave={() => setConfirmKind("leave")}
          onDissolve={() => setConfirmKind("dissolve")}
          onError={fail}
        />
      )}
      {room && panel === "search" && (
        <SearchPanel room={room} members={members} onJump={jumpTo} onClose={() => setPanel(null)} />
      )}

      {error && (
        <button className="banner error floating" onClick={() => setError(null)} title="点击关闭">
          {error}
        </button>
      )}
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
      {confirmKind && (
        <ConfirmDialog
          key={confirmKind}
          {...confirmProps[confirmKind]}
          onConfirm={() => confirmManage(confirmKind)}
          onCancel={() => setConfirmKind(null)}
        />
      )}
      {ruleEdit && active && (
        <RuleDialog
          rule={ruleEdit}
          onConfirm={async (r) => {
            await api.setRoomRule(active, r);
            setRuleEdit(null);
            await afterManage(active);
          }}
          onCancel={() => setRuleEdit(null)}
        />
      )}
      {dialog && (
        <Dialog
          key={dialog}
          {...dialogProps[dialog]}
          onConfirm={(v) => confirmDialog(dialog, v)}
          onCancel={() => setDialog(null)}
        />
      )}
    </div>
  );
}

export default App;
