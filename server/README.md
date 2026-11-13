# OH-MY-GANG! 自有云端服务

一个端口同时承担两件事：

1. **Nostr 风格 WebSocket 中继** —— 客户端把「中继」直接指向这里（`ws://服务器IP:转发端口`），房间内消息走 kind `24810`、标签 `x` = room_id 十六进制，content 为 Base64 包。协议与客户端完全一致，端到端加密不变。
2. **6 位短码 HTTP API** —— 建队时服务器分配 6 位大写字母数字短码（去掉 0/O/1/I 等易混字符）并铸造兼容的 `GT2-` 长码；进队时把短码发回服务器换回长码与 room_id；服务器真正「拥有」并可回收队伍码。

服务每 **10 分钟**扫描一次，自动解散并回收短码：

- 超过 **5 天**没有任何活动（无消息、无心跳）的队伍；
- 连续 **5 天**始终不足 **2 人**的队伍。

回收后房间进入黑名单，短码释放可再次分配。

---

## 一、本地运行（先验证一下）

```bash
cd server
cargo run --release
# 或自定义端口 / 数据库路径
PORT=8080 DATA_DIR=./ohmygang.db cargo run --release
```

健康检查：

```bash
curl http://127.0.0.1:8080/health
# {"ok":true}
```

冒烟测试短码接口：

```bash
# 建队（member_id 为 64 位十六进制，即 32 字节客户端公钥）
curl -s -X POST http://127.0.0.1:8080/api/team/create \
  -H 'content-type: application/json' \
  -d '{"member_id":"ababababababababababababababababababababababababababababababab"}'
# {"short_code":"123456","code":"GT2-...","room_id":"...."}

# 进队（用上面的 short_code）
curl -s -X POST http://127.0.0.1:8080/api/team/join \
  -H 'content-type: application/json' -d '{"code":"123456"}'
# {"code":"GT2-...","room_id":"...."}
```

---

## 二、部署到 Zeabur

两条路都可以：**CLI 直传部署**（当前实际用的，不需要 GitHub）或 **关联 GitHub 部署**。

### 路线 A：CLI 直传部署（当前实际用的）

```bash
cd server
npx zeabur@latest auth login          # 浏览器登录一次
npx zeabur@latest project create --name ohmygang-relay --region <你的独服 region-id>
npx zeabur@latest deploy --create --name ohmygang-relay --project-id <项目ID> -i=false
```

部署完成后配置两件事：

1. **持久化**：挂载 Volume 到 `/data`（服务的 `DATA_DIR` 默认 `/data/ohmygang.db`）。CLI 里用 GraphQL：
   ```bash
   npx zeabur@latest api graphql -f 'query=mutation { mountVolume(serviceID: "<服务ID>", id: "<卷名>", dir: "/data") }'
   ```
   不挂卷的话，每次重新部署队伍码记录都会清空。
2. **端口转发**（IP 直连用）：开启 `portForwardingMode: ENABLED`，然后读 `ports { forwardedPort }` 拿到转发端口：
   ```bash
   npx zeabur@latest api graphql -f 'query=mutation { updatePortForwardingMode(serviceID: "<服务ID>", environmentID: "<环境ID>", mode: ENABLED) }'
   npx zeabur@latest api graphql -f 'query=query { service(_id: "<服务ID>") { portForwardedHost ports(environmentID: "<环境ID>") { port forwardedPort } } }'
   ```
   输出里的 `portForwardedHost`（服务器 IP）+ `forwardedPort` 就是客户端要用的地址，形如 `14.103.234.253:31818`。

> 为什么用 IP 直连而不是域名：Zeabur 国内节点绑公网域名要求账号先过实名认证（部分云商还要求 ICP 备案）。端口转发走 `IP:端口` 无此要求，适合自用/小队场景。代价是流量不走 TLS——不过房间内容本来就是端到端加密的（服务器看不到明文），只有短码接口是明文 JSON，自用可接受。

