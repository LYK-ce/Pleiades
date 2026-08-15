# Pictor 桥上/下行数据契约（Rust → Godot）

> 创建日期：2026-08-16
> 关联：`Task/task_16_pictor_kernel.md`（3.4 地面站消费侧 + 3.5 桥类）
> 用途：告知 Godot（Pictor）侧桥会发哪些信号、数据格式与编码是什么，方便那边接信号做渲染/控制。

---

## 一、总览

桥（`pictor-kernel`，产物 `libpictor_kernel.so`）跑在 Godot 进程内，内部起一个后台 tokio 线程运行无头 Pleiades，把两条总线的数据同步给 Godot：

| 数据源 | 内容 | Godot 信号 |
|---|---|---|
| `robot_bus`（ORION 遥测帧） | 各车位姿 + 合并地图 | `pose_received` / `map_updated` |
| `event_bus`（State JSON） | 节点上/下线、节点名 | `peer_connected` / `peer_disconnected` / `peer_info_updated` |
| 桥自身 | 后台初始化完成 | `kernel_ready` |

**同步机制（方案 B）**：后台同步 task 周期读数据 → 塞线程安全队列 → Godot 每帧调 `poll()` → 在主线程排空队列并 emit 信号（零跨线程发信号，不启用 experimental-threads）。

---

## 二、信号清单（上行：Rust → Godot）

### 1. `kernel_ready()`

- 无参。
- 含义：后台 `core_bootstrap` 完成、`NodeHandle` 就绪。
- 时机：就绪后发一次。
- ⚠️ Godot 在收到 `kernel_ready` 前不应调 `send_command`（未就绪会返回 `false`）。

### 2. `pose_received(peer_id: String, x: f32, y: f32, yaw: f32, vx: f32, vy: f32)`

- 来源：`ClusterInfoTable`（`robot_bus` 的 POSE 帧入表）。
- 时机：约 100ms 周期，**每辆车一条**（按车发）。
- 字段：
  - `peer_id`：**hex**（完整 peer_id 原始字节的 hex 编码，如 `00e9...`）。
  - `x` / `y`：全局世界坐标（米），世界范围 [0,128)。
  - `yaw`：朝向角（弧度）。
  - `vx` / `vy`：速度（米/秒）。

### 3. `map_updated(data: PackedByteArray)`

- 来源：`OccupancyGrid`（`robot_bus` 的 MAP_DELTA 帧合并进 merged 表）。
- 时机：约 100ms 周期（当前实现每次发**全量**，增量待实现）。
- 数据：65536 字节（256×256 格 × 0.5m 分辨率），log-odds **i8**（-8~+8）按 u8 位模式传输。
- 三态由显示层按阈值 ±6 派生：`>+6` 占用 / `<-6` 空闲 / 中间未知。

### 4. `peer_connected(peer_id: String)`

### 5. `peer_disconnected(peer_id: String)`

- 来源：`event_bus` State `type = peer_connected / peer_disconnected`（mDNS 发现 + TCP 连接建立/断开）。
- 时机：事件驱动。
- `peer_id`：**base58**（`PeerId::to_string()`，形如 `12D3Koo…`）。

### 6. `peer_info_updated(peer_id: String, peer_name: String)`

- 来源：`event_bus` State `type = peer_info_updated`（peer-info gossip）。
- 时机：事件驱动（节点上报名字后）。
- `peer_id`：base58；`peer_name`：节点名（车名）。

---

## 三、event_bus 原始事件与转发情况

`event_bus` 的 State JSON 有 `type` 字段，桥只转发其中 3 种：

| `type` | 是否转发 | 目标信号 | 说明 |
|---|---|---|---|
| `peer_connected` | ✅ | `peer_connected` | 连接建立 |
| `peer_disconnected` | ✅ | `peer_disconnected` | 连接断开 |
| `peer_info_updated` | ✅ | `peer_info_updated` | 节点名（gossip） |
| `peer_discovered` | ❌ 不转发 | — | mDNS 发现（仅内部日志） |
| `peer_left` | ❌ 不转发 | — | mDNS 过期 |

> `event_bus` 里还有 ML_review 侧的 models / sessions 等 State 事件，均与机器人无关、不转发。

---

## 四、⚠️ 已知问题：peer_id 编码不统一

- `pose_received` 的 `peer_id` = **hex**（来自 `ClusterInfoTable` 的原始字节）。
- `peer_connected / peer_disconnected / peer_info_updated` 的 `peer_id` = **base58**（来自 `event_bus` JSON 的 `PeerId::to_string()`）。

Godot 侧把「车 sprite」（hex 键）和「节点列表项」（base58 键）关联时，需要做编码转换（base58 ↔ hex ↔ bytes）。建议后续统一为 hex-of-bytes（见 task_16 3.4 决策），届时只改桥侧即可。

---

## 五、下行接口（Godot → Rust，补充）

| handle | 参数 | 返回 | 说明 |
|---|---|---|---|
| `send_command` | `peer_id: String(hex), frame: PackedByteArray` | `bool` | 下发 ORION 完整帧（Godot 拼帧），fire-and-forget |
| `poll` | — | — | 每帧调用，排空队列并 emit 信号 |

- `send_command` 的 `frame` 是**完整 ORION 帧**（magic/len/sysid/compid/msgid/payload/checksum），由 Godot 侧（Pictor 现有 `orion_frame` / `orion_messages` 编码）构造。
- `send_command` 的 `peer_id` 是 **hex**（与 `pose_received` 一致）。

---

## 六、Godot 侧接入要点（供参考）

1. `_process` 里每帧调 `kernel.poll()`。
2. 连接 `kernel_ready` → 之后才可发命令。
3. 连接 `pose_received` / `map_updated` 做渲染。
4. 连接 `peer_connected` / `peer_disconnected` / `peer_info_updated` 维护节点列表。
5. 注意 hex / base58 编码差异（第四节）。
