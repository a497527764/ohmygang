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
