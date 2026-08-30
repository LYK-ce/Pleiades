# task_22_2_lua_state_machine — 车决策状态机回迁 Lua

> 状态：方案已定稿（2026-08-20），**尚未开始实施**
> Created Date ： 2026-08-20
> Modified Date ： 2026-08-20
> 依赖：`Task/task_22_universal_robot.md`（步骤 5+6 车脚本迁移已完成）
> 关联设计文档：`docs/design_doc/universal_robot_design.md`
> 分支：`Universal-Robot`（稳定分支 `Pleiades-Orion` / `Godot-Library` 不动）

---

## 一、目标

修正 task 22 步骤 5+6 引入的「无状态 Lua 决策」导致的车行为不等价问题，**把旧 `executor.rs` 的三态机（Idle / Turning / Moving）以 Lua 脚本形式请回来**，恢复车行为与稳定分支完全一致。

核心结论：**决策状态机放 Lua，任务生命周期继续留 Rust**——Rust 管「任务到没到终点」，Lua 管「这一格怎么走过去」。

## 二、背景

task 22 步骤 5+6 用「无状态 Lua 决策」（`programs/robot/car.lua`，每 50ms 全量重算）替换了旧 executor 三态机。Code review 发现 4 个行为不等价：

| # | 问题 | 严重度 |
|---|------|:---:|
| 1 | **转向前不 stop**：`FUNC_CAR_RUN` 是持续模式，直行中直接切 `SpinLeft/SpinRight` 会带前进速度原地旋转（打滑/漂移/失控） | 🔴 致命 |
| 2 | 丢 `sub_target_threshold_m = 0.2`：旧逻辑距格心 0.2m 内才换目标，现改成跨格边界才换 | 🟠 |
| 3 | 丢 `straight_align_threshold_deg = 10°`：现只用 5° 阈值，5°~10° 区间旧直行新转向，车更"抖" | 🟠 |
| 4 | 命令速率从「每格一次」变「20 次/秒」重发 | 🟡 |

根因：**无状态决策无法表达「先停再转」这类跨 tick 的时序动作**——时序动作天然需要记忆（上一次是什么动作、停了多久）。因此需要把状态机请回来；但放回 Lua 决策层（而非 Rust），以保留 task 22 的通用化收益（车/机各写各的状态机，共用同一套 caps）。

## 三、范围

### 本次要做

1. `car.lua` 从「无状态」重写为「三态机」（Idle / Turning / Moving）
2. 恢复旧 executor 的阈值参数：`sub_target_threshold_m=0.2`、`turn_align_threshold_deg=5°`、`straight_align_threshold_deg=10°`、`turn_speed=10`、`move_speed=30`
3. 命令去重：只在状态切换点发命令（恢复「每格一次」）
4. 转向前先停 + 隔 tick（stop 与 turn 之间隔一个 50ms tick）
5. 同步更新 `universal_robot_design.md` 与 `wb_22_universal_robot.md`：「无状态决策」改口为「有状态、状态在 Lua」
6. 修复急停后 `sub_target` 未清空的回归：急停时意图广播应为「无意图」（`sub_target = None`，见 §6 步骤 2）

### 本次不做

- **不改 Rust caps**：`get_path` / `GoalService` / `World` / `register_robot_caps` 接口保持不变
- **不改 STM32Device**：不做设备层「切换即停」——状态机在 Lua 已保证转向前先停，设备层不重复做
- 不拆 `GoalService`（保留其「完整目标服务」封装）
- 机脚本 / `MavlinkDevice`（仍为后续 task）
- 急停（`check_emergency_stop`）继续留 Rust，不下放 Lua
- **急停解除后 Lua 状态冻结 → 卡死**：默认地图内无动态障碍闯入，暂不处理（见 §8 风险 2）

## 四、涉及文件

### 4.1 主要改动

| 文件 | 改动 |
|---|---|
| `programs/robot/car.lua` | 无状态 → 三态机（Idle/Turning/Moving + 阈值常量 + 命令去重） |
| `Src/Robot/core/robot.rs` | `main_loop` 急停时意图广播清空（`sub_target = None`） |
| `Src/Robot/core/goal.rs` | 可选：加 `clear_sub_target`（只清 `sub_target`、保留 `goal`），急停时调用 |

### 4.2 不动

