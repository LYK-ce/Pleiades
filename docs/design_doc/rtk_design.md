# RTK 局域差分定位接入设计（RTK Design）

> 创建日期：2026-08-31
> 状态：架构已定，待实现（本文固化已讨论的决策；「待定细节」见 §9）
> 参考实现：`uploads/lg290p_enu.py`（临时 SSH 方案的 Python 参考）
> 使用文档：`uploads/RTK_UM960_LG290P_使用文档.md`

---

## 1. 背景与目标

### 1.1 背景

车（UGV）当前用 **SLAM + 里程计** 做相对定位，随时间漂移。RTK（Real-Time Kinematic）可提供**相对基站的厘米级**定位，作为新的定位源。

现有参考是**临时 SSH 方案**：`lg290p_enu.py` 在本机（Pictor 电脑）读 UM960 基站，通过 SSH 把 RTCM 转发给 Jetson 上的 LG290P，再经 SSH 读回 GGA 换算 ENU。该方案依赖 SSH、单机单点，需集成进 Orion 的 libp2p 数据面。

### 1.2 目标

1. 地面站（`pleiades-terminal`）作为 **RTK 基站节点**，接 UM960；
2. 车（`pleiades-ugv`）作为 **流动站**，接 LG290P，获得厘米级定位；
3. RTCM 改正数据走 **gossipsub 广播**，复用现有 robot 数据面；
4. 定位结果以 offset 方式更新 `RobotState`。

---

## 2. RTK 原理与硬件拓扑

### 2.1 原理简述

RTK 需要**两个 GNSS 接收机**差分：

- **基站（Base）**：位置已知（Survey-in 自动确定），观测卫星后计算**改正数**，以 **RTCM（二进制）** 广播；
- **流动站（Rover）**：用自己的原始观测 + 基站改正数，**在自己内部**解算出相对基站的厘米级位置，以 **NMEA GGA（文本）** 输出。

关键认知：

- 基站广播的是**改正数**，不是「位置」；解算发生在**流动站内部**；
- 得到的是**相对基站**的 ENU 坐标（不是全球绝对坐标）；
- **RTCM = 改正数据（二进制）**，**GGA = 位置结果（文本）**，两者方向相反、格式不同。

### 2.2 硬件拓扑

| 设备 | 角色 | 连接位置 | 串口 |
|------|------|---------|------|
| UM960 | RTK 基站（Base） | Pictor 电脑（terminal 节点） | `/dev/ttyUSB0` @ 460800 |
| LG290P | 流动站（Rover） | 无人车（ugv 节点） | `/dev/ttyUSB2` @ 460800 |

### 2.3 ENU 坐标

ENU 原点 = 基站天线相位中心：`E` 向东 +，`N` 向北 +，`U` 向上 +（米）。

---

## 3. 架构决策（已定）

| # | 决策 | 结论 |
|---|------|------|
| D1 | UM960 驱动放哪 | **`pleiades-terminal`**（地面站 = 控制终端 = RTK 基站，同一节点/peer_id；以后要独立再拆） |
| D2 | LG290P 驱动放哪 | **`pleiades-ugv`**（车侧，任务「设备跟节点走」） |
| D3 | 三个纯函数归属 | `parse_gga` / `ecef` / `enu` **基站、车各写一份，不共享**（设备端彻底分离） |
| D4 | RTCM 封装 | **包 ORION 帧（新增 `msgid=6`）**，payload 装 RTCM 原始帧（双层帧） |
| D5 | 进程内通道 | **复用 robot_bus**，不开 rtk_bus |
| D6 | 快照重放 | **复用现有统一机制**（发布即更新快照），零改动；RTCM 重放无害 |
| D7 | 车侧认领 msgid | **方式 A**：LG290P 驱动独立订阅 robot_bus，自己 `match msgid==6`；`cluster_consumer` 不改 |
| D8 | RTCM 切帧 | **基站侧切帧广播**（做法①），车侧收到即完整帧 |
| D9 | 定位更新方式 | offset（相对初始 ENU 位移）更新 `RobotState.x/y/z`；`RTK_FIXED` 才覆盖，失锁保持 |

---

## 4. 数据流

```html
<div style="font-family:sans-serif;max-width:780px;font-size:13px;color:#334155;">
  <div style="display:flex;flex-direction:column;gap:6px;">
    <div style="background:#eef2ff;border:1px solid #c7d2fe;border-radius:8px;padding:8px 10px;">
      <b style="color:#3730a3;">① 基站（terminal / UM960）</b>
      <div style="color:#4b5563;">Survey-in 定原点（读 GGA quality=7）→ 读 RTCM 二进制 → <b>切帧</b> → 包 ORION(msgid=6) → <code>Gossipsub_Publish(TOPIC_RTK_RTCM, orion_frame)</code></div>
    </div>
    <div style="text-align:center;color:#94a3b8;">↓ gossipsub 广播（一份 RTCM 给所有车）</div>
    <div style="background:#ecfdf5;border:1px solid #a7f3d0;border-radius:8px;padding:8px 10px;">
      <b style="color:#065f46;">② 车（ugv）—— network 收到</b>
      <div style="color:#4b5563;"><code>swarm_events</code> 按 topic 名匹配 → <code>robot_bus.Publish(StreamRaw)</code> 原样透传</div>
    </div>
    <div style="text-align:center;color:#94a3b8;">↓ robot_bus（进程内）</div>
    <div style="background:#fffbeb;border:1px solid #fde68a;border-radius:8px;padding:8px 10px;">
      <b style="color:#92400e;">③ LG290P 驱动（车侧）</b>
      <div style="color:#92400e;">订阅 robot_bus → 解析 ORION → <code>match msgid==6</code> → 取 payload（RTCM 帧）→ <b>写串口喂 LG290P</b></div>
    </div>
    <div style="text-align:center;color:#94a3b8;">↓ LG290P 解算</div>
    <div style="background:#f8fafc;border:1px solid #e2e8f0;border-radius:8px;padding:8px 10px;">
      <b style="color:#0f172a;">④ 读回位置</b>
      <div style="color:#4b5563;">LG290P 输出 GGA → <code>parse_gga</code> → <code>ecef</code> → <code>enu</code> → offset 更新 <code>RobotState</code></div>
    </div>
  </div>
</div>
```

