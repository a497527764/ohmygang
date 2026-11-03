# 钢一下

以英雄联盟「心之钢」为主题的去中心化群聊：局域网直连加公共中继，不需要自建服务器；连发消息叠心之钢层数。Windows 桌面应用，Tauri 2（Rust 核心 + React 界面）。

## 安装和使用

1. 运行安装包 `钢一下_<版本>_x64-setup.exe`（在 `src-tauri/target/release/bundle/nsis/` 里），按当前用户安装，不需要管理员权限。
2. 第一次运行时 Windows 防火墙会询问，请勾选「专用网络」并允许，否则同一局域网的群友找不到你。
3. 点「建房」起个群名，房间码会自动复制，发给群友；群友点「进房」粘贴房间码即可加入。
4. 5 秒内连发 3 条消息叠一层心之钢（规则房主可改）。头像右下角是层数，群资料里有排行榜。
5. 点窗口的关闭按钮只会收进右下角托盘，程序继续收消息；要彻底退出，右键托盘图标选「退出」。
6. 换电脑前在「设置 → 备份与恢复」导出备份，文件在「文档\钢一下备份」。

## 文档

- [需求](docs/requirements.md)
- [架构](docs/architecture.md)
- [模块接口](docs/module-interfaces.md)
- [验收对照表](docs/acceptance.md)
- 协作规则：[AGENTS.md](AGENTS.md)

## 开发

```bash
pnpm install
pnpm tauri dev
cd src-tauri && cargo test --lib
pnpm tauri build
```

`pnpm tauri build` 生成 NSIS 安装包，第一次运行会从 GitHub 下载 NSIS 工具。

第一次运行时 Windows 防火墙会询问是否允许联网，请勾选「专用网络」并允许，否则收不到局域网里其他人的消息。

### 单机双开（开发用）

一台电脑上开两个实例互相测试：先用 `pnpm tauri dev` 开第一个，再在另一个 PowerShell 窗口里运行：

```powershell
$env:LOLGANG_INSTANCE = '2'; & '.\src-tauri\target\debug\lolgang.exe'
```

第二个实例使用独立的数据目录、凭据条目和 UDP 端口（47481）。开发版的广播同时发往两个端口，所以两个实例能互相发现。
