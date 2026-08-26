# 终端接口差异：Godot-Library → Universal-Robot

> Created Date ： 2026-08-26
> Modified Date ： 2026-08-26
> 目的：说明 `Godot-Library` 分支 → 当前 `Universal-Robot` 分支（含 task_22_4 飞控接入）机器人节点与地面站/终端（Pictor）之间的通信协议差异，供终端侧同步适配。

---

## 一、差异总览

线上（wire）协议只有 **两处破坏性/新增差异** + **一处语义变化**：

| # | 项 | 类型 | 影响 |
|---|---|---|---|
| 1 | ORION_POSE（msgid=1）`PoseData` 增加 `z: f32`，payload **33→37 字节** | 🔴 破坏性 | 旧终端解析会失败/错位 |
| 2 | ORION_MANUAL_CONTROL（msgid=4）`action` 新增 `takeoff=10` / `land=11` | 🟢 新增枚举 | 旧终端发 0~9 仍兼容 |
| 3 | ORION_MANUAL_CONTROL 的 `param`（速度档位）对 forward/backward/spin 被忽略，速度改由设备层 config 绑定 | 🟡 语义变化 | 不破坏字节布局，但地面站发速度不再生效 |

其余消息（`TASK_SET`、`MAP_FULL`、`MAP_DELTA`）与帧头（`Frame`）在 `Godot-Library` 分支时已是最终形态，**无变化**。

---

## 二、帧格式 Frame（无变化）

两分支完全一致（Task 13 已把 sysid 从 1B 末字节升级为变长完整 peer_id，早于 Godot-Library 基线）：

```
magic(1B 0x4F) | len(4B u32 BE) | seq(1B) | sysid_len(1B) | sysid(N) | compid(1B) | msgid(2B u16 BE) | payload(len) | checksum(2B)
```

| 字段 | 长度 | 值/语义 |
|---|---|---|
| `magic` | 1B | `0x4F`（'O'） |
| `len` | 4B u32 BE | payload 字节数 |
| `seq` | 1B | 第一版恒 `0` |
| `sysid_len` | 1B | sysid 字节数；`0` = 无身份（地面站上行，配 compid=200） |
| `sysid` | N | 发送方完整 libp2p PeerId 二进制（Ed25519 下 38B） |
| `compid` | 1B | `1`=车/机，`200`=地面站/终端 |
| `msgid` | 2B u16 BE | 1=POSE / 2=MAP_FULL / 3=MAP_DELTA / 4=MANUAL_CONTROL / 5=TASK_SET |
| `payload` | N | 按 msgid 定义 |
| `checksum` | 2B | 第一版恒 `0`（无 CRC16） |

---

## 三、ORION_POSE（msgid=1）`PoseData` —— 🔴 破坏性

### 旧版（Godot-Library，33 字节）

```
time_boot_ms u32 | x f32 | y f32 | vx f32 | vy f32 | yaw f32 | valid u8 | sub_gx i32 | sub_gy i32
   0..4          | 4..8  | 8..12 | 12..16 | 16..20 | 20..24  | 24       | 25..29     | 29..33
```

### 新版（当前，37 字节）

```
time_boot_ms u32 | x f32 | y f32 | z f32 | vx f32 | vy f32 | yaw f32 | valid u8 | sub_gx i32 | sub_gy i32
   0..4          | 4..8  | 8..12 | 12..16| 16..20 | 20..24 | 24..28  | 28       | 29..33     | 33..37
```

| 项 | 旧版 | 新版 |
|---|---|---|
| 字段 | `time_boot_ms, x, y, vx, vy, yaw, valid, sub_gx, sub_gy` | `time_boot_ms, x, y, z, vx, vy, yaw, valid, sub_gx, sub_gy` |
| 字节数 | 33 | **37** |
| `z` | 无 | **新增 `f32`（4B），插在 `y` 之后、`vx` 之前** |
| 后续字段偏移 | — | `vx/vy/yaw/valid/sub_gx/sub_gy` 全部 **+4** |
| 长度校验 | `decode_pose` 校验 ==33 | `decode_pose` 校验 ==37（旧 33B 帧会被拒绝） |

字段语义：
- `time_boot_ms` u32 开机毫秒；`x/y/z` 全局世界坐标（m，z 为垂直高度，车恒 0、机写飞控 EKF）。
- `vx/vy` 速度（m/s）；`yaw` rad（顺时针为正）。
- `valid` bool（1B）：意图有效标志，`false` = 无当前任务/子目标。
- `sub_gx/sub_gy` i32：D\* 寻路「下一格」网格坐标，`valid=false` 时忽略。

---

## 四、ORION_MANUAL_CONTROL（msgid=4）—— 🟢 新增枚举

payload 3 字节不变：`action u8 | param i16 BE`。

### action 枚举完整列表