| 文件 | 说明 |
|---|---|
| `Src/Robot/world.rs` | `World::get_path` 不变 |
| `Src/VM/capability_binding.rs` | caps 接口不变 |
| `Src/Robot/control/device/stm32/mod.rs` | 不动 |

## 五、设计

### 5.1 分工边界

| 层级 | 职责 |
|------|------|
| **Rust（少量改动）** | ① `GoalService::get_path`：任务生命周期（到终点检测 + pop mission + 群发分配 + D* 寻路），返回「下一格 `{gx,gy}`」或 `nil`；② `check_emergency_stop`：急停安全兜底；③ caps：`self.get_position/attitude/velocity`、`world.get_cell/agents/path`、`action.*`；④ `state_notifier`：位姿/意图广播（100ms，读 `execute_state.sub_target` 组 `PoseData` 广播） |
| **Lua（car.lua）** | 三态机（Idle/Turning/Moving）：拿格 → 算角偏差 → 转向（先停再转）/ 直行 / 0.2m 换格 / 直行连续化；命令去重 |

**衔接点**：Lua 每次调 `world.get_path()` 拿「下一格」或 `nil`。`nil` = 无任务 / 已到终点 / 不可达 → Lua 只 `stop()`，**完全不碰 `mission_queue`、不 pop 任务**。

**广播边界**：意图/位姿广播全在 Rust 侧（`GoalService.sub_target` → `main_loop` 同步 → `execute_state` → `state_notifier` 组帧 → `Gossipsub_Publish`）。Lua 只通过 `get_path` 间接影响 `GoalService.sub_target`，不直接参与广播。

### 5.2 Lua 三态机流程

Lua 顶层 `local`（upvalue，跨 tick 持久）：`state`、`sub_target`（正走向的格）+ 常量（`CELL_RESOLUTION=0.5` 等）。

`on_tick()` 每 50ms 被 Rust 调用：

```
Idle（停下思考）
  next = world.get_path()          ← Rust 内部完成到终点检测/pop/寻路
  next == nil → action.stop()，保持 Idle，return
  sub_target = next
  delta = 下一格方向 - yaw（归一化）
  |delta| > 5°  → 发 turn_left/right，state = Turning
  否则          → 发 move_forward，state = Moving

Turning（旋转对齐）
  delta = sub_target 方向 - yaw
  |delta| ≤ 5° → action.stop()，state = Idle
  否则保持 Turning（不重复发 turn）

Moving（直行）
  dist = 距 sub_target 格中心距离（get_position + 格中心坐标）
  dist < 0.2m（到达当前格）：
    next = world.get_path()        ← 前瞻：此时车 floor 已在 sub_target 格，返回下一格
    next == nil → action.stop()，state = Idle，return
    next 方向 - yaw：
      < 10°  → 直行连续化：sub_target = next，保持 Moving（不停车）
      ≥ 10°  → action.stop()，state = Idle   ← 下一 tick Idle 才转向（天然隔 50ms）
  否则（没到 0.2m）→ 不发命令，继续直行
```

### 5.3 关键设计点

1. **Lua 状态跨 tick 持久**：`car.lua` 顶层 `local`（upvalue）在同一个 `LuaContext` 的多次 `on_tick()` 调用间持久，这是状态机能工作的前提。实施前须确认 `spawn_decision_thread` 是**单一 context 反复调 `on_tick`**（现状是），并保持不破坏。
2. **命令去重**：只在状态切换点发命令（Idle→发 turn/forward，Turning 对齐→发 stop，Moving 到达需转→发 stop）。Moving 直行中、Turning 旋转中都不重复发。
3. **转向前先停 + 隔 tick**：Moving 到达 0.2m 且需转向时 → `stop` 回 Idle；下一 tick（50ms 后）Idle 才发 turn。等价于旧 executor 的时序（stop 与 turn 之间隔一个 tick，车有 50ms 刹停）。
4. **0.2m 距离判定在 Lua**：格中心世界坐标 = `((gx+0.5)*CELL_RESOLUTION, (gy+0.5)*CELL_RESOLUTION)`，用 `self.get_position()` 算欧氏距离。几何逻辑进 Lua，代价约几行，可接受。
5. **sub_target 双份状态**：Lua 的 `sub_target`（决策：正走向哪格）与 Rust `GoalService` 的 `sub_target`（意图广播 `sub_gx/sub_gy`）是两份，通过 `get_path` 返回值同步。正常流程一致，但更新时机有短暂差异（Lua 到达 0.2m 才更新 vs Rust 每次调用更新），与旧 executor 的「executor.sub_target vs execute_state.sub_target」分离类似，可接受但需知晓。

