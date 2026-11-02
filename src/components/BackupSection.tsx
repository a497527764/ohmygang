// 设置页里的「备份与恢复」（需求 3.1、架构 8.3）：导出到「文档\心之钢群聊备份」，从文件导入。
// 口令必填；本机已有房间时导入会清空它们，需要再确认一次。导入成功后整页重新加载，换成恢复的身份。
import { useState } from "react";
import { api } from "../api";

const MIN = 6;
const pad = (n: number) => String(n).padStart(2, "0");

function stamp(): string {
  const d = new Date();
  return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
}

async function fileToBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

export function BackupSection() {
  const [kind, setKind] = useState<"identity" | "full">("full");
  const [pass, setPass] = useState("");
  const [pass2, setPass2] = useState("");
  const [saved, setSaved] = useState<string | null>(null);
  const [file, setFile] = useState<File | null>(null);
  const [importPass, setImportPass] = useState("");
  const [confirmOverwrite, setConfirmOverwrite] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const exportProblem =
    pass.length < MIN ? `口令至少 ${MIN} 个字符` : pass !== pass2 ? "两次输入的口令不一样" : null;

  async function doExport() {
    setError(null);
    setSaved(null);
    setBusy(true);
    try {
      setSaved(await api.exportBackup(kind, pass, stamp()));
      setPass("");
      setPass2("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function doImport(overwrite: boolean) {
    if (!file) return;
    setError(null);
    setBusy(true);
    try {
      await api.importBackup(await fileToBase64(file), importPass, overwrite);
      window.location.reload();
    } catch (e) {
      const msg = String(e);
      if (msg.includes("确认覆盖")) setConfirmOverwrite(true);
      else setError(msg);
      setBusy(false);
    }
  }

  return (
    <section className="settings-section">
      <h3>备份与恢复</h3>
      <p className="settings-note">
        换电脑或重装系统前先备份。备份文件用口令加密，忘了口令就打不开，请记牢。同一个身份同一时间只在一台电脑上用。
      </p>

      <div className="backup-block">
        <div className="radio-row" role="radiogroup" aria-label="备份内容">
          <label>
            <input type="radio" checked={kind === "full"} onChange={() => setKind("full")} />
            身份和全部群（含聊天记录）
          </label>
          <label>
            <input type="radio" checked={kind === "identity"} onChange={() => setKind("identity")} />
            只备份身份
          </label>
        </div>
        <div className="pass-row">
          <input type="password" value={pass} onChange={(e) => setPass(e.currentTarget.value)} placeholder="设置备份口令" aria-label="备份口令" />
          <input type="password" value={pass2} onChange={(e) => setPass2(e.currentTarget.value)} placeholder="再输一次" aria-label="再输一次口令" />
          <button className="soft" disabled={busy || !!exportProblem} onClick={doExport} title={exportProblem ?? undefined}>
            导出备份
          </button>
        </div>
        {pass.length > 0 && exportProblem && <p className="settings-note">{exportProblem}</p>}
        {saved && <p className="settings-ok">已保存到 {saved}</p>}
      </div>

      <div className="backup-block">
        <div className="pass-row">
          <input
            type="file"
            accept=".hsbk"
            aria-label="选择备份文件"
            onChange={(e) => {
              setFile(e.currentTarget.files?.[0] ?? null);
              setConfirmOverwrite(false);
            }}
          />
        </div>
        <div className="pass-row">
          <input type="password" value={importPass} onChange={(e) => setImportPass(e.currentTarget.value)} placeholder="备份口令" aria-label="导入口令" />
          <button className="soft" disabled={busy || !file || !importPass} onClick={() => doImport(false)}>
            从备份恢复
          </button>
        </div>
        {confirmOverwrite && (
          <div className="settings-warn">
            本机已经有身份和群。恢复备份会清空本机现在的身份、群和聊天记录，换成备份里的。恢复后各个群要先和群友同步一次才能发言。
            <div className="warn-actions">
              <button className="ghost" onClick={() => setConfirmOverwrite(false)}>
                取消
              </button>
              <button className="primary" disabled={busy} onClick={() => doImport(true)}>
                清空并恢复
              </button>
            </div>
          </div>
        )}
      </div>
      {error && <p className="field-error">{error}</p>}
    </section>
  );
}
