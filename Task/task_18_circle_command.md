# Task 18: Circle 命令（环形散布）

> 创建日期：2026-08-16
> 状态：已实施（代码 + 单测通过，2026-08-16）
> 范围：新增 `Mission::Circle` 命令——多车围绕目标点环形散布（第一版到达即停、不朝圆心）

---

## 一、背景

系统目前只有一种自动任务命令 `Mission::Goto`（群发棋盘散布）。`docs/design_doc/multi_robot_control.md` §4.3 曾规划「围堵」环形分布（θ=i/N·2π），但因半径来源等设计点未决，一直停留在文档阶段。现立项实现。

## 二、命令语义

- `Mission::Circle { x, y, members }` —— 与 `Goto` 结构完全一致：`x/y` 为圆心（世界坐标，米），`members` 为成员列表（非空=群发）。
- 每辆车按 peer_id 字节升序排序取序号，在环上**均匀铺开**算出各自目标位置 → 零通信、零协商、零冲突（与 Goto 同构）。
- 第一版：到达即停，**不做朝向圆心**（复用 executor 现有「到 goal 后 stop」行为）。

## 三、方案设计

### A. 环形候选格（复用 `ring_cells`）

| 决策点 | 定稿 |
|---|---|
| 环形状 | 方环（Chebyshev 环），**复用 `ring_cells(r)`**，非真圆 |
| 半径 | **写死 0.5m = 圆心格与车格之间隔 1 格**，不进协议；常量 `CIRCLE_RING_RADIUS_CELLS: i32 = 2`（车在切比雪夫距离 2 的环上） |
| 候选格 | `ring_cells(2)` = 16 格（圆心外隔 1 格的环），固定顺序「正上方起顺时针」 |
| 起始角/方向 | 复用 `ring_cells` 现有固定顺序（正上方 (0,-r) 起、顺时针），不新增定义 |
| 棋盘奇偶过滤 | **不加**（环上已有间距，且要保持「圈」形） |
| 环满 | 暂不考虑（≤4 车 < 16 格）；代码仍保留 `InsufficientSlots` 防御 |

`ring_cells(2)` 顺序（相对圆心偏移，共 16 格）：`(0,-2) (1,-2) (2,-2) (2,-1) (2,0) (2,1) (2,2) (1,2) (0,2) (-1,2) (-2,2) (-2,1) (-2,0) (-2,-1) (-2,-2) (-1,-2)`，即正北起顺时针一周；**第一格 = (0,-2)**。

### B. 均匀铺开分配

**`build_circle_slots(center, grid)`**（与 `build_slots` 同构的「全局压缩」）：

1. `(cx, cy) = world_to_grid(center)`；越界 → `OutOfBounds`。
2. （圆心格 Occupied **不判失败**——circle 语义是「围住」，圆心本身是障碍也合法。）
3. 枚举 `ring_cells(CIRCLE_RING_RADIUS_CELLS)` → 16 格，过滤 越界 / Occupied → `available: Vec<(f32,f32)>`（格中心世界坐标，保持固定顺序）。

**`group_circle_mission(center, members, own_peer_id, grid)`**：

```rust
let available = build_circle_slots(center, grid)?;
if members.is_empty() {
    // 单车：环的第一个位置
    return available.first().copied().ok_or(AssignError::InsufficientSlots);
}
let mut sorted: Vec<&Vec<u8>> = members.iter().collect();
sorted.sort();
let idx = sorted.iter().position(|m| m.as_slice() == own_peer_id).ok_or(AssignError::NotMember)?;
let n = sorted.len();
if n > available.len() { return Err(AssignError::InsufficientSlots); }
// 均匀铺开：整数除法，确定性
available.get(idx * available.len() / n).copied().ok_or(AssignError::InsufficientSlots)
```

**数值例子**（圆心格 (128,128)，16 格可用，圆心与车之间隔 1 格）：

