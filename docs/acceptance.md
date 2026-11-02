# 验收对照表

对应需求 2.3 的 8 条验收。每条列出：自动测试（`cd src-tauri && cargo test --lib` 全部覆盖）、在一台电脑上开两个实例做的自测、需要两台机器由所有者确认的部分。

状态截至 2026-11-03（M3 完成）。

| # | 验收 | 自动测试 | 单机双开自测 | 两台机器（所有者） |
|---|------|----------|--------------|--------------------|
| 1 | 一人建房分享房间码，其余人可加入；房间码输错时被校验位拦下 | `protocol::room_code` 的校验位测试；`app_core::tests` 建房、进房；`node::tests::two_nodes_join_chat_and_agree_on_stacks` | 建房、复制房间码、进房 | M1-3 局域网、M1-4 跨网验收已通过 |
| 2 | 同网与跨网均能进房与聊天（跨网经加密公共中继） | `node::tests`（局域网）；`node::relay_tests`（经本地假中继：只走中继、局域网加中继混合） | 两个实例都显示 8 个默认中继已连接、对方为「局域网」 | 2026-11-03 跨网验收通过（一台家里宽带、一台移动或联通热点，均关闭代理）。**待办：** 热点上逐个中继的测试结果补记到 `docs/relay/` |
| 3 | 按房间规则触发心之钢，全员层数一致（含后加入、乱序到达），本机可播音效 | `heartsteel` 判定与重放、`app_core::send_tests`（乱序、倒序一致）、`app_core::manage_tests`（改规则只影响以后的消息）；节点集成测试层数一致 | 角标与排行榜一致；实时叠层播放一次音效，免打扰或音量 0 时不响 | M1 两机验收层数一致 |
| 4 | 退出并重启后，聊天记录与层数仍在 | `storage` 重开文件、`app_core::restart_tests`、`node::tests::restarted_node_reconnects_and_stays_consistent` | 每次重启开发版数据都在 | M1 两机验收通过 |
| 5 | 可维护多个房间并切换 | `app_core::tests::room_prefs_change_only_given_fields_and_pinned_rooms_come_first`、`node::tests::only_the_active_room_uses_the_fast_have_period`、`app_core::manage_tests`（退出、重进、取消加入） | 多个房间切换、置顶、免打扰 | — |
| 6 | 丢包 / 断网重连后未确认消息自动补发，重复投递不产生重复气泡或叠层 | `sync::resend`；`node::tests::unconfirmed_message_is_resent_and_then_marked_sent`、`chats_missed_while_offline_are_backfilled` | 关掉乙后甲发 5 条显示「发送中」，乙重开后 5 条全部收到且无重复，甲的变为已发送 | 建议用安装包在两台机器上拔网线再插回复测一次 |
| 7 | 断线重连后一次补齐大量历史叠层，层数正确且不连续播放音效 | `node::tests::late_joiner_catches_up_state_without_old_sounds`；`sync::plan::missing_total_is_not_capped`（缺得多时补齐不会被误判为没进展） | 补齐的消息不触发浮动提示和音效 | — |
| 8 | 恢复身份备份后，同步回来的本人旧消息不产生新叠层 | `app_core::backup_tests`（只恢复身份、完整恢复、覆盖确认、口令错误）；`sync::plan::activation_waits_for_my_own_latest_chat`；`node::tests::importing_another_identity_resets_room_sync` | 完整备份导出后覆盖导入，房间回到加入中、和甲同步后恢复可发言，聊天记录都在 | 建议在第二台机器上用备份文件恢复一次 |

## M3 新增功能的自测

| 功能 | 怎么验证的 |
|------|------------|
| 群管理（改群名、改规则、退出、解散、取消加入） | `app_core::manage_tests`；`node::tests::leave_notice_goes_out_before_the_room_goes_quiet` |
| 托盘常驻 | 启动开发版后发关闭窗口消息：进程仍在、局域网端口仍在收、窗口隐藏；再次启动程序：窗口重新出现，只有一个进程 |
| 重复打开 | 第二次启动的进程立即退出，已有窗口被调到前面 |
| 安装包 | NSIS 安装包静默安装到临时目录，装好的程序能启动并加载内置界面，然后静默卸载 |

## 所有者待办

1. 在移动或联通热点上用 `tools/relay-probe/relay-test.html` 跑一次中继测试，把结果记进 `docs/relay/`（M1 验收第 4 条剩下的部分）。
2. 用安装包在两台机器上走一遍：安装、进房、聊天叠层、断网重连补发、备份恢复。
3. 推送（所有者自己来）。