## 六、详细实施步骤

### 步骤 1：确认 Lua upvalue 持久前提

阅读 `Src/Robot/core/robot.rs::spawn_decision_thread`，确认：单一 `LuaContext` 加载 `car.lua` 后，每 tick 反复调用同一 `on_tick`（upvalue 不重建）。若现状不满足，先修正。

### 步骤 2：修复急停后 `sub_target` 未清空（回归）

task 22 步骤 5 把急停从 executor 抽到 `check_emergency_stop` 时，漏掉了旧 executor 急停时的 `sub_target = None`。现状：急停时只 `stop + mark_obstacle`，`GoalService.sub_target` 保持急停前值 → 意图广播仍发急停前的旧格子（`valid=true`），语义错误（车已停却仍广播「想走到 B 格」）。

修复：`main_loop` 急停时（`emergency == true`）意图广播清空为 `None`；更彻底可给 `GoalService` 加 `clear_sub_target`（只清 `sub_target`、保留 `goal`，急停时调用，障碍消失后 `get_path` 重新绕行）。

### 步骤 3：写 `car.lua` 三态机

按 §5.2 流程重写 `car.lua`，阈值常量照旧 executor 默认值硬编码（或注释标注来源）。

### 步骤 4：编译 + caps 通路验证

`./build.sh check` 通过；先用手动测试脚本验证 `self.get_position` / `world.get_path` / `action.*` 通路正常。

### 步骤 5：实车验证行为等价性（关键）

在 `Universal-Robot` 分支上验证「走格子 / 转向 / 直行 / 到达 / 急停」与稳定分支一致。重点：转向前停车、直行连续化、命令去重。

### 步骤 6：文档同步

更新 `universal_robot_design.md`（「无状态决策」→「有状态、状态在 Lua」）、`wb_22_universal_robot.md`（补记本任务决策与结论）。

## 七、验证方式

| 验证项 | 方式 |
|---|---|
| 编译 | `./build.sh check` 通过 |
| caps 通路 | 手动测试脚本：`get_position` / `get_path` / `action.*` 可用 |
| 转向前先停 | 实车：直行中遇拐点，先停（≥50ms）再原地转，无带速度打滑 |
| 直行连续化 | 实车：连续直行多格（<10°）不停车 |
| 命令去重 | 日志/串口：每格只发 1 次 turn / forward，非 20 次/秒 |
| 急停 | 实车：前方 <0.3m 强制停 + 意图广播清空（`valid=false`） |

## 八、风险与注意

1. **行为一致性是安全底线**：本任务本质是对 task 22 步骤 5+6 的修正，迁移后「走格子、转向、直行、到达、急停」必须与稳定分支完全一致，这是合入前提。
2. **急停后 Lua 状态冻结 → 卡死（已知限制，暂不处理）**：急停时 `main_loop` 跳过 `on_tick`，三态机的 `state`/`sub_target` 冻结在急停前状态；障碍消失后若 Lua 仍在 Moving 态且 `dist > 0.2m`，会「继续等到达」而车已被 stop 停住 → 卡死。默认地图内无动态障碍闯入，暂不处理；若未来需支持动态障碍，须把「急停时跳过 `on_tick`」改为「急停时通知 Lua 重置状态」（tick 通道带事件类型，发 `Reset`）。
3. **急停意图广播必须清空**：急停时 `sub_target` 应广播 `None`（旧 executor 行为），否则其他车会误判本车仍朝旧格移动。§6 步骤 2 修复。
4. **文档同步**：本任务推翻了 `universal_robot_design.md` 中「砍掉 Idle/Turning/Moving 状态机、无状态决策」的既定目标，设计文档与 workbook 必须同步改口，否则后人会困惑「为何状态机又回来了」。
5. **sub_target 双份状态**：Lua 与 Rust 各持一份 `sub_target`，正常一致但更新时机有差，后续改动 `get_path` 语义时需留意。
6. **每次改动小步提交**：每个步骤单独 commit，便于回滚。
