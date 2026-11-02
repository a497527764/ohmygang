# 心之钢群聊 — 模块接口草图

> 版本：修订版（2026-11-02 设计审查后）  
> 对应：[architecture.md](./architecture.md)、[requirements.md](./requirements.md)  
> 性质：接口级草图（示意名），非最终 API 冻结。协议行为以架构文档第 3–8 节为准

---

## 0. 约定

- **Command**：UI → Rust  
- **Event**：Rust → UI  
- Id：`MemberId`（32 字节公钥）/ `RoomId`（16 字节）/ `EventId`（32 字节哈希）  
- `ShortId`：公钥前 8 字节，只用于同步帧  
- 错误：`Result<T, AppError>`；至少区分 `InvalidRoomCode`（校验位错误）、`NotOwner`、`RoomNotActive`（`Joining` / `Left` / `Dissolved` 时不能发送）、`TextTooLong`、`BackupPassphraseWrong`、`IdentityExists`

---

## 1. `core`

进程入口，也是唯一知道所有模块的地方。其他模块之间不互相回调。

```text
fn start() -> ()
fn stop() -> ()
fn on_packet(from: PeerAddr, raw: Bytes) -> ()   # transport 收到包后调用，见下方入站流程
```

**入站流程**

```text
transport 收到包
  → core：按包头 room_id 找到房间；protocol.open_packet 解密并解码成 Frame
  → Events 帧，对每条事件：
       protocol.verify_stateless（不通过则丢弃）
       storage.put_event（已存在则跳过；同序号不同内容则标记分叉）
       按 kind 分发：
         Chat → chat.on_ingested
         Heartsteel → heartsteel.recompute_author
         ConfigNotice / Dissolve → rooms.on_state_event，再 heartsteel.recompute_room
         其他状态事件 → rooms.on_state_event
         未知 kind → 只入库
       向 UI 发事件
  → Have / Want / WantFloors / Floors 帧 → sync
```

**出站流程**

```text
UI 调 Command → 业务模块在一个事务内写入事件，返回 Vec<SignedEvent>
  → core 调 sync.broadcast_new(room_id, events)
```

---

## 2. `identity`

```text
fn bootstrap() -> IdentityView
fn set_profile(nickname: String) -> IdentityView
  # core 随后为所有 Active 房间生成 Profile 事件，并交给 sync
fn export_backup(kind: IdentityOnly | IdentityAndRooms, passphrase: String) -> BackupFile
  # 口令必填；文件用 Argon2id 派生的密钥加密
fn import_backup(file: BackupFile, passphrase: String, overwrite: bool) -> IdentityView
  # 本机已有身份且 overwrite = false 时返回 IdentityExists；恢复后所有房间回到 Joining
fn public_fingerprint() -> String
```

`IdentityView { member_id, nickname, fingerprint }`

头像由 UI 按 `member_id` 和昵称生成，第一版没有头像字段。

---

## 3. `rooms`

```text
fn create_room(title: String, params: RuleParams) -> (RoomSummary, RoomCode)
fn join_room(code: RoomCode) -> RoomSummary
  # 立即返回 status = Joining；同步完成后发 RoomStatusChanged
  # 校验位错误时返回 InvalidRoomCode
fn leave_room(room_id: RoomId) -> ()              # Joining 时为取消，删除本地记录
fn dissolve_room(room_id: RoomId) -> ()           # 仅房主
fn list_rooms() -> Vec<RoomSummary>
fn set_active_room(room_id: RoomId) -> ()
fn rename_room(room_id: RoomId, title: String) -> ()                # 仅房主
fn set_room_rules(room_id: RoomId, params: RuleParams) -> Rule      # 仅房主，核心分配 rule_epoch
fn get_room_rules(room_id: RoomId) -> Rule        # 当前规则
fn get_room_code(room_id: RoomId) -> RoomCode     # 群管理页复制用；不写日志
fn list_members(room_id: RoomId) -> Vec<MemberView>
fn mark_read(room_id: RoomId) -> ()               # 已读游标移到当前最大 local_seq
fn set_room_prefs(room_id: RoomId, prefs: RoomPrefs) -> ()
```

内部：

```text
fn on_state_event(event: &SignedEvent) -> Vec<RoomChange>    # 更新缓存：房主、成员、昵称、群名、规则、解散
fn rebuild_state(room_id: RoomId) -> ()                      # 从 events 重建缓存
```

