# Workbook — Task 24: RTK 局域差分定位接入

> 对应任务：`Task/task_24_rtk.md`
> 关联设计：`docs/design_doc/rtk_design.md`
> 分支：`Pleiades-Orion`
> 创建日期：2026-08-31

---

## 目标

terminal（地面站）= RTK 基站（UM960），ugv（车）= 流动站（LG290P）；RTCM 改正数走 gossipsub 广播；定位结果以 offset 更新 `RobotState.x/y`（z 不更新）。

## 关键决策（讨论定稿 2026-08-31）

- **D10**：offset 用「首次 FIXED 车位置」做 ENU base（`enu()` base 参数 = 车首次 FIXED GGA 坐标），**不需要基站坐标下行**（数学上 base 在差分中抵消）。
- **D11**：**z 不更新**（LG290P 忽略 U 分量，`RobotState.z` 保持设备端现状）。
- **`rtk_fixed` 实时写**：FIXED→true，失锁/非 FIXED→false（位置 x/y/z 保持）。
- **首次 FIXED 跳变**：靠操作约束「车停在 origin(64,64) 直到 FIXED，期间不动」解决（同一位置启动）。
- **RTCM 订阅**：config 驱动（`[Network].subscribe_topics`），不硬编码全节点订阅；ugv 配 `["pleiades/robot/rtcm"]`，terminal/uav/sim 不配。
- `survey_seconds` 默认 180；LG290P 默认 Rover 模式不需配；UM960 掉电配置丢失 → Init 每次无条件重发。

## 实施记录

### 阶段 0 — base 改动（2026-08-31 完成）

- `protocol/mod.rs`：加 `MSGID_RTCM: u16 = 6`。
- `protocol/messages.rs`：`PoseData` 加 `rtk_fixed: bool`；`encode_pose`/`decode_pose` 37→38B（末尾 1 字节）；测试同步（补 rtk_fixed + 38B）。
- `state.rs`：`RobotState` 加 `rtk_fixed: bool`；写者注释改「传感器字段设备端写 + x/y STM32 里程计+LG290P RTK 共写 + rtk_fixed LG290P 独写」。
- `robot.rs`：`state_notifier` 快照元组加 `rtk_fixed`，PoseData 构造加字段。
- `cluster/consumer.rs`：测试 `make_pose` 补 `rtk_fixed: false`。
- 网络：`Gossipsub/mod.rs` 加 `TOPIC_RTK_RTCM`；`Network/mod.rs` re-export；`swarm_events.rs` 透传分支加 RTCM；`config.rs`+`bootstrap.rs`+`network_service.rs` 加 `subscribe_topics` 配置订阅（Start 订阅 = 基础 5 个 + config 追加）。

### 阶段 1 — terminal 侧 UM960 基站（完成）

- `Cargo.toml` 补 serde/toml/toml_edit/tokio-util/tracing。
- `config.rs`（新）：`TerminalConfig`/`Um960Config`/`Ensure_Terminal_Config`（补 [um960] 默认 false/ttyUSB0/460800/180）。
- `device/um960/geo.rs`（新）：`GgaFix`/`parse_gga`/`ecef`/`enu`（照搬 python L27-94）+ 测试。
- `device/um960/rtcm_parser.rs`（新）：`Rtc3Parser` 切帧（0xD3+长度，不校验 CRC，4096 上限）+ 测试。
- `device/um960/mod.rs`（新）：`Um960Device` + 四态状态机 Init→Surveying→Stable→Broadcasting + `spawn`（复用 spawn_port，std RwLock phase）。
- `lib.rs`：`spawn_background` 里 core_bootstrap 后装配 UM960（enabled 才 spawn，失败告警不拖垮）。

### 阶段 2 — ugv 侧 LG290P 流动站 + STM32 写者改造（完成）

- `config.rs`：加 `Lg290pConfig` + `fill_lg290p`（[lg290p] 默认 false/ttyUSB2/460800）+ 测试。
- `device/lg290p/geo.rs`（新）：与 terminal 物理独立一份。
- `device/lg290p/mod.rs`（新）：`Lg290pDevice` + 3-task（TX/RX spawn_port + robot_bus 订阅）+ 2 态状态机 WaitingFirstFix→Tracking + `handle_gga_line`（FIXED offset 更新 x/y、y 取反、z 不动；失锁写 rtk_fixed=false）+ `rtcm_relay_loop`。
- `stm32/mod.rs` **写者改造**：删 local_state origin 注入；RPT_SPEED 改 `pending_dt=Some(dt)`；全量覆盖 → 传感器照写 + x/y 读-改-写（同一写锁内 `accumulate(&mut *guard, dt)`）；spawn_mock 同步改造 + test_mock_origin_injected 改手动注入 origin。
- `odometry.rs`：契约注释改「必须在 STM32 RX 回调同一写锁内调用」。
- `bootstrap.rs`：`CarDeviceHandler::new` 加 `robot_bus` 参数。
- `robot_handler.rs`：`CarInner`/`CarDeviceHandler` 加 lg290p/robot_bus；start 装配 LG290P（enabled 才 spawn，失败清理 stm32/lidar）；shutdown 清理。

## 验证

- `cargo check` 全 workspace 通过（无 error）。
- `cargo test -p pleiades-terminal`：10 passed。
- `cargo test -p pleiades-ugv`：87 passed。
- `cargo test -p pleiades-base` **无法完整编译**（既有问题：VM/ML_Engine/Storage/PeerManagement 等模块测试引用了过时 API，与 task24 无关）。

## 遗留 / 风险

- **跨端同步**：POSE 37→38B，Godot 侧 MessageParser 需同步升级；`decode_pose` 严格 38B，需整车/机/终端同版本合入。
- **code review 边界问题（用户决定暂不修）**：
  - `handle_gga_line` 缺 NaN 有限值校验（畸形 GGA 污染 RobotState 并广播）。
  - `stm32` `pending_dt` 单次覆盖，批量读多帧 RPT_SPEED 丢中间位移（spawn_mock 无影响，spawn 生产有影响）。
  - 轻微：dead-code 字段、try_send 静默吞、line_buf 无上限、测试覆盖不足（37B 拒绝/handle_gga_line/坏长度重同步）。
- 实机/双节点联调未做。