### 路线 B：关联 GitHub 部署（备选）

1. 确保 `server/` 目录（含 `Cargo.toml`、`Cargo.lock`、`src/`、`Dockerfile`）已进入仓库并提交。
2. 打开 Zeabur 控制台 → **Create Project** → **Deploy from GitHub**，授权并选中仓库。
3. 因为这是一个 monorepo（根目录是 Tauri 桌面应用），新建服务时把 **Root Directory（根目录）** 设为 `server`，Zeabur 会自动找到里面的 `Dockerfile` 构建。
   > 备选：保持根目录不变，用环境变量 `ZBPACK_DOCKERFILE_PATH=server/Dockerfile`。
4. **端口**：服务读环境变量 `PORT`（默认 8080），Zeabur 会自动注入，无需手动改。
5. **持久化 / 端口转发**：同路线 A 的两步配置。

### 构建说明

首次构建约几分钟（编译 Rust + 内置 SQLite）。构建完成后服务会自动启动，健康检查 `GET /health` 返回 `{"ok":true}`。

### 路线 C：更新包托管（国内直连的自有更新源）

GitHub 直连在国内时通时断，第三方加速镜像寿命不可控。服务器在国内可达的 IP 上，
顺便把更新清单和安装包也放这里：客户端 updater 的 endpoints 第一位就是它，连不上再回落 GitHub 和镜像。

安装包有 minisign 端到端签名（公钥编译进客户端），HTTP 明文传输被篡改也过不了校验，所以不走 TLS 也安全。

**启用（两步）**：

1. 在 Zeabur 给服务加环境变量 `UPLOAD_TOKEN=<自定义随机串>`（同时给 GitHub 仓库加 secret `RELAY_UPLOAD_TOKEN` 设同一个值，CI 发版后自动同步）。
2. 发版后同步：CI 配好 secret 会自动跑，但**海外 runner 向国内中继传大包容易超时**——CI 里这一步已设为失败不拦截发版，超时后在国内本机补跑即可：
   ```bash
   # 国内本机加 GH_MIRROR 加速下载 release 产物（实测 86MB 约 1 分钟）
   RELAY_UPLOAD_TOKEN=<同值> GH_MIRROR=https://ghfast.top/ node scripts/release.mjs mirror v0.2.1
   ```

**上传 API**（CI / 手动都会调它）：`POST /api/admin/update?token=<UPLOAD_TOKEN>&name=<文件名>`，body 是原始字节，存到 `/data/updates/`。
**下载**：`GET /update/<文件名>`（含 `GET /update/latest.json`）。文件名白名单：字母数字加 `.-_`，防路径穿越。

---

## 三、客户端需要改的配置

部署好上面这个服务后，把客户端的云端地址指向你的服务器地址。在客户端 `src-tauri/src/cloud.rs` 顶部有两个常量：

```rust
// 你的服务器地址（Zeabur 端口转发出来的 IP:端口，明文直连）
pub const CLOUD_API:   &str = "http://14.103.234.253:31818";
pub const CLOUD_RELAY: &str = "ws://14.103.234.253:31818";
```

- `CLOUD_RELAY` —— 客户端 `RelayPool` 直接连接这里（云端是唯一中继）。
- `CLOUD_API`  —— 客户端调用 `/api/team/create`、`/api/team/join`、`/api/team/heartbeat` 的基地址。

建队 / 进队 / 心跳的交互（见客户端改造部分）：

- **建队**：客户端向 `POST {CLOUD_API}/api/team/create` 提交自己的公钥（member_id），服务器返回 6 位 `short_code` 和铸造好的 `GT2-` 长码与 `room_id`。客户端拿这个长码像往常一样派生 room_key，并**仍然写入 RoomCreate 事件**以便中继里激活房间。
- **进队**：用户输入 6 位字母数字短码 → 客户端向 `POST {CLOUD_API}/api/team/join` 换取 `GT2-` 长码与 `room_id`；若输入的是旧式 `GT2-` 长码则直接透传（兼容存量队伍）。
- **心跳**：每个活跃房间定期（例如每 60 秒）向 `POST {CLOUD_API}/api/team/heartbeat` 上报 `room_id` + 自己的 `member_id`，让服务器能正确统计人数、决定哪些队伍该回收。