```text
RuleParams  { window_ms, threshold, cooldown_ms }            # 范围见架构 5.1
Rule        { rule_epoch, window_ms, threshold, cooldown_ms }
RoomStatus  = Joining | Active | Left | Dissolved
RoomPrefs   { muted, pinned }
RoomSummary { room_id, title, last_active, unread, is_owner, status, muted, pinned }
MemberView  { member_id, fingerprint, nickname, stacks, path, is_owner, is_fork_flagged, clock_skew_warning }
```

**Events**

```text
RoomUpdated { room_id }
RoomStatusChanged { room_id, status }
MemberJoined | MemberLeft { room_id, member_id }
ActiveRoomChanged { room_id }
RoomRulesChanged { room_id, rule }
UnreadChanged { room_id, unread }
ForkDetected { room_id, member_id }
```

---

## 4. `chat`

```text
fn send_text(room_id: RoomId, text: String) -> ChatMessage
  # 事务内：写聊天事件 + heartsteel.evaluate_on_send + 可能写叠层事件
  # 文本上限 900 字节；非 Active 返回 RoomNotActive
fn list_messages(room_id: RoomId, before: Option<TimelineCursor>, limit: u32) -> Vec<TimelineItem>
```

```text
TimelineItem   = Chat(ChatMessage) | Notice(RoomNotice)
TimelineCursor { display_ts, event_id }
ChatMessage    { event_id, author_id, ts_ms, display_ts, text, delivery: Option<Pending | Sent> }
                 # delivery 只对本人消息有值
RoomNotice     { event_id, kind: Join | Leave | Rename | ConfigNotice | Dissolve, author_id, ts_ms, display_ts }
```

内部：

```text
fn on_ingested(event: &SignedEvent, is_live: bool) -> TimelineItem    # 计算 display_ts，更新未读
```

**Events**

```text
TimelineItemAdded { room_id, item, is_live }
ChatDeliveryChanged { room_id, event_id, delivery }
```

`send_text` 的结果直接返回给 UI，不再发 `TimelineItemAdded`，所以 UI 不需要去重。

---

## 5. `sync`

合并了原 `reliability`。不对 UI 暴露协议细节。

```text
fn broadcast_new(room_id: RoomId, events: Vec<SignedEvent>) -> ()
  # 首发，并安排 6 / 15 / 30 秒重发，直到被 Have 确认
fn on_have(room_id: RoomId, from: MemberId, period_s: u16, vector: Vec<HaveEntry>) -> ()
  # 更新确认状态和在线状态；计算缺口并排入 Want 队列；抑制本机待发的 Have
fn on_want(room_id: RoomId, from: ShortId, ranges: Vec<WantRange>) -> ()    # 按速率发 Events（live = false）
fn on_want_floors(room_id: RoomId, from: ShortId, n: u32) -> ()             # n 不超过本机上限
fn on_floors(room_id: RoomId, vector: Vec<(ShortId, u64)>) -> ()            # 写 chat_floor
fn begin_join(room_id: RoomId) -> ()                                        # Joining 流程，见架构 7.4
fn set_room_activity(room_id: RoomId, level: Active | Background | Passive | Off) -> ()
  # Background = 后台房；Passive = 已解散；Off = 已退出
```

```text
HaveEntry { author: ShortId, state_max: u32, chat_max: u32 }
WantRange { author: ShortId, stream, from_seq, to_seq }     # 单次合计最多 64 个事件
```

**Events**

```text
SyncProgress { room_id, missing, fetched }
SyncComplete { room_id }
```

---

## 6. `heartsteel`

```text
fn leaderboard(room_id: RoomId) -> Vec<(MemberId, u32)>     # 只含当前成员，按层数降序
fn stacks_of(room_id: RoomId, member_id: MemberId) -> u32
```

房间规则经 `rooms.get_room_rules` / `rooms.set_room_rules`，不在本模块设置。音效偏好在 `settings`。

**Events**

```text
StacksChanged { room_id, changes: Vec<(MemberId, u32)> }
  # 任何层数变化（实时或补齐），用于更新角标和排行
HeartsteelLive { room_id, member_id, new_stacks, event_id }
  # 只对 live 的有效事件（判定见架构 5.6），用于播放音效和动画
```

**内部**

```text
fn evaluate_on_send(room_id, new_msg, tx) -> Option<SignedEvent>
  # 只被 chat.send_text 调用，和聊天事件同一事务；其他路径一律不判定
fn evaluate_emit(my_chats, last_valid, rules, current_rule) -> Option<Vec<Trigger>>    # 纯函数，架构 5.3
fn replay_author(events, rules, dissolve_ts) -> Vec<HeartsteelEvent>                   # 纯函数，架构 5.4
fn recompute_author(room_id, author_id, tx) -> Option<StackChange>
fn recompute_room(room_id, tx) -> Vec<StackChange>
```

