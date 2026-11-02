// 主界面（M1-5，微信式）：左边窄栏（我的头像、聊天、设置），房间列表，聊天区；群资料是右侧抽屉，设置是弹出页。
// 数据都从 api.ts 取；收到 hs://event 时只刷新受影响的部分。成员路径、中继和局域网状态没有推送，每 5 秒刷新一次。
// 音效在这里决定播不播：本机音效开关、房间免打扰、音量。
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
  type Settings,
} from "./api";
import { Avatar } from "./components/Avatar";
import { ChatView, type Flash, type Pulse } from "./components/ChatView";
import { Dialog } from "./components/Dialog";
import { GroupDrawer } from "./components/GroupDrawer";
import { Icon } from "./components/Icon";
import { RoomList } from "./components/RoomList";
import { SettingsPage } from "./components/SettingsPage";
import { displayName } from "./names";
import { playHeartsteel } from "./sound";
import "./app.css";

const PAGE = 100;

type DialogKind = "create" | "join" | "nickname";

function App() {
  const [me, setMe] = useState<Identity | null>(null);
  const [rooms, setRooms] = useState<Room[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [members, setMembers] = useState<Member[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [relays, setRelays] = useState<RelayHealth[] | null | undefined>(undefined);
  const [network, setNetwork] = useState<NetworkStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [dialog, setDialog] = useState<DialogKind | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pulse, setPulse] = useState<Pulse | null>(null);
  const [flash, setFlash] = useState<Flash | null>(null);
  const [boardKey, setBoardKey] = useState(0);
  // 事件回调里要读最新值，用 ref 存一份
  const activeRef = useRef<string | null>(null);
  const roomsRef = useRef<Room[]>([]);
  const settingsRef = useRef<Settings | null>(null);
  const membersRef = useRef<Member[]>([]);
  roomsRef.current = rooms;
  settingsRef.current = settings;
  membersRef.current = members;

  const fail = useCallback((e: unknown) => setError(String(e)), []);

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
      setMessages([]);
      setMembers([]);
      setDrawerOpen(false);
      try {
        const [list, people] = await Promise.all([api.messages(roomId, null, PAGE), api.members(roomId)]);
        if (activeRef.current !== roomId) return;
        setMessages(list);
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
        if (e.type === "timeline") {
          setMessages((list) => mergeMessages(list, [e.message]));
          api.markRead(e.roomId).then(loadRooms).catch(fail);
        }
        if (e.type === "stacks" || e.type === "room" || e.type === "status" || e.type === "live") {
          loadMembers(e.roomId);
          setBoardKey((k) => k + 1);
        }
      }
      if (e.type === "mismatch") setError("有群友的版本和你不同，请双方更新到同一版本");
      if ((e.type !== "timeline" && e.type !== "delivered") || e.roomId !== current) loadRooms();
    });
    return () => {
      off.then((f) => f());
    };
  }, [loadRooms, loadMembers, fail]);

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

  const room = rooms.find((r) => r.roomId === active) ?? null;

  async function copyCode() {
    if (!active) return;
    try {
      await navigator.clipboard.writeText(await api.roomCode(active));
      setToast("房间码已复制，发给群友即可加入");
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
      setToast("建好了，房间码已复制，发给群友即可加入");
    } else if (kind === "join") {
      const joined = await api.joinRoom(value.trim());
      await loadRooms();
      await openRoom(joined.roomId);
    } else {
      setMe(await api.setNickname(value));
      if (active) loadMembers(active);
    }
    setDialog(null);
  }

  async function send(text: string): Promise<boolean> {
    if (!active) return false;
    try {
      const sent = await api.send(active, text);
      setMessages((list) => mergeMessages(list, [sent]));
      loadMembers(active);
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  }

  async function loadOlder() {
    if (!active || messages.length === 0) return;
    const first = messages[0];
    try {
      const older = await api.messages(active, { displayTs: first.displayTs, eventId: first.eventId }, PAGE);
      setMessages((list) => mergeMessages(list, older));
    } catch (e) {
      fail(e);
    }
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
    create: { title: "建一个群", confirm: "建房", placeholder: "群名称，比如：周五开黑", maxLength: 64 },
    join: { title: "用房间码进房", confirm: "进房", placeholder: "粘贴群友发来的房间码（HS1- 开头）" },
    nickname: { title: "修改昵称", confirm: "保存", placeholder: "最多 32 个字", maxLength: 32, initial: me?.nickname ?? "" },
  };

  return (
    <div className="app">
      <nav className="rail" aria-label="主导航">
        <button className="rail-me" onClick={() => setSettingsOpen(true)} title="我的资料和设置">
          {me ? <Avatar fingerprint={me.fingerprint} name={me.nickname} size={36} /> : <span className="avatar" />}
        </button>
        <button className="rail-item current" title="聊天" aria-current="page">
          <Icon name="chat" size={22} />
        </button>
        <span className="rail-spacer" />
        <button className="rail-item" onClick={() => setSettingsOpen(true)} title="设置" aria-label="设置">
          <Icon name="gear" size={22} />
        </button>
      </nav>

      <RoomList
        rooms={rooms}
        active={active}
        onOpen={openRoom}
        onCreate={() => setDialog("create")}
        onJoin={() => setDialog("join")}
      />

      <div className="main">
        {network?.lanDegraded && (
          <div className="banner warn">局域网端口被其他软件占用，现在只能经中继联系群友。详情见设置。</div>
        )}
        {error && (
          <button className="banner error" onClick={() => setError(null)} title="点击关闭">
            {error}
          </button>
        )}
        {room ? (
          <ChatView
            room={room}
            me={me}
            members={members}
            messages={messages}
            relays={relays}
            canLoadOlder={messages.length >= PAGE}
            pulse={pulse}
            flash={flash}
            onLoadOlder={loadOlder}
            onSend={send}
            onOpenDrawer={() => setDrawerOpen(true)}
          />
        ) : (
          <div className="welcome">
            <img src={heartsteel} alt="" width={96} height={96} />
            <p>在群里 5 秒内连发 3 条消息，就能叠一层心之钢。</p>
            <div className="welcome-actions">
              <button className="primary" onClick={() => setDialog("create")}>
                建房
              </button>
              <button className="soft" onClick={() => setDialog("join")}>
                用房间码进房
              </button>
            </div>
          </div>
        )}
        {toast && (
          <div className="toast" role="status">
            {toast}
          </div>
        )}
      </div>

      {room && drawerOpen && (
        <GroupDrawer
          room={room}
          me={me}
          members={members}
          refreshKey={boardKey}
          pulse={pulse}
          onClose={() => setDrawerOpen(false)}
          onCopyCode={copyCode}
          onPrefs={setPrefs}
          onError={fail}
        />
      )}
      {settingsOpen && settings && (
        <SettingsPage
          me={me}
          settings={settings}
          relays={relays}
          network={network}
          onClose={() => setSettingsOpen(false)}
          onEditNickname={() => setDialog("nickname")}
          onUpdate={updateSettings}
          onPreview={playHeartsteel}
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
