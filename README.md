# 钢一下

常驻桌面的英雄联盟小玩具：按住 Alt 拖动，在屏幕任何地方发英雄联盟的信号；按住 Ctrl 拖动放表情。和朋友组队后，你发的信号、表情、文字会出现在队友屏幕上同样的位置。开了「对线模式」，队友的鼠标会变成你屏幕上的小英雄，贴近他、充能、出手，就能叠一层心之钢。

不需要服务器：同一个局域网里直接连，不在一起时经公共中继转发，内容加密。Tauri 2（Rust 核心 + React 控制面板）。

## 平台

| 系统 | 状态 |
|---|---|
| Windows 10、11 | 正式支持 |
| macOS 13 以上（Apple 芯片） | 实验版，没在真机上测试过。要在「系统设置 → 隐私与安全性 → 辅助功能」里允许钢一下 |
| Linux（X11 桌面） | 实验版，没在真机上测试过。要有窗口合成器才能透明；Wayland 会话拿不到全局键鼠，不支持 |

## 怎么用

| 做什么 | 默认按键 |
|---|---|
| 信号轮盘：按住拖动，松开在中心放那一格的信号 | Alt + 左键拖动 |
| 表情轮盘 | Ctrl + 左键拖动 |
| 暂停或恢复全部拦截 | Ctrl + Alt + P |
| 对线模式开关 | Ctrl + Alt + L |
| 对队友说一句话（也能输入 /ff 投降、/noff 反对投降、/ready 准备确认） | Alt + T |
| 按住显示队伍面板（队员、在线情况、心之钢层数、投票） | Tab |

触发键、快捷键、轮盘里放什么都能在控制面板里改。只有这些手势会被拦下，别的点击和按键照常交给下面的程序；全屏游戏占着屏幕时，触发键加点击原样交给游戏。文字快捷键和面板键只在队伍里才拦。

- **组队**：「队伍」页建一个队伍，把队伍码（GT2 开头）发给朋友，最多 16 人。改昵称、头像也在这一页。
- **表情**：44 个英雄联盟表情，也能导入自己的图（最多 24 个，可以是动图、带音效）。队友没有这张图时会自动传过去。
- **对线模式**：开着的人在别人屏幕上是一个跟着鼠标走的小英雄。贴近对手 3 秒充能，再按住信号触发键单击他，就叠一层心之钢；同一个人 30 秒内只能叠一次。「队伍」页有排行榜。
- **投票**：投降要 70% 同意，准备确认要所有人接受，踢人要过半。被踢的人的程序会自己退出队伍。
- **功能页**：开机启动，队伍面板、文字、投票各自的开关和按键。

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

`pnpm release` 先运行 `pnpm tauri build` 生成 Windows 的 NSIS 安装包，再把它改名为 `lolgang_<版本>_x64-setup.exe`（在 `src-tauri/target/release/bundle/nsis/`）。macOS 和 Linux 的安装包用 GitHub Actions 的「打包」工作流手动打。版本号在 `src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`package.json` 三处，发版时一起改。

### 单机双开（开发用）

一台电脑上开两个实例互相测试：先用 `pnpm tauri dev` 开第一个，再在另一个 PowerShell 窗口里运行：

```powershell
$env:LOLGANG_INSTANCE = '2'; & '.\src-tauri\target\debug\lolgang.exe'
```

第二个实例使用独立的数据目录、凭据条目和 UDP 端口（47481）。开发版的广播同时发往两个端口，所以两个实例能互相发现。两个实例都装了键盘钩子，后启动的那个先收到按键。

## 致谢和素材

信号轮盘、信号动画、表情、队伍面板、投票、键鼠手势判定，以及 macOS、Linux 的键鼠监听，移植自 [lolPing](https://github.com/darrenprx/lolPing)（作者 Danrui Meng，MIT 许可），移植来的文件开头注明了来源。

图标、音效、表情来自英雄联盟，版权归 Riot Games 所有，不在 MIT 许可范围内。钢一下不代表 Riot Games，也没有得到 Riot Games 的认可。