`HeartsteelBody { rule_epoch, triggers: Vec<Trigger> }`  
`Trigger { event_id, ts_ms }`

---

## 7. `protocol`

纯函数模块，不做 I/O，负责全部线上格式（架构第 4、6 节）。

```text
fn parse_room_code(s: &str) -> Result<ParsedCode>            # 忽略大小写和连字符，检查校验位
fn format_room_code(secret, owner_tag) -> RoomCode
fn build_event(unsigned: UnsignedEvent, sk) -> SignedEvent   # 计算 event_id 并签名
fn verify_stateless(event: &SignedEvent, room_id) -> Result<()>   # 架构 4.3
fn seal_packet(room_key, room_id, frame: &Frame) -> Vec<Bytes>    # 超过 1200 字节时拆成多包
fn open_packet(room_key, raw: &[u8]) -> Result<Frame>
fn room_id_of_packet(raw: &[u8]) -> Option<RoomId>           # 只读包头，用于找到密钥
```

```text
Frame = Events { live, events }
      | Have { from, period_s, vector }
      | Want { to, ranges }
      | WantFloors { to, n }
      | Floors { vector }
```

```text
ChatBody { text }
RoomCreateBody { title, rule }
ConfigNoticeBody { rule }
RenameBody { title }
ProfileBody { nickname }
HeartsteelBody { rule_epoch, triggers }
# Join / Leave / Dissolve 的 body 为空
```

---

## 8. `crypto`

```text
fn derive_room_keys(secret, owner_tag) -> (RoomKey, RoomId)   # HKDF，架构第 3 节
fn seal(room_key, nonce, aad, plaintext) -> Ciphertext        # XChaCha20-Poly1305
fn open(room_key, nonce, aad, ciphertext) -> Plaintext
fn generate_identity() -> (IdentitySk, MemberId)              # Ed25519
fn sign(identity_sk, event_id) -> Signature
fn verify(member_id, event_id, sig) -> bool
fn fingerprint(member_id) -> String
fn protect_local(bytes) -> Bytes                              # Windows DPAPI
fn unprotect_local(bytes) -> Bytes
fn derive_backup_key(passphrase, salt) -> BackupKey           # Argon2id
```

---

## 9. `transport`

只收发不透明字节，不知道房间密钥，也不解析帧内容。

```text
fn start() -> ()
fn stop() -> ()
fn lan_broadcast(packet: Bytes) -> ()
fn lan_send(addr: PeerAddr, packet: Bytes) -> ()
fn relay_publish(room_id: RoomId, packet: Bytes, priority: Live | Control | Backfill) -> ()
fn relay_subscribe(room_id: RoomId) -> ()
fn relay_unsubscribe(room_id: RoomId) -> ()
fn set_internet_allowed(allowed: bool) -> ()
fn set_relays(urls: Vec<String>) -> ()
fn relay_status() -> Vec<RelayHealth>
```

路径判断（架构 6.5）放在 `sync`，因为它要结合成员信息和心跳时间。

```text
PeerAddr = Lan(SocketAddr) | Relay
PathKind = Lan | Relay | Unknown
RelayHealth { url, state: Ok | Cooling | Retired | Connecting, last_echo_ms }
```

**Events**

```text
TransportPathChanged { room_id, member_id, path }     # 由 sync 发出
RelayHealthChanged { url, state }
ProtocolMismatch { room_id, remote_proto }            # 由 core 发出
```

---

## 10. `storage`

```text
fn put_event(event: StoredEvent, tx) -> PutResult     # Inserted | Duplicate | Fork
fn get_event(event_id) -> Option<StoredEvent>
fn query_timeline(room_id, before, limit) -> Vec<StoredEvent>
fn query_stream(room_id, author, stream, from_seq, to_seq) -> Vec<StoredEvent>
fn query_heartsteel(room_id, author) -> Vec<StoredEvent>
fn query_state_events(room_id) -> Vec<StoredEvent>
fn have_vector(room_id) -> Vec<HaveEntry>
fn missing_ranges(room_id, author, stream, from_seq, to_seq) -> Vec<Range>
fn next_seq(room_id, author, stream) -> u64
fn last_ts(room_id, author) -> u64
fn recent_chat_floors(room_id, n) -> Vec<(MemberId, u64)>
fn set_delivery(event_id, delivery) -> ()
fn upsert_member(...) -> ()
fn put_rule(room_id, rule) / get_rules(room_id) -> HashMap<u32, Rule>
fn get_chat_floor(room_id, author) / set_chat_floor(room_id, author, seq)
fn get_read_cursor(room_id) / set_read_cursor(room_id, local_seq)
fn load_settings() / save_settings()
fn export_room_bundle(room_id) / import_room_bundle(...)
```

