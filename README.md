# 心之钢群聊

以英雄联盟「心之钢」为主题的去中心化群聊：局域网直连加公共中继，不需要自建服务器；连发消息叠心之钢层数。Windows 桌面应用，Tauri 2（Rust 核心 + React 界面）。

## 文档

- [需求](docs/requirements.md)
- [架构](docs/architecture.md)
- [模块接口](docs/module-interfaces.md)
- 协作规则：[AGENTS.md](AGENTS.md)

## 开发

```bash
pnpm install
pnpm tauri dev
cd src-tauri && cargo test --lib
```

第一次运行时 Windows 防火墙会询问是否允许联网，请勾选「专用网络」并允许，否则收不到局域网里其他人的消息。

### 单机双开（开发用）

一台电脑上开两个实例互相测试：先用 `pnpm tauri dev` 开第一个，再在另一个 PowerShell 窗口里运行：

```powershell
$env:LOLGANG_INSTANCE = '2'; & '.\src-tauri\target\debug\lolgang.exe'
```

第二个实例使用独立的数据目录、凭据条目和 UDP 端口（47481）。开发版的广播同时发往两个端口，所以两个实例能互相发现。
