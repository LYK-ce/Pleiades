# Pictor 桥上/下行数据契约（Rust → Godot）

> 创建日期：2026-08-16
> 关联：`Task/task_16_pictor_kernel.md`（3.4 地面站消费侧 + 3.5 桥类）
> 用途：告知 Godot（Pictor）侧桥会发哪些信号、数据格式与编码，方便那边接信号做渲染/控制。

---

## 一、总览

桥（`pictor-kernel`，产物 `libpictor_kernel.so`）跑在 Godot 进程内，是**哑管道**——不解析、不合并业务数据，只把两条总线的数据原样转发：

| 数据源 | 内容 | Godot 信号 |
|---|---|---|
| `robot_bus`（ORION 原始帧） | POSE / MAP_FULL / MAP_DELTA 帧，原样转发 | `robot_frame` |
| `event_bus`（State JSON） | 节点上/下线、节点名 | `peer_*` 系列 |
| 桥自身 | 后台初始化完成 | `kernel_ready` |

**同步机制（方案 B）**：后台 task 收数据 → 塞线程安全队列 → Godot 每帧调 `poll()` → 在主线程排空队列并 emit 信号（零跨线程发信号，不启用 experimental-threads）。

---

## 二、信号清单（上行：Rust → Godot）

### 1. `kernel_ready()`

- 无参。
- 含义：后台 `core_bootstrap` 完成、`NodeHandle` 就绪。
- 时机：就绪后发一次。
- ⚠️ Godot 在收到 `kernel_ready` 前不应调 `send_command`（未就绪会返回 `false`）。

### 2. `robot_frame(data: PackedByteArray)`

- 含义：**原始 ORION 帧**（msgid 1/2/3 = POSE / MAP_FULL / MAP_DELTA），桥不解析。
- 来源：`robot_bus`（gossipsub 收到的遥测帧，`StreamRaw` 原样透传）。
- 时机：事件驱动（车端 pose 约 10Hz、map_delta 约 1Hz）。
- Godot 侧用现有 `MessageParser.parse_orion_frame(data)` 解码，按 `msgid` 分发（与现在 WS 的 `_read_packets` 完全一致）。
- 帧头 `sysid` = 发送方完整 peer_id 原始字节（车辆身份来源）。

### 3. `peer_discovered(peer_id: String)`

### 4. `peer_left(peer_id: String)`

- 来源：`event_bus` State `type = peer_discovered / peer_left`（mDNS 发现 / mDNS 过期）。
- 时机：事件驱动。
- `peer_id`：**hex**。
- 语义：发现层事件——`peer_discovered` 在连接建立**前**触发；`peer_left` 是 mDNS 缓存过期（最慢约 2 分钟），供显示"发现中/连接中"等过渡态。

### 5. `peer_connected(peer_id: String)`

### 6. `peer_disconnected(peer_id: String)`

- 来源：`event_bus` State `type = peer_connected / peer_disconnected`（TCP 连接建立/断开）。
- 时机：事件驱动。
- `peer_id`：**hex**。

### 7. `peer_info_updated(peer_id: String, peer_name: String)`

- 来源：`event_bus` State `type = peer_info_updated`（peer-info gossip）。
- 时机：事件驱动（节点上报名字后）。
- `peer_id`：hex；`peer_name`：节点名（车名）。

---

## 三、event_bus 原始事件与转发情况

`event_bus` 的 State JSON 有 `type` 字段，桥转发其中 5 种：

| `type` | 是否转发 | 目标信号 | 说明 |
|---|---|---|---|
| `peer_discovered` | ✅ | `peer_discovered` | mDNS 发现 |
| `peer_left` | ✅ | `peer_left` | mDNS 过期 |
| `peer_connected` | ✅ | `peer_connected` | 连接建立 |
| `peer_disconnected` | ✅ | `peer_disconnected` | 连接断开 |
| `peer_info_updated` | ✅ | `peer_info_updated` | 节点名（gossip） |

> `event_bus` 里还有 ML_review 侧的 models / sessions 等 State 事件，均与机器人无关、不转发。

---

## 四、peer_id 编码（统一为 hex）

所有信号的 `peer_id` 统一为 **hex-of-bytes**（完整 peer_id 原始字节的 hex 编码，如 `00e9…`）。

- `event_bus` 里的 `peer_id` 原本是 base58（`PeerId::to_string()`），桥在 `event_loop` 里统一转成 hex（base58 → `PeerId` → `to_bytes()` → hex）再转发。
- `robot_frame` 帧头 `sysid` 本身就是原始字节，Godot 侧 hex 编码后与 peer 事件一致。
- 因此 peer 事件、`robot_frame`（sysid）、`send_command` 的 peer_id 编码一致，Godot 侧无需再做 base58 ↔ hex 转换。

---

## 五、下行接口（Godot → Rust）

| handle | 参数 | 返回 | 说明 |
|---|---|---|---|
| `send_command` | `peer_id: String(hex), frame: PackedByteArray` | `bool` | 下发 ORION 完整帧（Godot 拼帧），fire-and-forget |
| `poll` | — | — | 每帧调用，排空队列并 emit 信号 |

- `send_command` 的 `frame` 是**完整 ORION 帧**（magic/len/sysid/compid/msgid/payload/checksum），由 Godot 侧（Pictor 现有 `orion_frame` / `orion_messages` 编码）构造。
- `send_command` 的 `peer_id` 是 **hex**。

---

## 六、Godot 侧接入要点（供参考）

1. `_process` 里每帧调 `kernel.poll()`。
2. 连接 `kernel_ready` → 之后才可发命令。
3. 连接 `robot_frame` → 用现有 `MessageParser.parse_orion_frame` 解码 → 按 msgid 分发（pose 更新 sprite / map_delta 累加渲染）。
4. 连接 `peer_*` 系列维护节点列表。
5. peer_id 已统一为 hex（见第四节），无需编码转换。