---

## 11. `settings`（经 `core` 暴露）

```text
fn get_settings() -> LocalSettings
fn update_settings(patch: LocalSettingsPatch) -> LocalSettings
```

心之钢音效内置在前端（2026-11-02 所有者决定），不再提供选择音效文件的接口。

```text
LocalSettings {
  sound_enabled, volume,
  internet_allowed, relays: Vec<String>,
  history_limit_lan,      # 默认 5000
  history_limit_relay,    # 默认 500
}
```

这些都是本机偏好，不影响层数计算。已全部实现：`internet_allowed`、`relays`、`sound_enabled`（默认开）、`volume`（0–100，默认 80）、`history_limit_lan`（默认 5000）、`history_limit_relay`（默认 500）。中继列表改回和默认一模一样时不再单独保存，以后跟随默认列表。

---

## 12. UI 桥（Tauri）

**Commands**  
`identity_*` · `backup_export` · `backup_import` · `rooms_*`（含 `rooms_get_code`、`rooms_set_prefs`、`rooms_set_active`、`rooms_rule`、`rooms_rename`、`rooms_set_rule`、`rooms_leave`、`rooms_dissolve`、`rooms_cancel_join`）· `chat_send_text` · `chat_list` · `heartsteel_leaderboard` · `settings_*` · `relay_status` · `network_status`（局域网端口及是否被占）

**Events**  
`chat://item` · `chat://delivery` · `heartsteel://stacks` · `heartsteel://live` · `room://*` · `sync://progress` · `transport://path` · `transport://relay` · `protocol://mismatch`

前端流程：

```text
chat_send_text → 返回 ChatMessage（已入库，delivery = Pending）→ 追加到列表
on chat://delivery → 更新发送中/已发送
on chat://item → 按 display_ts 插入（后端保证同一事件只发一次）
on heartsteel://stacks → 更新角标和排行
on heartsteel://live → 本机音效和动画
on room://status → 切换 Joining / Active / Left / Dissolved 的界面
```

---

## 13. 模块依赖方向

```
ui → core（通过 Tauri Commands / Events）
core → identity, rooms, chat, sync, heartsteel, settings, protocol, transport, storage, crypto
settings → storage
chat → heartsteel, protocol, storage
heartsteel → protocol, storage
rooms → protocol, crypto, storage
identity → crypto, storage
sync → protocol, transport, storage
protocol → crypto
transport、crypto、storage → 不依赖其他业务模块
```

禁止：

- UI 直接碰 transport / crypto 细节；
- transport 解析帧内容；
- 除 `chat.send_text` 外的任何路径生成叠层事件；
- 层数绕过重放直接修改；
- 业务模块反向调用 `core`（需要发送就把事件返回给 `core`）。

---

## 14. 优先写的测试（`cargo test`）

| 测试 | 内容 |
|------|------|
| 叠层向量 | 架构 5.5 的 V1–V9、R1–R7 |
| 收敛性质 | 随机打乱事件到达顺序，层数不变；诚实作者的事件重放全部有效 |
| 恢复不重复 | 本人旧消息经同步入库，不产生叠层事件 |
| 房间码 | 往返解析；打错一位被校验位拦下；换 `owner_tag` 得到不同的 `room_id` |
| 房主信任根 | 非房主的 `RoomCreate`、`ConfigNotice`、`Dissolve` 不生效 |
| 事件 | 固定输入得到固定 `event_id`（golden 向量）；篡改任一字段后验签失败 |
| 未读 | `ts` 很旧的迟到消息也计入未读；首次进房的历史不计入 |
| 音效 | 补齐帧的叠层事件只发 `StacksChanged`，不发 `HeartsteelLive` |

---

## 15. 文档状态

