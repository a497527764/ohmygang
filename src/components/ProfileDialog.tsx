// 个人资料（V1.1）：点左上角自己的头像打开。改昵称、换头像（内置图标配底色，或上传图片），复制公钥指纹。
// 改完按「保存」才生效：每次保存会在你所在的每个群写一条资料事件同步给群友，所以挑头像时先只在这里预览。
import { useState } from "react";
import { api, type Avatar as AvatarData, type AvatarInput, type Identity } from "../api";
import { PRESET_COLORS, PRESET_ICONS } from "../avatars";
import { compressForAvatar } from "../image";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";

interface Props {
  me: Identity;
  onSaved: (me: Identity) => void;
  onClose: () => void;
  onToast: (text: string) => void;
}

/// 编辑中的头像：没有（自动生成）、内置、图片（data 是 Base64，用来保存）。
type Choice =
  | { kind: "none" }
  | { kind: "preset"; icon: number; color: number }
  | { kind: "image"; dataUrl: string; data: string };

function initialChoice(a: AvatarData | null): Choice {
  if (a?.kind === "preset") return a;
  if (a?.kind === "image") return { kind: "image", dataUrl: a.dataUrl, data: a.dataUrl.split(",")[1] ?? "" };
  return { kind: "none" };
}

function toAvatar(c: Choice): AvatarData | null {
  if (c.kind === "preset") return c;
  if (c.kind === "image") return { kind: "image", dataUrl: c.dataUrl };
  return null;
}

function toInput(c: Choice): AvatarInput | null {
  if (c.kind === "preset") return c;
  if (c.kind === "image") return { kind: "image", data: c.data };
  return null;
}

export function ProfileDialog({ me, onSaved, onClose, onToast }: Props) {
  const [nickname, setNickname] = useState(me.nickname);
  const [choice, setChoice] = useState<Choice>(initialChoice(me.avatar));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const color = choice.kind === "preset" ? choice.color : 4;
  const icon = choice.kind === "preset" ? choice.icon : -1;

  async function upload(file: File | undefined) {
    if (!file) return;
    setError(null);
    try {
      const c = await compressForAvatar(file);
      setChoice({ kind: "image", data: c.data, dataUrl: `data:image/webp;base64,${c.data}` });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function save() {
    setBusy(true);
    setError(null);
    try {
      onSaved(await api.setProfile(nickname.trim(), toInput(choice)));
      onToast("资料已保存，群友会看到新的头像和昵称");
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  async function copyFingerprint() {
    await navigator.clipboard.writeText(me.fingerprint).catch(() => {});
    onToast("公钥指纹已复制");
  }

  return (
    <div className="overlay" onMouseDown={onClose}>
      <div
        className="dialog profile-dialog"
        role="dialog"
        aria-label="个人资料"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
        }}
      >
        <header className="profile-top">
          <div className="profile-avatar">
            <Avatar fingerprint={me.fingerprint} name={nickname} avatar={toAvatar(choice)} size={84} />
            <label className="avatar-upload" title="上传图片做头像">
              <Icon name="camera" size={16} />
              <input type="file" accept="image/*" hidden onChange={(e) => upload(e.currentTarget.files?.[0])} />
            </label>
          </div>
          <div className="profile-fields">
            <label className="field">
              <span>昵称</span>
              <input
                autoFocus
                value={nickname}
                maxLength={32}
                placeholder="群友看到的名字"
                onChange={(e) => setNickname(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") save();
                }}
              />
            </label>
            <div className="fingerprint">
              <span>公钥指纹</span>
              <code>{me.fingerprint}</code>
              <button className="icon-button small" onClick={copyFingerprint} title="复制" aria-label="复制公钥指纹">
                <Icon name="copy" size={14} />
              </button>
            </div>
          </div>
        </header>

        <section className="avatar-picker">
          <div className="picker-head">
            <h3>内置头像</h3>
            <button
              className="link-button"
              disabled={choice.kind === "none"}
              onClick={() => setChoice({ kind: "none" })}
            >
              用默认头像
            </button>
          </div>
          <div className="icon-grid" role="radiogroup" aria-label="头像图标">
            {PRESET_ICONS.map((e, i) => (
              <button
                key={e}
                role="radio"
                aria-checked={i === icon}
                className={i === icon ? "selected" : undefined}
                style={{ background: PRESET_COLORS[color] }}
                onClick={() => setChoice({ kind: "preset", icon: i, color })}
              >
                {e}
              </button>
            ))}
          </div>
          <div className="swatches" role="radiogroup" aria-label="头像底色">
            {PRESET_COLORS.map((c, i) => (
              <button
                key={c}
                role="radio"
                aria-checked={choice.kind === "preset" && i === color}
                aria-label={`底色 ${i + 1}`}
                className={choice.kind === "preset" && i === color ? "selected" : undefined}
                style={{ background: c }}
                onClick={() => setChoice({ kind: "preset", icon: icon >= 0 ? icon : 0, color: i })}
              />
            ))}
          </div>
          <p className="panel-note">
            也可以点头像右下角的相机上传图片，会裁成正方形并压缩到很小，方便同步给群友。昵称可以和别人重名，核对身份请看公钥指纹。
          </p>
        </section>

        {error && <p className="field-error">{error}</p>}
        <div className="dialog-actions">
          <button className="ghost" onClick={onClose}>
            取消
          </button>
          <button className="primary" disabled={busy} onClick={save}>
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
