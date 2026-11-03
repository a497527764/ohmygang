// 个人资料（V1.1 第二版，参考 WhatsApp）：在左栏里打开，不挡住聊天。
// 大头像，鼠标移上去点「更换头像」：上传图片、选内置头像、用默认头像；下面是昵称（点铅笔改）和公钥指纹。
// 每次改动会在你所在的每个群写一条资料事件同步给群友，所以内置头像先在弹窗里预览，点「使用」才保存。
import { useEffect, useRef, useState } from "react";
import { api, type Avatar as AvatarData, type AvatarInput, type Identity } from "../api";
import { PRESET_COLORS, PRESET_ICONS } from "../avatars";
import { compressForAvatar } from "../image";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";

interface Props {
  me: Identity;
  onSaved: (me: Identity) => void;
  onToast: (text: string) => void;
  onError: (e: unknown) => void;
}

/// 把当前头像原样转回保存用的格式（只改昵称时头像不变）。
function inputOf(a: AvatarData | null): AvatarInput | null {
  if (a?.kind === "preset") return a;
  if (a?.kind === "image") return { kind: "image", data: a.dataUrl.split(",")[1] ?? "" };
  return null;
}

export function ProfilePanel({ me, onSaved, onToast, onError }: Props) {
  const [menu, setMenu] = useState(false);
  const [picking, setPicking] = useState(false);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(me.nickname);
  const [busy, setBusy] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenu(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  async function save(nickname: string, avatar: AvatarInput | null, done: string) {
    setBusy(true);
    try {
      onSaved(await api.setProfile(nickname.trim(), avatar));
      onToast(done);
      return true;
    } catch (e) {
      onError(e);
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function upload(f: File | undefined) {
    if (!f) return;
    try {
      const c = await compressForAvatar(f);
      await save(me.nickname, { kind: "image", data: c.data }, "头像已更换，群友会看到新头像");
    } catch (e) {
      onError(e);
    }
  }

  async function saveName() {
    if (name.trim() === me.nickname) {
      setEditing(false);
      return;
    }
    if (await save(name, inputOf(me.avatar), "昵称已保存")) setEditing(false);
  }

  return (
    <>
      <header className="side-head">
        <h1>个人资料</h1>
      </header>
      <div className="side-scroll">
        <div className="profile-hero">
          <div className="avatar-stage" ref={menuRef}>
            <Avatar fingerprint={me.fingerprint} name={me.nickname} avatar={me.avatar} size={168} />
            <button className="avatar-change" onClick={() => setMenu((m) => !m)} disabled={busy}>
              <Icon name="camera" size={26} />
              更换头像
            </button>
            {menu && (
              <div className="menu avatar-menu" role="menu">
                <button
                  role="menuitem"
                  onClick={() => {
                    setMenu(false);
                    file.current?.click();
                  }}
                >
                  <Icon name="image" size={18} />
                  上传图片
                </button>
                <button
                  role="menuitem"
                  onClick={() => {
                    setMenu(false);
                    setPicking(true);
                  }}
                >
                  <Icon name="smile" size={18} />
                  选内置头像
                </button>
                {me.avatar && (
                  <button
                    role="menuitem"
                    onClick={() => {
                      setMenu(false);
                      save(me.nickname, null, "已换回默认头像");
                    }}
                  >
                    <Icon name="close" size={18} />
                    用默认头像
                  </button>
                )}
              </div>
            )}
            <input
              ref={file}
              type="file"
              accept="image/*"
              hidden
              onChange={(e) => {
                upload(e.currentTarget.files?.[0]);
                e.currentTarget.value = "";
              }}
            />
          </div>
        </div>

        <div className="profile-field">
          <label htmlFor="profile-name">你的昵称</label>
          {editing ? (
            <div className="field-edit">
              <input
                id="profile-name"
                autoFocus
                value={name}
                maxLength={32}
                onChange={(e) => setName(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") saveName();
                  if (e.key === "Escape") {
                    setName(me.nickname);
                    setEditing(false);
                  }
                }}
              />
              <span className="field-count">{32 - Array.from(name).length}</span>
              <button className="icon-button" onClick={saveName} disabled={busy} aria-label="保存昵称" title="保存">
                <Icon name="check" size={20} />
              </button>
            </div>
          ) : (
            <div className="field-show">
              <span>{me.nickname || "还没有昵称"}</span>
              <button
                className="icon-button"
                onClick={() => {
                  setName(me.nickname);
                  setEditing(true);
                }}
                aria-label="修改昵称"
                title="修改昵称"
              >
                <Icon name="edit" size={18} />
              </button>
            </div>
          )}
          <p>这是群友看到的名字，可以和别人重名。</p>
        </div>

        <div className="profile-field">
          <label>公钥指纹</label>
          <div className="field-show">
            <code>{me.fingerprint}</code>
            <button
              className="icon-button"
              onClick={async () => {
                await navigator.clipboard.writeText(me.fingerprint).catch(() => {});
                onToast("公钥指纹已复制");
              }}
              aria-label="复制公钥指纹"
              title="复制"
            >
              <Icon name="copy" size={18} />
            </button>
          </div>
          <p>每个人独有，核对「是不是本人」时以它为准。</p>
        </div>
      </div>

      {picking && (
        <PresetPicker
          me={me}
          busy={busy}
          onCancel={() => setPicking(false)}
          onPick={async (icon, color) => {
            if (await save(me.nickname, { kind: "preset", icon, color }, "头像已更换，群友会看到新头像")) setPicking(false);
          }}
        />
      )}
    </>
  );
}

interface PickerProps {
  me: Identity;
  busy: boolean;
  onCancel: () => void;
  onPick: (icon: number, color: number) => void;
}

/// 选内置头像：图标和底色，上面实时预览。
function PresetPicker({ me, busy, onCancel, onPick }: PickerProps) {
  const start = me.avatar?.kind === "preset" ? me.avatar : { icon: 0, color: 4 };
  const [icon, setIcon] = useState(start.icon);
  const [color, setColor] = useState(start.color);
  return (
    <div className="overlay" onMouseDown={onCancel}>
      <div
        className="dialog picker-dialog"
        role="dialog"
        aria-label="选内置头像"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
      >
        <h2>选内置头像</h2>
        <div className="picker-preview">
          <Avatar fingerprint={me.fingerprint} name={me.nickname} avatar={{ kind: "preset", icon, color }} size={88} />
        </div>
        <div className="icon-grid" role="radiogroup" aria-label="图标">
          {PRESET_ICONS.map((e, i) => (
            <button
              key={e}
              role="radio"
              aria-checked={i === icon}
              className={i === icon ? "selected" : undefined}
              onClick={() => setIcon(i)}
            >
              {e}
            </button>
          ))}
        </div>
        <div className="swatches" role="radiogroup" aria-label="底色">
          {PRESET_COLORS.map((c, i) => (
            <button
              key={c}
              role="radio"
              aria-checked={i === color}
              aria-label={`底色 ${i + 1}`}
              className={i === color ? "selected" : undefined}
              style={{ background: c }}
              onClick={() => setColor(i)}
            />
          ))}
        </div>
        <div className="dialog-actions">
          <button className="ghost" onClick={onCancel}>
            取消
          </button>
          <button className="primary" disabled={busy} onClick={() => onPick(icon, color)}>
            使用
          </button>
        </div>
      </div>
    </div>
  );
}