---

## 5. base 改动清单

### 5.1 ORION 协议扩展

- `protocol/mod.rs`：新增 `MSGID_RTCM = 6`（现有 1-5 为 POSE / MAP_FULL / MAP_DELTA / MANUAL_CONTROL / TASK_SET）；
- `protocol/messages.rs`：新增 `encode_rtcm(rtcm_frame) -> Vec<u8>`（套 ORION 帧头 + `msgid=6` + payload 填 RTCM 字节）。

### 5.2 gossipsub（4 处机械改动）

| 文件 | 改动 |
|------|------|
| `Network/Gossipsub/mod.rs` | 加 `TOPIC_RTK_RTCM` 常量（建议 `"pleiades/robot/rtcm"`） |
| `Network/network_service.rs` | 订阅列表加 `gossipsub::IdentTopic::new(TOPIC_RTK_RTCM)` |
| `Network/swarm_events.rs` | match 加分支：`TOPIC_RTK_RTCM => robot_bus.Publish(StreamRaw { payload: message.data })`（与 POSE/MAP 同款透传） |
| `Network/mod.rs` | `pub use ... TOPIC_RTK_RTCM` |

> 快照重放**无需改动**：`command_handler` 里「发布即 `snapshot_cache.Update`」是统一机制，RTCM 会自动重放最近一帧（无害甚至帮助新车更快进入 Fixed）。

---

## 6. 驱动改动清单

### 6.1 UM960 基站驱动（`pleiades-terminal`）

复用 `Robot/util/serial/port.rs` 的 `spawn_port`。状态机：

```
初始化 → 发配置命令(UNLOG → MODE BASE TIME N → GPGGA COM1 1)
       → Survey-in 等待（读 GGA 等 quality=7 且连续两帧稳定）
       → 关 GGA(UNLOG COM1 GPGGA) → 开 RTCM(RTCM1006/1033/1074/1084/1094/1124)
       → 持续：读 RTCM → 切帧(Rtc3Parser) → 包 ORION(msgid=6) → Gossipsub_Publish
```

- TX：发文本命令（`\r\n` 结尾）；
- RX：切帧状态机 `Rtc3Parser`（找 `0xD3` → 读长度 → 攒满切出）；
- 切帧是**同步纯逻辑**，跑在 `on_bytes` 回调里；广播走 mpsc channel → async task（复用现有 STM32/LiDAR 模式）。

### 6.2 LG290P 流动站驱动（`pleiades-ugv`）

- 订阅 robot_bus（broadcast，与 `cluster_consumer` 并列的又一个订阅者）；
- 解析 ORION 帧 → `match msgid==6` → 取 payload（RTCM 帧）→ 写串口；
- RX 回调读 GGA → `parse_gga` → `ecef` → `enu` → offset 更新 `RobotState`。

---

## 7. 坐标约定与转换

| 坐标系 | 轴约定 |
|--------|--------|
| ENU（RTK） | E=东 +，N=北 +，U=上 + |
| 世界（robot） | x=东 +，y=南 +，z=上 + |

**offset 更新**（记录车初始 ENU `(E₀, N₀, U₀)`，对齐世界原点 `(x₀, y₀, z₀)`）：

```
world_x = x₀ + (E − E₀)
world_y = y₀ − (N − N₀)     // ⚠️ N 北为 +，世界 y 南为 +，需取反
world_z = z₀ + (U − U₀)
```

- 对齐的是「**车的初始 ENU**」↔ 世界原点，**不是**基站 Survey-in 原点（Survey-in 是基站侧的事）；
- RTK 只给**位置**（经纬高），**不给朝向**：`RobotState.yaw` 不更新，仍靠 IMU/里程计。

---

## 8. 关键注意点

1. **RTK_FIXED 才覆盖**：`quality==4` 才用 offset 更新位置；失锁（FLOAT/SINGLE）保持上次值，否则车会跳飞。
2. **RTCM 是二进制流**：无换行分界，靠 `0xD3` 帧头 + 长度字段切帧；串口 `read` 分块随机，必须用缓冲区攒帧。
3. **Survey-in**：约 120 秒，期间基站天线须静止；基站移动后必须重新 Survey-in。
4. **切帧在基站侧**：保证每条 gossipsub 消息 = 一帧完整 RTCM，车侧零拼接。
5. **双层帧**：外层 ORION（传输 + 身份 + 校验），内层 RTCM（业务数据），车侧解两层（现有 POSE/MAP 同款）。

---

## 9. 待定细节

| 项 | 待定内容 |
|----|---------|
| topic 名 | 建议 `pleiades/robot/rtcm`（未最终拍板） |
| ORION 帧身份 | `sysid` = terminal PeerId、`compid` = `GROUND_STATION(200)`（待确认） |
| Survey-in 等待 | 阻塞等待 vs 状态机阶段 + 超时（待定） |
| offset 基准 | 初始 ENU `(E₀,N₀,U₀)` 存哪、何时记录（首次 FIXED 时记录，待定） |
| LG290P 放 `pleiades-ugv` | 倾向已定，但未最终拍板（车/机共享时可能需抽到 base） |