---

## 四、HTTP API 参考

所有接口请求/响应均为 JSON（`Content-Type: application/json`）。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/` `/health` | 健康检查，返回 `{"ok":true}` |
| POST | `/api/team/create` | 建队，分配 6 位短码并铸造 GT2 长码 |
| POST | `/api/team/join` | 用 6 位短码换回 GT2 长码与 room_id |
| POST | `/api/team/heartbeat` | 上报房间活跃度与成员，供回收判定 |
| GET | `/api/team/code?room=<room_id_hex>` | 用 room_id 反查当前短码 |

### POST /api/team/create
请求：
```json
{ "member_id": "<64 位十六进制 = 32 字节客户端公钥>" }
```
响应 `200`：
```json
{ "short_code": "123456", "code": "GT2-XXXX-XXXX-...", "room_id": "<32 位十六进制>" }
```
- 限流：同一 IP 每小时最多 30 次建队，超出返回 `429`。
- `member_id` 非法（非 32 字节十六进制）返回 `400`。

### POST /api/team/join
请求：
```json
{ "code": "123456" }
```
响应 `200`：
```json
{ "code": "GT2-XXXX-XXXX-...", "room_id": "<32 位十六进制>" }
```
- 短码不存在或已回收返回 `404`。
- `code` 非 6 位字母数字短码（或旧式 GT2 长码格式）返回 `400`。

### POST /api/team/heartbeat
请求：
```json
{ "room_id": "<32 位十六进制>", "member_id": "<64 位十六进制>" }
```
响应 `200`：`{ "ok": true }`；房间已回收返回 `404`。

---

## 五、回收规则（服务器自动执行）

| 规则 | 条件 | 动作 |
| --- | --- | --- |
| 长期不活跃 | `now - last_active > 5 天` | 解散队伍、回收短码、room_id 进黑名单 |
| 长期人少 | 连续 `5 天` 内活跃成员始终 `< 2 人` | 同上 |

- 回收扫描周期：**每 10 分钟**一次。
- 活跃判定来源：客户端心跳上报（`/api/team/heartbeat`）与中继里的消息活动，汇总到内存登记表并落库。
- 被回收的 room_id 写入 `blacklist` 表，中继会拒绝任何发给该房间的事件，防止旧码复活。

---

## 六、安全与限流

- 建队接口按来源 IP 限流（每小时 30 次），防止有人用脚本把 6 位短码池占满。
- 中继层对 `content` 做 Base64 解码并限制体积（≤ 1200 字节），拒绝畸形包。
- 端到端加密不变：服务器只保管 `room_secret + owner_tag` 到短码的映射，从不接触明文消息内容，也无法解密房间消息。
- `blacklist` 机制确保已回收房间不可被重新激活。

---

## 七、目录结构

```
server/
├── Cargo.toml          # 独立 binary crate：ohmygang-relay
├── Dockerfile          # 多阶段构建（Rust 构建 / debian-slim 运行）
├── .dockerignore
├── README.md           # 本文件
└── src/
    ├── main.rs         # 入口：绑定端口、启动回收任务、集成测试
    ├── net.rs          # 单端口同时分流 WebSocket 中继与 HTTP 短码 API
    ├── relay.rs        # Nostr 风格中继 + 内存成员/活跃登记表
    ├── api.rs          # 短码 create / join / heartbeat / code 接口 + 限流
    ├── store.rs        # SQLite 持久化（teams / members / blacklist）
    └── protocol.rs     # 复刻客户端 GT2 长码与 room_id 派生（逐字节一致）
```
