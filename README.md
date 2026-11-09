# lolGang（钢一下）

常驻桌面的英雄联盟小玩具：按住 Alt 拖动，在屏幕任何地方发英雄联盟的信号；按住 Ctrl 拖动放表情。和朋友组队后，你发的信号、表情、文字会出现在队友屏幕上同样的位置，按住 V 说话队友直接听到。

不需要服务器：同一个局域网里直接连，不在一起时经公共中继转发，内容加密。Tauri 2（Rust 核心 + React 控制面板）。

## 平台

| 系统 | 状态 |
|---|---|
| Windows 10、11 | 正式支持 |
| macOS 13 以上（Apple 芯片） | 实验版，没在真机上测试过。要在「系统设置 → 隐私与安全性 → 辅助功能」里允许 lolGang |
| Linux（X11 桌面） | 实验版，没在真机上测试过。要有窗口合成器才能透明；Wayland 会话拿不到全局键鼠，不支持 |

## 怎么用

| 做什么 | 默认按键 |
|---|---|
| 信号轮盘：按住拖动，松开在中心放那一格的信号 | Alt + 左键拖动 |
| 表情轮盘 | Ctrl + 左键拖动 |
| 暂停或恢复全部拦截 | Ctrl + Alt + P |
| 对队友说一句话（也能输入 /ff 投降、/noff 反对投降、/ready 准备确认） | Alt + T |
| 按住说话，松开发给队友 | V |
| 按住显示队伍面板（队员、在线情况、投票） | Tab |

所有按键都在控制面板的「快捷键」页改。只有这些手势会被拦下，别的点击和按键照常交给下面的程序；全屏游戏占着屏幕时，触发键加点击原样交给游戏。文字快捷键、说话键和面板键只在队伍里才拦。

- **组队**：「队伍」页建一个队伍，把队伍码（GT2 开头）发给朋友，最多 16 人。改昵称、头像也在这一页。
- **表情**：44 个英雄联盟表情，也能导入自己的图（最多 24 个，可以是动图、带音效）。队友没有这张图时会自动传过去。
- **语音**：在队伍里按住说话键（默认 V）说话，松开发给队友，队友那边直接播放。点一下 V 照常打出字母；按下后马上又按了别的键也算打字。说了 0.3 秒以后可以照常按别的键（边说话边放技能）。一次最长 30 秒。「语音」页可以换麦克风、调队友语音的音量、录 3 秒试音。
- **投票**：投降要 70% 同意，准备确认要所有人接受，踢人要过半。被踢的人的程序会自己退出队伍。
- **功能页**：开机启动、主题，队伍面板、文字、投票的开关（按键都在「快捷键」页）。

点控制面板的关闭按钮只会收进托盘；要退出，右键托盘图标选「退出」。

## 网络和隐私

- 局域网：UDP 端口 47480 广播。第一次运行时 Windows 防火墙会询问，请勾选「专用网络」并允许，否则同一局域网的队友找不到你。
- 互联网：不在同一个局域网时经公共 Nostr 中继转发（默认列表在 `src-tauri/src/settings2.rs`）。所有内容都用队伍密钥加密，中继只看得到密文。「队伍」页可以关掉「允许互联网」，只用局域网。
- 不连别的服务器，不收集任何数据。身份密钥存在系统的凭据管理（Windows 凭据管理器、macOS 钥匙串、Linux Secret Service）。

## 文档

- 2.0：[需求](docs/v2/requirements.md)、[架构](docs/v2/architecture.md)
- 1.x（聊天版，已停止）：[需求](docs/requirements.md)、[架构](docs/architecture.md)、[模块接口](docs/module-interfaces.md)、[验收对照表](docs/acceptance.md)
- 协作规则：[AGENTS.md](AGENTS.md)
- 第三方声明：[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)

## 开发

```bash
pnpm install
pnpm tauri dev
cd src-tauri && cargo test --lib
pnpm release
```

`pnpm release` 先运行 `pnpm tauri build` 生成 Windows 的 NSIS 安装包（本机打的包没有更新签名，只用来自己试），再把它改名为 `lolgang_<版本>_x64-setup.exe`（在 `src-tauri/target/release/bundle/nsis/`）。macOS 和 Linux 的安装包用 GitHub Actions 的「打包」工作流手动打；正式发版见下面。

### 发版和自动更新

已安装的程序启动 30 秒后、之后每 6 小时查一次 `https://github.com/lijianqiao/lolGang/releases/latest/download/latest.json`，查到新版本只提示，用户在「关于」页点「更新并重启」才下载。更新包用 Tauri 的更新签名校验，签名不对就不装。

第一次准备（只做一次）：

1. 在自己电脑上生成更新签名密钥，按提示设一个密码：`pnpm tauri signer generate -w "$HOME/.tauri/lolgang.key"`。会得到私钥 `lolgang.key` 和公钥 `lolgang.key.pub`。私钥和密码只放在你自己电脑和 GitHub Secrets 里，不进代码库、不发给任何人；丢了以后，已安装的程序就再也收不到更新。
2. 把 `lolgang.key.pub` 的内容填进 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`（公钥可以公开）。没填时发版工作流会直接失败。
3. GitHub 仓库 → Settings → Secrets and variables → Actions，新建两个 Secret：`TAURI_SIGNING_PRIVATE_KEY`（私钥文件的全部内容）和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（第 1 步设的密码）。

每次发版：

1. 改版本号：`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`package.json` 三处一致，提交并推送。
2. 打带说明的标签并推送：`git tag -a v2.0.0 -m "这一版改了什么"`，`git push origin v2.0.0`。
3. 「发版」工作流在三个平台打包、签名，全部成功后建 Release，上传安装包和 `latest.json`。标签的说明就是更新说明。

### 单机双开（开发用）

一台电脑上开两个实例互相测试：先用 `pnpm tauri dev` 开第一个，再在另一个 PowerShell 窗口里运行：

```powershell
$env:LOLGANG_INSTANCE = '2'; & '.\src-tauri\target\debug\lolgang.exe'
```

第二个实例使用独立的数据目录、凭据条目和 UDP 端口（47481）。开发版的广播同时发往两个端口，所以两个实例能互相发现。两个实例都装了键盘钩子，后启动的那个先收到按键。

## 致谢和素材

信号轮盘、信号动画、表情、队伍面板、投票、键鼠手势判定，以及 macOS、Linux 的键鼠监听，移植自 [lolPing](https://github.com/darrenprx/lolPing)（作者 Danrui Meng，MIT 许可），移植来的文件开头注明了来源。

图标、音效、表情来自英雄联盟，版权归 Riot Games 所有，不在 MIT 许可范围内。lolGang 不代表 Riot Games，也没有得到 Riot Games 的认可。