| N | idx | 落点（方向） |
|---|---|---|
| 1（单车） | 第一个位置 | 北 (128,126) |
| 2 | 0, 8 | 北 (128,126)、南 (128,130) —— 直径两端 |
| 3 | 0, 5, 10 | 北 (128,126)、东南 (130,129)、西南 (126,130) —— 近似 120° |
| 4 | 0, 4, 8, 12 | 北 (128,126)、东 (130,128)、南 (128,130)、西 (126,128) —— 四向 |

被占格先被压缩掉（顺延），再在剩余 `available` 上均匀铺开——与 Goto「跳过 Occupied → 紧凑列表 → 按序号取」同一套机制。

### C. 协议层（改动最小）

- `MissionItem` **保持 9 字节不变**（半径不进协议）。
- 新增 `pub const MISSION_CIRCLE: u8 = 1;`（`x/y` 复用为圆心）。
- `command_decode.rs` 三分支：
  - `member_count==1`：`MISSION_CIRCLE → Mission::Circle { x, y, members: vec![] }`
  - `>1`：找「第一个 Goto 或 Circle」，按 type 构造并透传 members。

## 四、文件改动清单

| # | 文件 | 改动 |
|---|---|---|
| 1 | `Src/Robot/core/command.rs` | `Mission` 加 `Circle { x: f32, y: f32, members: Vec<Vec<u8>> }` |
| 2 | `Src/Robot/core/protocol/messages.rs` | 加 `MISSION_CIRCLE: u8 = 1` |
| 3 | `Src/Robot/core/protocol/command_decode.rs` | 单/群发分支识别 Circle |
| 4 | `Src/Robot/core/planning/assignment.rs` | 加 `CIRCLE_RING_RADIUS_CELLS`、`build_circle_slots`、`group_circle_mission` |
| 5 | `Src/Robot/core/executor.rs` | pop 分支加 `Mission::Circle` arm（与 Goto 同构） |
| 6 | `Src/Robot/core/protocol/mod.rs` | re-export `MISSION_CIRCLE` |
| 7 | 文档 | `orion_protocol.md` §3.5、`multi_robot_control.md` §4.3/4.4、`Architecture/Pleiades_Architecture.md` §3.10 |

> `slam/grid.rs`、`pathfinder.rs` 不改。

## 五、实施步骤

1. `command.rs` 加枚举变体。
2. `messages.rs` 加常量 + `mod.rs` re-export。
3. `command_decode.rs` 识别 Circle（单/群发）。
4. `assignment.rs` 实现 `build_circle_slots` / `group_circle_mission`（复用 `ring_cells`、`cell_center_world`、`AssignError`）。
5. `executor.rs` 加 Circle arm（复用 D* 装载 + 到达即停）。
6. 单测 + `cargo build --bin orion-robot` + `cargo test --lib robot::`。
7. 文档同步 + `deploy_robot.sh` 部署实车联调。

## 六、测试

| 用例 | 输入 | 期望 |
|---|---|---|
| 单车 | members 空 | 落环第一个位置（北格 (128,126) 中心） |
| 两车 | members=[a,b] | 北 (128,126)、南 (128,130)（直径两端，对角） |
| 三车 | members=[a,b,c] | 三落点两两互异、均匀 |
| 四车 | members=[a,b,c,d] | 北东南西四向 |
| 被占顺延 | 环上某格 Occupied | 该格被跳过，剩余仍均匀、无冲突 |
| 圆心越界 | center 越界 | `OutOfBounds` |
| 圆心 Occupied | 圆心格障碍 | 不失败（围住语义），环格正常分配 |
| NotMember | own 不在 members | `NotMember` |
| InsufficientSlots | available < N（防御） | `InsufficientSlots` |
| 确定性 | 同输入两次 | 结果一致 |
| 协议 roundtrip | `MISSION_CIRCLE` 往返 | type/x/y 保持，9 字节布局不变 |

**实车联调（后续）**：4 车围圈，验证落点、无碰撞、到达即停不转车头。
