// 「关于」页：版本、lolPing 致谢和许可、Riot 素材声明（需求 4.7）。
import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import notices from "../../THIRD_PARTY_NOTICES.md?raw";

export function AboutPage() {
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion, () => setVersion(""));
  }, []);

  return (
    <div className="page">
      <h1>关于</h1>
      <section className="card prose">
        <h2>钢一下 {version}</h2>
        <p>和开黑的朋友在桌面上互发英雄联盟信号和表情、说话、投票，开着对线模式还能叠心之钢。免费，不需要服务器。</p>
        <p className="muted">
          正式支持 Windows。macOS 和 Linux 是实验版，没有在真机上验证过：macOS 要在「系统设置 → 隐私与安全性 → 辅助功能」里允许钢一下；
          Linux 只支持 X11 桌面。
        </p>
      </section>
      <section className="card prose">
        <h2>致谢</h2>
        <p>
          信号轮盘、信号动画、表情、队伍面板、投票、键鼠手势判定，以及 macOS、Linux 的键鼠监听，移植自 lolPing（作者 Danrui Meng，MIT 许可）。
        </p>
        <details>
          <summary>第三方声明全文</summary>
          <pre className="notices">{notices}</pre>
        </details>
      </section>
      <section className="card prose">
        <h2>素材声明</h2>
        <p>
          轮盘和信号的图标、音效、表情来自英雄联盟，版权归 Riot Games 所有。钢一下不代表 Riot Games，也没有得到 Riot Games
          的认可。如 Riot Games 提出要求，将删除这些素材。
        </p>
      </section>
    </div>
  );
}