| 值 | 动作 | param 含义 | 旧版 | 新版 |
|---|---|---|---|---|
| 0 | `forward` | 速度档位（i16，现被忽略，见 §六） | ✅ | ✅ |
| 1 | `backward` | 速度档位（同上） | ✅ | ✅ |
| 2 | `spin_left` | 速度档位（同上） | ✅ | ✅ |
| 3 | `spin_right` | 速度档位（同上） | ✅ | ✅ |
| 4 | `stop` | 忽略（0） | ✅ | ✅ |
| 5 | `beep` | 时长 ms（cast u16） | ✅ | ✅ |
| 6 | `start_lidar` | 忽略 | ✅ | ✅ |
| 7 | `stop_lidar` | 忽略 | ✅ | ✅ |
| 8 | `switch_to_manual` | 忽略 | ✅ | ✅ |
| 9 | `switch_to_auto` | 忽略 | ✅ | ✅ |
| **10** | **`takeoff`** | **忽略** | ❌ | ✅ **新增** |
| **11** | **`land`** | **忽略** | ❌ | ✅ **新增** |

对应常量（`protocol/messages.rs`）：`ACTION_TAKEOFF=10`、`ACTION_LAND=11`（Task 22_4）。

映射（`protocol/command_decode.rs`）：
- 0~7 → `Command::Manual(ManualCmd::*)`
- 8/9 → `Command::Mode(ModeCmd::SwitchToManual/SwitchToAuto)`
- 10 → `Command::Manual(ManualCmd::Takeoff)`（起飞固定 1.8m）
- 11 → `Command::Manual(ManualCmd::Land)`

---

## 五、其他消息（无变化）

### ORION_TASK_SET（msgid=5）

`MissionItem`（9 字节）：`mission_type u8 | x f32 BE | y f32 BE`（`MISSION_GOTO=0`、`MISSION_CIRCLE=1`）。

`TaskSetPayload`：`mission_count u8 | member_count u8 | members[member_count] | missions[mission_count]`，`members[i] = len u8 + peer_id[len]`。三分支（取消 / 单车 / 群发）两分支一致。

### ORION_MAP_FULL（msgid=2）

`time_boot_ms u32 | origin_gx i32 | origin_gy i32 | width u16 | height u16 | resolution f32 | data[width×height] i8`，总长 `20 + 65536`。`data` = log-odds 原始值 i8（−8~+8）。

### ORION_MAP_DELTA（msgid=3）

`time_boot_ms u32 | count u16 | entries[count]`，`entry = gx i32 | gy i32 | delta i8`（9B/项），总长 `6 + 9×count`。

---

## 六、MANUAL_CONTROL `param` 速度语义变化（🟡，Task 22_5 D2）

- 帧里 `param`（i16 速度档位）**仍然编码/解码**，且仍被 `command_decode` 塞进 `ManualCmd::Forward(param)` 等。
- 但当前 `dispatch` 对 `Forward/Backward/SpinLeft/SpinRight` 调用的是**无参** `motion.move_forward()/move_backward()/turn_left()/turn_right()`（`_` 忽略 param），实际速度由设备层 config 绑定：
  - 车：`chassis.forward_speed`（缺省 30）、`chassis.turn_speed`（缺省 10）
  - 机：`flight_ctrl.vel_fwd`（缺省 0.3 m/s）、`flight_ctrl.yaw_rate_deg`（缺省 15 °/s）
- **含义**：地面站仍按旧协议发 `param` 速度不会报错，但车/机实际速度不再由该字段决定；仅 `beep` 的时长 param 仍生效。

---

## 七、地面站 / Pictor / 终端适配清单

| # | 适配点 | 是否破坏旧终端 | 说明 |
|---|---|---|---|
| 1 | **POSE 解析 33→37 字节** | ✅ 会解析失败/错位 | 在 offset 12 处读 `z`（f32 BE），`vx/vy/yaw/valid/sub_gx/sub_gy` 相应后移 4 字节 |
| 2 | **MANUAL_CONTROL 编码新增 action 10/11** | 部分 | 旧终端发 0~9 仍兼容；需要起飞/降落则新增 `takeoff=10`/`land=11` 的编码与 UI 入口 |
| 3 | **MANUAL_CONTROL `param` 速度语义** | 不破坏，行为变化 | 地面站发的速度档位不再决定实际速度（改由 `config.toml` 绑定）；若依赖 param 调速需改为只发意图 |
| 4 | Frame / MAP_FULL / MAP_DELTA / TASK_SET | 无需改 | 两分支一致 |

**一句话升级指引**：Pictor/终端只需做两件事——① POSE 解析加 `z` 并把布局改成 37B；② 若要用无人机，MANUAL_CONTROL 编码加 `takeoff(10)`/`land(11)`。其余消息无需改动；另注意速度 param 现已由设备层绑定（地面站发速度不再生效）。