- 2026-11-02 按设计审查修订：合并 `reliability` 到 `sync`，新增 `core`、`protocol`、`settings`，移除 `friends` 和打洞接口，补齐缺失的接口。
- 原文引用的 `design-review.md` 未入库，相关引用已移除。
- 2026-11-02 M1-1 已实现：`crypto`、`protocol`（房间码、事件、事件内容）、`heartsteel`（判定与重放）、`rooms`（状态推导），共 44 个单元测试。包与帧、存储、网络和界面尚未实现。
- 2026-11-02 M1-2 已实现：`storage`（SQLite）、`identity`（主密钥存 Windows 凭据管理器）、`rooms` 建房与进房、`chat`（发言、消息列表、未读）、`heartsteel` 重算与层数缓存、`core` 的入站处理。`core` 在代码里叫 `app_core`（Rust 标准库已有 `core`），对外是 `Core` 结构体，事件通知为 `CoreEvent`。共 80 个单元测试（另有 1 个读写真实凭据管理器的测试，默认不跑，用 `cargo test --lib keystore -- --ignored` 手动验证）。网络（`sync`、`transport`、`protocol` 的包与帧）、`settings` 和界面尚未实现；进房后 Joining 转 Active 在 M1-3。
- 2026-11-02 M1-3 已实现：`protocol` 的帧与加密包、`transport` 的局域网 UDP 与洪水防护、`sync` 的同步决策（纯函数），以及节点 `node`（对应 `sync` 的运行部分，独占 `Core`，处理命令、收包、定时器）。界面桥 `commands.rs` 提供 10 个命令，事件统一为 `hs://event`。界面是最小可用版（房间、聊天、成员与层数）。共 114 个单元测试，含 4 个双节点集成测试。中继（M1-4）、排行榜与音效（M1-5）、聊天补齐与重发（M2）尚未实现。
- 2026-11-02 M1-3 两台机器局域网验收通过（release 版 exe，同一路由器、关闭代理）：进房、实时聊天、连发叠层两边层数一致、关掉重开后数据还在。
- 2026-11-03 M1-4 已实现：`transport` 的 Nostr 事件、发送预算、中继连接池（`RelayPool`），`sync` 的路径判定，`settings`（允许互联网、中继列表，表结构第 2 版），节点接入中继。和本文档第 9 节的差异：订阅不按房间单独增删，而是整体更新房间列表（`set_rooms`）；关闭互联网或改中继列表时整个连接池重建，没有单独的 `set_internet_allowed` / `set_relays`；路径变化不发事件，界面每 5 秒查一次。`LocalSettings` 目前只有 `internet_allowed` 和 `relays`。新增命令 `settings_get`、`settings_update`、`relay_status`，`rooms_members` 带路径。共 147 个单元测试，含 8 个连本地假中继的连接池测试和 4 个经假中继的多节点集成测试。
- 2026-11-03 M3 已实现：
  - 群管理：`rooms_rule`、`rooms_rename`、`rooms_set_rule`、`rooms_leave`、`rooms_dissolve`、`rooms_cancel_join`；
  - 托盘常驻；
  - NSIS 安装包。
  - 共 185 个单元测试。
  - 验收对照见 [acceptance.md](acceptance.md)。
- 2026-11-03 M2 已实现：
  - 送达状态与 6/15/30 秒重发（`NodeEvent::Delivered`，界面通知类型 `delivered`）；
  - 聊天补齐（`WantFloors` / `Floors`、`sync_state`、设置项 `history_limit_lan` / `history_limit_relay`）；
  - 后台房降频（`rooms_set_active`）与网络恢复；
  - 身份与房间备份：`backup_export(kind, passphrase, stamp)` 返回文件路径，`backup_import(data, passphrase, overwrite)` 的 `data` 为 Base64，错误 `IdentityExists` 等。
  - 共 180 个单元测试；单机双开自测：离线期间的消息重连后补齐且不重复、已发送标记、完整备份导出后覆盖导入。
- 2026-11-03 M1-5 已实现：本机设置加音效开关和音量；房间免打扰、置顶（`rooms_set_prefs`）；排行榜命令 `heartsteel_leaderboard`（同层同名次）；单实例（再次打开只调出已有窗口）；局域网端口被占时退回随机端口并在界面提示（`network_status`）。界面改为微信式：头像带层数角标、群资料抽屉（成员网格、排行榜、免打扰和置顶）、设置页。共 155 个单元测试；界面用单机双开脚本自测（角标与排行榜一致、免打扰和音量 0 不响、音量 80 响一次、置顶、开关互联网、非法中继地址）。
- 2026-11-03 M1-4 跨网两机验收通过（release 版 exe，一台家里宽带、一台移动或联通手机热点，都关闭代理）：进房、实时聊天、连发叠层两边层数一致、成员显示「中继」、关掉重开后数据还在。
