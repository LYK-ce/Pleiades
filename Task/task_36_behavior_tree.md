# task_36_behavior_tree — 任务层改造：命令队列 + 行为树

> Presented by KeJi
> Created Date ： 2026-10-26
> Modified Date ： 2026-10-27
> 状态：**Step 1~8 已定稿、待执行**（设计结论见 §三 + §十一；未决问题见 §七；实施计划见 §十二）
> 关联文档：`docs/design_doc/orion_protocol.md`（§3.5 `ORION_TASK_SET`）、`Task/archived task/task_22_universal_robot.md`（任务执行框架起点）、`Task/archived task/task_23_universal_robot.md`（base 骨架 + `DeviceHandler`）、`Architecture/robot_arch.md`、`Task/task_35_pedestrian.md`（头部格式参照）

---

## 一、背景与动机

### 1.1 现在任务层长什么样（"跑起来"阶段）

| 环节 | 现状 | 依据 |
|---|---|---|
| 命令入口 | `ORION_TASK_SET`（msgid 5）→ `command_consumer` → `parse_orion_frame` → `Command::Auto(AutoCmd::Set(Vec<Mission>))` | `pleiades-base/src/Robot/core/robot.rs:312-320`、`core/protocol/command_decode.rs:27-70` |
| 队列 | `MissionQueue` = `Vec<Mission>`，方法只有 push / pop_next / clear / replace（另有 peek / len / is_empty 三个**全仓无调用者**） | `core/mission.rs:11-13`、`:17-55` |
| 任务种类 | 只有 `Goto{x,y,members}` / `Circle{x,y,members}`，载荷只有坐标 + 群发名单 | `core/command.rs:52-64` |
| 任务执行 | 每 50 ms：`device.on_tick()` → `GoalService::get_path()`（① 到达检测 → ② pop 任务 → ③ 格级到达 → ④ 寻路）→ `DecisionExecutor::decide()` → 下发动作 | `robot.rs:280-281`、`:335-343`；`pleiades-ugv/src/ugv/goal.rs:69-164`；`ugv/robot_handler.rs:217-244` |
| 执行层 | `Idle / Turning / Moving` 三状态机，输出 `MoveForward / TurnLeft / TurnRight / Stop` 意图 | `core/state.rs:94-110`；`ugv/executor.rs:38-51` |
| 生命周期 | 任务 `pop_next()` 出队即离开队列，执行到"到达 / 不可达"为止 | `ugv/goal.rs:88`、`:76-84`、`:157-162` |

一句话概括：**队列的每个元素直接映射成一段执行逻辑** —— `get_path()` 里 ①②③④ 是固定顺序，任务类型靠 `match mission { Goto => …, Circle => … }` 展开，**"任务类型"和"执行流程"是同一段代码**。

### 1.2 三个具体局限

**(1) 任务一次性**：Mission 只有坐标，没有时限、没有循环、没有结束条件。

- `command.rs:52-64`：字段只有 `x / y / members`，无 duration、无次数、无终止条件；
- `mission.rs:11-13`：队列元素是裸 `Mission`，无时间戳、无运行态；
- `goal.rs:76-84`：唯一的结束判据是"世界距离 < `ARRIVAL_THRESHOLD_M`"（`goal.rs:27`，0.3 m）。

→ "围控 2 小时后返航"在现有类型里**无法表达**：只能在队列里放两条任务，而第一条（Circle）没有"2 小时后结束"的出口，永远停不下来（`Circle` 当前语义是"到达即停、不朝圆心"，`command.rs:58-62`）。

**(2) 失败即丢弃**：任务先出队，再判断能不能做。

- `ugv/goal.rs:88`：`pop_next()` —— 任务在这一行**已经离开队列**；
- `ugv/goal.rs:96-101`（群发 Goto 分配失败）、`:112-116`（群发 Circle 同理）、`:157-162`（D* 不可达）三处都是 `warn!` + 清 `goal`/`pathfinder` + `return None`；
- sim 同构：`sim/goal.rs:215`、`:229`、`:287-289`。

→ 日志之后任务**永久消失**：没有重试、没有回执、队列里没有痕迹。地面站只能从位姿"不动了"反推失败。

**(3) 更新即重建**：任何 `TASK_SET` 都是整队替换 + 硬重置。

- `robot.rs:315-319`：`mission_queue.replace(list)` + `device.reset()`；
- `ugv/goal.rs:167-170`：`reset()` 清 `goal` + 丢 `pathfinder`（当前路径全丢）；
- `robot_handler.rs:210-215`：停车 + 写 `ExecuteState{ Idle, None }`。

→ 改一个参数（"目标点偏 2 m"）必须重发整队；进度、已走路径、重试计数一律作废。这是当前 `replace` 的**唯一**语义（`mission.rs:37-40`、`command.rs:44-48`）。

**(4) 附带的结构性观察**（不是独立局限，是"新命令 = 新树"的动机）：加一种任务要同时改四处 —— `Mission` 变体（`command.rs:52-64`）、协议 mission_type（`messages.rs:257-259`）、解码分支（`command_decode.rs:36-44`）、`get_path()` 的 match（`goal.rs:89-122`），且四处的失败语义各自独立。

---

## 二、目标与非目标

### 2.1 目标

1. 任务层从"队列元素 → 一段执行流程"改为"**队列 + 行为树**"：队列保留，树承接任务内部的控制流（顺序、条件、超时、重试、恢复）。
2. 每类命令一棵树（`Goto` 一棵、`Circle/Encircle` 一棵），**新命令 = 新树**，不再往 `get_path()` 里加分支。
3. 树内失败由恢复结构（Fallback + 等待重试 + 上报）承接，消除"不可达静默丢任务"。
4. 长任务用"带终止条件"表达：队列中每一项都要有明确的终止途径（到达 / 失败 / 超时 / **被覆盖**）。
5. 执行层（三状态机 + 前进/转向/停止输出）**保持不变**，树只改变"谁驱动它、按什么顺序驱动"。
6. 树可被单测：三值语义可断言，不依赖串口 / 网络。

### 2.2 非目标（硬边界）

| 边界 | 内容 | 核对结论 | 依据 |
|---|---|---|---|
| 多机协同 / 分工 | 谁去哪、谁围谁、编队 | **上移到指挥层（地面站）**（§十一 #27）：车端删除 `assignment`，只保留 `cluster_obstacles` 的单车避碰；`core/cluster/*` 数据面不动 | `core/cluster/`（4 文件）；`assignment.rs:121`、`:171` |
| 感知 / 检测 | YOLO、相机、雷达驱动、建图 | 不做 | `ugv/lidar/`、`sim/sensors/`、`Task/archived task/task_29_yolo.md` |
| 串口协议 | STM32 车控帧、MAVLink | 不改 | `ugv/stm32/mod.rs:128-175`（stop / apply_action）、`uav/mavlink/` |
| ORION 帧格式 | `frame.rs` 帧头 | **帧头不改**；`TASK_SET` **载荷升版**：去掉 `member_count` / `members`（§十一 #28，Step 3） | `core/protocol/frame.rs`、`messages.rs:52-57` |
| 地面站 | Pictor（Godot） | **仓外**，不在本仓库；仓内只有 `pleiades-terminal/` 的 GDExtension 哑管道桥，不改 | `pleiades-terminal/src/lib.rs:6-10`（"桥是哑管道，不解析、不合并业务数据"） |
| 寻路 | D* Lite 算法 | 不改 | `planning/pathfinder.rs`（ugv/uav 401 行、sim 706 行） |
| 执行器状态机 | Idle / Turning / Moving 规则 | 不改 | `ugv/executor.rs:38-180` |
| 急停 | 前方障碍强制 stop | 不改，且**不进行为树**（结论 9） | `ugv/emergency_stop.rs:25`、`sim_handler.rs:456-472`、`robot_handler.rs:224-231` |

### 2.3 待确认的边界

- `Mission` 是否新增变体（如 `Patrol` / 带时限 `Encircle`）：[待确认]，取决于 §七 第 5、6 条（树种类 / 协议升版）。
- ~~树代码放 `pleiades-base` 还是三处设备 crate~~ → ✅ **已定**（§十一 #19）：**引擎在 base 一份，树结构与叶子在设备端各一份**。

---

## 三、已定结论（讨论结果，原样记录）

> 注：#1「多机协同不做」与 #5「队列语义未定稿」两条已被后续讨论更新——见 §十一 #27（协同上移）与 #15~#17（队列三语义）。本节其余内容保持原样。

| # | 结论 | 要点 |
|---|---|---|
| 1 | **单车 / 单机视角** | 行为树只管自己这一台设备，**不感知、不参与多机协同**；多机分工不在本任务范围内 |
| 2 | **三层结构** | 顶层 = 命令队列（保留现状，"来一条命令、执行一条"）；中层 = 行为树（新增，负责任务内部：顺序 / 条件 / 超时 / 重试 / 恢复）；底层 = 执行状态机（**保留不动**，即 Idle/Turning/Moving 与前进/转向/停止输出） |
| 3 | **每类命令一棵树** | `Goto` 一棵、`Circle/Encircle` 一棵，将来 `Patrol` 等再各加一棵；**新命令 = 新树** |
| 4 | **树写在 Rust 代码里** | 代码定义树，不用 XML、不做运行时热加载；可接受"改树要重新编译" |
| 5 | **队列语义** | 保留队列并新增语义；`replace` 明确为**"立刻打断当前任务并更新"**（不恢复旧任务）。`append`（入队尾）/ `preempt`（插队头）/ `cancel` / `update`（原地改参数）在讨论中出现过但**尚未定稿** → §七 |
| 6 | **长任务用"带终止条件"表达** | 例："围控 2 小时后返航" = 带时限 / 结束条件的循环，**不是永不结束的循环**；队列中**每一项都必须有结束条件**（到达 / 超时 / 完成 / 被覆盖） |
| 7 | **叶子是"可完成"的动作** | `GoTo` / `Hold` / `Encircle` / `Patrol` / `Track` / `Stop` 这一级；**前进 / 转向 / 停止这类连续输出不进行为树**，仍在执行层产生 |
| 8 | **三值语义** | 每个节点每 tick 返回 `SUCCESS / FAILURE / RUNNING`；`RUNNING` 不阻塞；被抢占的分支必须能 `halt`（停动作 + 清状态） |
| 9 | **安全逻辑在树外** | 急停等安全机制不通过队列、不通过树（本条只作边界声明，不在本任务细化） |
| 10 | **失败处理** | 不可达 / 失败不应再"静默丢弃任务"（现状如此，见 §1.2(2)），应由树内恢复结构（Fallback + 等待重试 + 上报）承接 —— 这是方案的动机之一 |

---

## 四、方案描述

### 4.1 三层数据流

```text
  地面站 Pictor（仓外，不在本仓库）
        │  ORION_TASK_SET (msgid 5, 9 B/mission item)
        ▼
  command_consumer ──▶ parse_orion_frame ──▶ Command::Auto(AutoCmd::Set(Vec<Mission>))
                                                     │
                                                     ▼
                                    ┌────────────────────────────────┐
   main_loop (robot.rs, 50 ms tick) │  ① 命令队列 MissionQueue（保留） │
                                    │     replace=立刻打断并更新        │
                                    └────────────────┬───────────────┘
                                                     │ 取队首 → 按 mission_type 选树
                                                     ▼
                                    ┌────────────────────────────────┐
                                    │  ② 行为树（新增，中层）          │
                                    │     tick() → SUCCESS/FAILURE/   │
                                    │              RUNNING            │
                                    │     halt() → 停动作 + 清状态     │
                                    └────────────────┬───────────────┘
                                                     │ 叶子要"走到某点"
                                                     ▼
                                    ┌────────────────────────────────┐
                                    │  ③ 执行状态机（不动，底层）      │
                                    │     GoalService: 到达/下一格/不可达│
                                    │     DecisionExecutor: Idle/      │
                                    │       Turning/Moving 三状态机    │
                                    └────────────────┬───────────────┘
                                                     │ DecisionResult{state, sub_target, action}
                                                     ▼
                              stm32 / mavlink / chassis：前进 / 转向 / 停止（连续输出）
                                                     ▲
                                    急停（树外，直接停）：emergency_stop → stop
```

要点：
- 地面站只看到"命令 + 位姿"，中间三层对它是透明的；协议不因引入树而改变（除非 §七 第 6 条定案）。
- 急停从 `on_tick` 最前面直接短路（`robot_handler.rs:224-231`、`sim_handler.rs:459-472`），**不经过队列、不经过树**（结论 9）。

### 4.2 队列 → 树 的切换关系

今天"没有树"，所以下表实际是**接线点**：每个事件今天在哪、引入树后改成什么。

| 事件 | 今天 | 改成 |
|---|---|---|
| `TASK_SET`（`member_count ≥ 1`）| `replace(list)` + `device.reset()`（`robot.rs:315-319`）| 先 `halt` 当前树（停动作 + 清状态）→ 队列替换 → 下一 tick 取队首建树。**`replace` = 立刻打断并更新，不恢复旧任务**（结论 5） |
| `TASK_SET`（`member_count = 0`，取消）| `replace(vec![])` + reset（`command_decode.rs:30-31`、`robot.rs:315-319`）| `halt` 树 + 清队 → 无树、待命 |
| 当前树 = None 且队列非空 | `pop_next()` 直接取走（`goal.rs:88`）| 取队首（**peek 语义**，`mission.rs:48-50` 已存在但全仓无调用）→ 选树 → 建树；出队时机推迟到"树给出 SUCCESS/FAILURE 之后" |
| 树返回 `SUCCESS` | 无对应物（"到达" = 清 `goal`，`goal.rs:76-84`）| `halt` 清理 → 出队 → 取下一个 |
| 树返回 `FAILURE` | 无对应物（静默丢弃，§1.2(2)）| 恢复结构已用尽 → 上报 + 出队（或停住待命）[待确认，§七 第 2 条] |
| 树返回 `RUNNING` | 无对应物 | 保持，下一 tick 继续 tick（不阻塞主循环，结论 8）|
| 切到 `Manual` 模式 | `clear()` + reset（`robot.rs:289-294`）| `halt` 树 + `clear()` + reset |

### 4.3 一棵树内部的组成

| 节点类别 | 语义 | 本任务中的实例 |
|---|---|---|
| 控制节点 | 组合子节点，按各自规则返回三值。**记忆式与反应式各一份**（§十一 #24） | `Sequence` / `ReactiveSequence`（顺序）、`Fallback` / `ReactiveFallback`（失败换路 + 优先级抢占，结论 10 的恢复入口）|
| 装饰器 | 单子节点 + 修饰 | `Timeout`（结论 6 的时限）、`Retry`（结论 10 的重试）、`Repeat` / `RepeatForever`、`Guard` / `Inverter` |
| 条件节点 | 只读观测，瞬时返回 | `Arrived`、`HasGoal`、`TimeoutExpired` |
| 动作叶子 | **可完成**的动作（结论 7），每 tick 推进直到给出终值 | `GoTo` / `Hold` / `Encircle` / `Patrol` / `Track` / `Stop` |

**Goto 树结构示意**（伪树，非实现代码；参数值均待定）：

```text
Goto(x, y)                                ← 每类命令一棵，根 = Sequence
└─ Sequence
   ├─ Condition  HasGoal(x, y)            瞬时：坐标合法 → SUCCESS，否则 FAILURE
   ├─ Decorator  Timeout(T)               超过 T 仍未完成 → FAILURE（结论 6：必须有结束条件）
   │  └─ Fallback                         ← 恢复结构（结论 10）
   │     ├─ Sequence                      主路：走过去 + 停稳
   │     │  ├─ Action  GoTo(x, y)         叶子：每 tick 问执行层「已到达 / 下一格 / 不可达」
   │     │  │                             → RUNNING / SUCCESS / FAILURE
   │     │  └─ Action  Hold(Δt)           叶子：停稳确认
   │     └─ Sequence                      恢复路（主路失败才走）
   │        ├─ Action  Wait(Δt)           退避等待
   │        └─ Action  GoTo(x, y)         重试（次数由 Retry 装饰器约束）
   └─ Action     Report(FAILURE, reason)  上报（结论 10：不再静默）
```

**tick 与 halt 语义**（结论 8）：
- 每 tick 从根往下走一遍；`RUNNING` 表示"还在做"，**不阻塞**主循环其它分支（tick 语义，非线程阻塞）。
- `halt` 必须做两件事：**停动作**（发 `Stop`，`stm32/mod.rs:128-142`）+ **清状态**（树内节点状态、执行器 `ExecuteState`）。动作不是自动到期的（`apply_action` 只是"发一条记一条"，`stm32/mod.rs:164-175`），所以 halt 必须显式下发 Stop；固件侧是否有动作超时 [待确认]。

### 4.4 树与执行器的接口边界（谁调用谁、每 tick 谁驱动）

| 问题 | 结论 | 依据 |
|---|---|---|
| 谁驱动 tick | `main_loop` 的 50 ms `auto_tick` → `device.on_tick()`（已有）| `robot.rs:280-281`、`:335-343` |
| 树在哪被 tick | 在 `on_tick` 内部，替换现在那一行 `get_path()` 调用 | `ugv/robot_handler.rs:233`、`uav/robot_handler.rs:131`、`sim_handler.rs:475` |
| 树是否直接发动作 | **否**。叶子只表达"要去哪、是否还要继续"，动作仍由 `DecisionExecutor::decide()` 产出，由 device 下发 | `ugv/executor.rs:38-51`、`robot_handler.rs:241` |
| 叶子的输入 | 叶子**自己持有能力端口**直接读（不造观测快照，§十一 #21/#22）；但"到达 / 下一格 / 不可达"必须可区分——今天的 `get_path() -> Option<(i32,i32)>` 把三者都返回 `None`，形式待定（§七 #7）| `ugv/goal.rs:69-164`、`:76-84` vs `:157-162` |
| 叶子的输出 | 三值 `SUCCESS / FAILURE / RUNNING` | 结论 8 |
| 执行器状态谁写 | 今天 `main_loop`/`on_tick` 单点写 `ExecuteState`；引入树后**仍应单点写**（写者不变，只是"由树的结果落到 `ExecuteState`"）| `core/state.rs:125-135`（注释："写者 = main_loop"）|
| 安全 | 急停不进树；树被 halt / 不存在时急停照常生效 | `robot_handler.rs:224-231` |
| 黑板 | ~~形态与归属未定~~ → ✅ **v1 不做**（§十一 #21）；跨拍状态由各节点自己的字段保存 | §七 第 4 条 |

---

## 五、对照当前代码的变更清单

### 5.1 先核对复制关系（本任务最关键的事实）

| 文件 | ugv | uav | sim | md5 一致性 | 核对结论 |
|---|---|---|---|---|---|
| `executor.rs` | 286 行 | 286 行 | 286 行 | **三者逐字节相同**（`e697b6c2…`）| 三份副本，完全相同；**本任务不动它**，三处都不受影响 |
| `goal.rs` | 189 行 | 191 行 | 440 行 | **三处全不同** | **已分叉**：ugv/uav 近同构（uav 唯一差异 = `:139-155` 注释掉动态障碍、改喂空 grid）；sim 是独立演进版（预测停点 `:76-83`、逐轴容差 `:38`、建图增量修补 `:109-127`）|
| `planning/assignment.rs` | 425 行 | 425 行 | 425 行 | 三者逐字节相同 | 三份副本；算法不动，只换调用点 |
| `planning/pathfinder.rs` | 401 行 | 401 行 | 706 行 | ugv = uav，sim 不同 | D* Lite；不动 |
| `planning/cluster_obstacles.rs` | 131 行 | 131 行 | 131 行 | 三者逐字节相同 | 不动 |
| `planning/mod.rs` | 18 行 | 18 行 | 18 行 | 三者逐字节相同 | 仅当树模块放设备端时才改 |
| 设备 handler | `robot_handler.rs` 262 行 | `robot_handler.rs` 154 行 | `sim_handler.rs` 525 行 | 各不同 | 三个 `DeviceHandler` 实现，各自接线 |

**由此得出的三条硬结论**：

1. `executor.rs` 是**逐字节相同的三份**（286 行 / `e697b6c241fec7ee5411c1c56b5c8002`）。方案要求它"保留不动"，所以**执行状态机不是本次改造的波及面**。
2. `goal.rs` **已经不是一套代码**：sim 版修过"按预测停点判到达"（`sim/goal.rs:29-92`，注释里记着实测误差 0.39 m 的 FAIL 案例），ugv/uav 还是"当前位置 < 0.3 m"（`ugv/goal.rs:76-84`）。→ 同一个 `Goto` 在 sim 与真车上到达判据**本来就不同**；树若"只写一份放 base"，这块差异必须显式处理（§七 第 5、8 条），不能悄悄统一。
3. 改动落地在 **3 个设备 crate**（ugv / uav / sim 各一套 `goal.rs` + handler）；若树进 `pleiades-base`，则新增 1 处 + 3 处接线（各 1~3 行）。

### 5.2 `pleiades-base/src/Robot/core/`

| 文件 | 现状职责 | 本任务需要做什么 | 改动程度 | 依据行号 |
|---|---|---|---|---|
| `command.rs` | `Mission` 枚举（Goto / Circle，只有坐标）+ `AutoCmd::Set` | `Mission` **迁出**到 `mission_queue/element.rs`（Step 1），Step 3 收敛为 `Goto { x, y }`；`command.rs` 只剩命令枚举 + re-export | 改 | `:44-48`、`:52-64` |
| `mission.rs` | FIFO `Vec<Mission>`；push / pop_next / clear / replace | **迁入 `mission_queue/` 目录并新增语义**（Step 1，§12.1.3）：`replace` 已定（立刻打断、不恢复）；`append` / `preempt` **已定**、`update` 待定；"取队首但不丢"= `head()`（新的 peek，原 `peek` 无调用者，更名）| 改 | `:11-13`、`:17-19`、`:22-28`、`:31-33`、`:37-40`、`:43-55` |
| `robot.rs` | `main_loop` 单 `select!`：命令分支 + 50 ms `auto_tick` → `device.on_tick()`；`AutoCmd::Set` → `replace` + `reset` | 树的生命周期挂在这里：`replace` 时 `halt` 树、tick 分支驱动树（通过 `DeviceHandler` 接口，倾向于不改 `on_tick` 签名而由设备端内部持有树 → §七 第 8 条）| 改 | `:280-281`、`:289-294`、`:312-320`、`:335-343` |
| `state.rs` | `DecisionState` / `MotionAction` / `DecisionResult` / `ExecuteState`（执行层契约）| **不动**：树的三值语义与执行层状态机是两套概念，不应塞进 `DecisionState`；树的运行态属于树自己（或黑板，§七 第 4 条）| 不动 | `:94-100`、`:103-110`、`:113-119`、`:129-135` |
| `protocol/messages.rs` | `MissionItem` 9 字节 = `type(1)+x(4)+y(4)`；`decode_task_set` 严格校验 `9×mission_count` | 若不携带时限/参数则不动；一旦要带 duration / 循环次数，**9 字节不够** → §七 第 6 条 | 待确认 | `:52-57`、`:265-286`、`:291-327`、`:314` |
| `protocol/command_decode.rs` | `TASK_SET` → `Vec<Mission>`；`members` 三分支（0 取消 / 1 单车 / >1 群发）；未知 mission_type `warn` 后丢弃（`filter_map`）| 新命令 = 新 `MISSION_*` 常量 + 新分支；未知类型仍丢弃（是否改成"上报失败"→ §七 第 3 条回执）| 改（若加命令）| `:27-70`、`:36-44`、`:47-68` |
| `mod.rs` | 模块清单（command / mission / robot / state / protocol …）| 仅当树进 base 时加一行 `pub mod behavior;` | 改（条件）| `:19-29` |

### 5.3 `pleiades-ugv/src/ugv/`

| 文件 | 现状职责 | 本任务需要做什么 | 改动程度 | 依据行号 |
|---|---|---|---|---|
| `goal.rs` | 完整目标服务：① 到达检测 → ② pop + 群发分配 + 立 goal → ③ 格级到达 → ④ 寻路 | **拆分**：任务切换（②）上移到树；保留"给定目标 → 下一格 / 已到达 / 不可达"的寻路能力供叶子调用，且**三个终态必须可区分**（今天都返回 `None`）| 改（中）| `:69-164`、`:76-84`、`:87-123`、`:126-137`、`:139-163`、`:167-170`、`:181-188` |
| `robot_handler.rs` | `on_tick` = 急停 → `get_path()` → `decide()` → `apply_action`；`reset` = 停车 + 清 goal + 写 Idle | 急停分支**逐字不动**；`:233` 那一行换成"tick 树"；`reset` 增加"halt 树"；`CarInner` 增加树实例字段 | 改（小）| `:217-244`、`:233`、`:241`、`:210-215`、`:33-41` |
| `executor.rs` | 三状态机纯决策（Idle/Turning/Moving），零副作用，带 8 个单测 | **不动**（结论 2 底层保留）| 不动 | `:38-51`、`:54-180`、`:200-286` |
| `planning/assignment.rs` | 群发 Goto / Circle 的散布位置计算 | **不动**；调用点从 `goal.rs` 移到树叶子后，入参不变 | 不动 | `:121`、`:171` |
| `emergency_stop.rs` | 前方障碍 → 停车 + 标记障碍供 D* 修补 | **不动**（安全在树外，结论 9）| 不动 | `:25` |
| `mod.rs` | 模块清单 | 仅当树放设备端时加一行 | 改（条件）| `:11-19` |

### 5.4 `pleiades-uav/src/uav/`

| 文件 | 现状职责 | 本任务需要做什么 | 改动程度 | 依据行号 |
|---|---|---|---|---|
| `goal.rs` | 与 ugv 近同构；差异：`:139-155` 注释掉动态障碍、喂空 `OccupancyGrid`（"天上无人小车"）| 与 ugv 同构改造；**保持这条差异**（不要因为共用树而把 UAV 的地面避障逻辑重新打开）| 改（中）| `:69-166`、`:88`、`:139-155`、`:160`、`:169-172` |
| `robot_handler.rs` | `on_tick` = `get_path()` → `decide()` → `apply_action`（机端无 2D 急停）| `:131` 换成"tick 树"；`reset` 增加 halt；`UavInner` 加树字段 | 改（小）| `:123-141`、`:131`、`:139`、`:116-121`、`:28-33` |
| `executor.rs` | 与 ugv **逐字节相同**（286 行）| 不动 | 不动 | 同上表 |
| `planning/assignment.rs`、`cluster_obstacles.rs` | 与 ugv 逐字节相同 | 不动 | 不动 | `:121`、`:171` |
| `mod.rs` | 模块清单 | 条件改 | 改（条件）| `:10-14` |

### 5.5 `pleiades-sim/src/sim/`

| 文件 | 现状职责 | 本任务需要做什么 | 改动程度 | 依据行号 |
|---|---|---|---|---|
| `goal.rs` | 440 行独立版：预测停点判到达、逐轴容差、格内二次逼近、建图增量 `Notify_Changed` | 与 ugv/uav **同构改造但保留 sim 的独有行为**（预测停点 / 格内逼近必须在树叶子下继续生效，否则到达精度回归到 0.39 m 的老问题）| 改（中）| `:29-92`、`:181-294`、`:204`、`:215`、`:229`、`:287-289`、`:296-299` |
| `sim_handler.rs` | `on_tick` = 装配门闩 → 底盘门闩 → 急停 → `get_path()` → `decide()` → `Apply_Action` | 门闩与急停**逐字不动**；`:475` 换成"tick 树"；`reset` 增加 halt | 改（小）| `:439-497`、`:456-472`、`:475`、`:477`、`:494`、`:428-437` |
| `executor.rs` | 与 ugv/uav 逐字节相同 | 不动 | 不动 | — |
| `planning/pathfinder.rs` | 706 行（比 ugv/uav 多 305 行，含 `Notify_Changed` 等）| 不动 | 不动 | — |
| `planning/assignment.rs` | 与 ugv 逐字节相同 | 不动（调用点 `:210`/`:224` 随叶子迁移）| 不动 | `:210`、`:224` |
| `control.rs` | 本地控制口（测试夹具）：直接构造 `Command` 注入 `robot_cmd_tx`，与地面站同一条下游 | **不动**（不进决策链，绕过 ORION 帧，天然随新语义走）| 不动 | `:7-23` |

### 5.6 明确"不需要改"的部分

| 部分 | 文件 | 为什么不用改 |
|---|---|---|
| 执行器状态机 | `ugv/uav/sim executor.rs`（三处逐字节相同，286 行）| 结论 2：底层保留不动；树只决定"何时、以什么目标"驱动它，决策规则（`TURN_ALIGN_RAD`、`SUB_TARGET_THRESHOLD_M`、直行连续化）与本次改造正交（`executor.rs:18-23`、`:54-180`）|
| 串口 / 飞控协议 | `ugv/stm32/`、`uav/mavlink/` | 动作语义（前进/转向/停止）不变；`apply_action` 的输入仍是 `MotionAction`（`stm32/mod.rs:164-175`）|
| 寻路算法 | `planning/pathfinder.rs` | D* Lite 是"给定目标怎么走"，与"任务怎么编排"无关 |
| 群发分配算法 | `planning/assignment.rs` | 结论 1：多机分工不在本任务；算法只被"调用点"影响，签名不变 |
| 他车动态障碍 | `planning/cluster_obstacles.rs` | 同上 |
| 集群数据面 | `core/cluster/*`（info / consumer / maintenance）| 结论 1：单机视角；入站 POSE → 表、表维护都是数据面 |
| 急停 | `ugv/emergency_stop.rs`、`sim/emergency_stop.rs` | 结论 9：安全机制不通过队列、不通过树 |
| 地图 / 建图 | `base/core/grid.rs`、`slam/`、`sensors/` | 树不产生也不消费地图的**写入**；D* 修补路径不变 |
| 帧格式 | `core/protocol/frame.rs` | 帧头与校验不动；载荷是否升版是独立未决项 |
| 地面站桥 | `pleiades-terminal/` | 仓内唯一"地面站相关"是 GDExtension 哑管道（`src/lib.rs:6-10`），不解析业务数据 |
| 位姿广播 | `robot.rs:204-255`（state_notifier）| 组帧字段（`PoseData` 38 字节，`messages.rs:16-34`）不变 |

---

## 六、范围之外

| 项 | 说明 |
|---|---|
| 多机分工 / 集群协同 | 谁去哪个点、编队、任务分配策略：不在本任务（结论 1）。当前群发语义只在 `decode` + `assignment` 里有，保持原样 |
| 集群数据面 | `core/cluster/*`、`TOPIC_ROBOT_POSE` 广播：不改 |
| 感知 / 检测 | YOLO、相机、雷达点云处理、目标跟踪（`Track` 只是**树里的叶子名**，不实现跟踪算法） |
| 地面站 | Pictor（Godot）在**仓外**，不在本仓库；本任务不涉及 |
| 仿真世界 | `orion-simulation-world/`（Godot 侧）不改；sim crate 只改上表所列 |
| 协议升版实施 | 只在 §七 记为未决，不在本任务实施 |

---

## 七、未决问题

> 以下均为**讨论未定稿**或**方案缺口**，本任务不做单方面决定。

1. **队列语义** — ✅ **已定（§十一 #15~#17）**：`preempt`（插队头）/ `append`（插队尾）/ `replace`（整队替换、打断、不恢复）三种；空列表 = `replace(vec![])` = 停车待命。**`update`（原地改参数）仍未定**——需要任务身份（id），协议亦无表达。现状只有 `replace` / `clear`（`mission.rs:31-40`、`robot.rs:289-294`、`:315-319`）。
2. **抢占后是否恢复** — ✅ **已定（§十一 #17）**：恢复，且**从暂停点继续**；实现方式是"树实例随任务走"（每任务一个实例、挂起不销毁），不引入全局快照。`replace` 仍明确"不恢复"。**实例如何随队列元素携带**（元素下沉设备端 / 元素泛型 / 设备端并行表）留到行为树 step。
3. **命令回执与状态查询**：协议没有 ACK / 任务状态查询消息（`msgid` 只有 1~6，`protocol/mod.rs:28-33`；`POSE` 只带 `valid` + `sub_gx/gy`，`messages.rs:16-34`）。地面站今天无法知道"任务成功/失败/被丢弃"。是否新增消息、还是复用 `POSE` 的 `valid` 位，未定。
4. **黑板** — ✅ **v1 不做（§十一 #21）**。节点用各自字段保存跨拍状态即可（`armed` / `attempts` / `used_ms` / `running_at`），不做共享黑板。真出现"兄弟节点必须共享一个值"时再引入；届时类型放 `Frame`（base）还是树实例（设备端）需重新评估。
5. **树的种类与子树复用策略** — 🟡 **部分已定（§十一 #11~#12）**：一任务一棵**顶层**树；顶层树之间不复用，**子树 / 叶子可复用**，复用 = 每次重新构建实例。仍未定：`Goto` 与 `Circle` 是否共享"走到某点"子树？sim 版的"预测停点 + 格内逼近"是放叶子内部还是抽成公共装饰器（待确认的倾向：放叶子内部，它是到达判据不是流程控制）？`Patrol`（多点顺序）能否由 `Goto` 子树循环组合？且 sim / ugv / uav 的 `goal.rs` 已分叉（§5.1），复用策略必须先回答"差异放哪"。
6. **协议升版** — ✅ **已定（§十一 #28）**：`TASK_SET` 载荷改为 `mission_count(1) + 9×mission_count`，删掉 `member_count` / `members`；`mission_count == 0` 即取消。车端、Pictor、协议文档**同批上线、无过渡期**；旧帧因长度校验必然被拒（两个方向都不可能误判）。长任务参数（duration 等）仍无载体，见第 6 条后续讨论。
7. **`get_path()` 三终态拆分的形式**：今天"到达"与"不可达"都返回 `None`（`ugv/goal.rs:76-84` vs `:157-162`），叶子的 `SUCCESS / FAILURE` 无法区分。返回值改成枚举（如 `Arrived / Next(cell) / Unreachable`）是必然，但**改在 base 定义还是设备端各自定义**未定。
8. **树代码归属** — ✅ **已定（§十一 #19）**：引擎在 base 的 `core/behavior_tree/`（一份）；树结构与叶子在三个设备 crate 各一份。**仍未定**：ugv / uav / sim 是**一次全改**还是先 sim 后真机（建议先 sim，见 §10 阶段 0）。
9. **树 tick 频率与时间基准** — ✅ **已定（§十一 #22）**：跟随 50 ms `auto_tick`（不自建分频）；计时用驱动每拍传入的 `Frame.dt_ms` **累积**，**不用墙钟**（挂起期间冻结）；`now_boot_ms()`（`protocol/mod.rs:42-47`）只作数据时间标签，不参与超时判定。
10. **失败上报的载体**：结论 10 要求"上报"，但协议没有对应字段（§七 第 3 条）。先落日志 + `POSE.valid=false`，还是等新消息？未定。
11. **`Mission` 新增变体**（`Patrol` / 带时限 `Encircle`）：`Circle` 已删（§十一 #27），当前只剩 `Goto { x, y }`；新增变体仍待定——一任务一树，加变体 = 加一个 `mission_type` + 一棵树 + 一处 `match`（设备端）。

---

## 八、风险

| # | 风险 | 如果不处理会怎样 |
|---|---|---|
| 1 | 树在三处各拷一份（沿用 `executor.rs` 的复制惯例）| 三份漂移。**这不是假设**：`goal.rs` 已经分叉成 189/191/440 三份，sim 修过的到达精度（`sim/goal.rs:29-92`）在真机不存在 —— 同一条 `Goto` 在 sim 与车上停在不同的位置精度，排查时会误判成"树的问题" |
| 2 | `halt` 只清树内状态、不同步执行器 | 动作不会自动到期（`stm32/mod.rs:164-175` 是"发一条记一条"），没人生成 `Stop` 车就**继续按上一 tick 的动作走**；另一侧若绕开 `ExecuteState` 单点写（`state.rs:125-135`），`sub_target` 会指向旧任务，状态机借道 self-heal（`executor.rs:101-108`）才能停，行为不可预测 |
| 3 | 抢占后树未重建、旧叶子的引用未释放 | 旧任务在下一 tick 继续推进（"打断"变成"叠加"），两个目标交替下发 → 车原地来回 |
| 4 | 时限 / 重试参数拍脑袋 | 结论 6 要求"每一项都有结束条件"，若 `Timeout` 值不合理：短任务被判超时（反复重试、原地抖动），长任务永不结束（回到"停不下来"的老问题）|
| 5 | 协议 9 字节私有扩展 | `decode_task_set` 严格校验 `9×mission_count`（`messages.rs:314`），长度不符 → 整帧 `None` → `command_consumer` 只 `warn`（`command_consumer.rs:36`）→ **任务静默不执行**，比现状更糟 |
| 6 | 没有回执（§七 第 3 条）| 树的失败恢复做得再好，地面站也只能从"车不动"推断；`Report` 叶子无处可写 → "不再静默丢任务"的目标在系统层面仍不成立 |
| 7 | sim 的独有逻辑在迁移中丢失 | 预测停点与格内逼近（`sim/goal.rs:68-92`、`:241-259`）一旦被"统一成 ugv 版"，到达误差回到实测 0.39 m（超过 0.3 判据）→ 仿真验收直接回归失败 |
| 8 | 50 ms tick 被树内同步/阻塞操作拖慢 | `auto_tick` 是单线程 `select!` 分支（`robot.rs:335-343`），树里任何 `await` 长操作会挤压寻路与急停的响应；急停在 `on_tick` 最前面，但仍在同一 tick 内 |
| 9 | 树内状态与 `ExecuteState` 双重真相 | 两处都能回答"我在干什么"，一旦不一致（如树说 RUNNING、执行器说 Idle），`sub_target` 广播（`robot.rs:219-229`）会向集群发错误意图，他车把它当动态障碍（`cluster_obstacles.rs`）|
| 10 | 只做 `Goto` 一棵树就宣布完成 | 结论 6 的长任务（带终止条件）落在 `Circle` 树上；只迁 `Goto` 等于把"任务一次性"这个动机留在原地 |

---

## 九、验收要点

> 只写判据方向，不含 step 编号与具体命令。

1. **外部行为不回归**：同样的 `TASK_SET`（单车 / 群发，`Goto` / `Circle`）在 sim 与真机链路上，到达判据与停点精度不低于改造前（sim 侧对齐 `sim/goal.rs:29-92` 的逐轴 ≤ 0.3 m；真机侧保持 `ugv/goal.rs:27` 的 0.3 m）。
2. **被抢占的分支确实停止动作**：`replace` 之后的一个 tick 内，执行器 `state = Idle`、`sub_target = None`、底盘收到显式 `Stop`（等价于今天 `robot_handler.rs:210-215` 的效果）；车不再朝旧目标移动。
3. **不可达不再静默丢任务**：构造不可达目标，任务**不**在"仅一条 `warn!` 后消失"（对照 `ugv/goal.rs:157-162` 现状）；能观察到"重试 → 上报 → 明确出队"的完整链条。
4. **每个任务都有终止途径**：任务以"完成 / 失败 / **被覆盖**"三种方式之一结束。持续型任务（一直巡逻）允许长期 `RUNNING`，靠 `replace` 终止——**不设运行期兜底时限**（§十一 #33）；任何情况下 `replace` 都要能在一个 tick 内让它停下。
5. **树可被单测**：三值语义可在无串口、无网络、无 tokio 运行时的条件下断言（对照 `executor.rs:200-286` 的纯决策单测风格）。
6. **`RUNNING` 不阻塞**：树处于 RUNNING 期间，急停、位姿广播、集群消费仍按原周期工作。
7. **安全边界不破**：树被 halt / 根本不存在 / 树内部 panic 被捕获的情况下，急停链路仍能停车（`robot_handler.rs:224-231`）。
8. **三处一致或差异有据**：ugv / uav / sim 树的对外行为差异（UAV 空 grid、sim 预测停点）有明确记录，不是"复制时忘了改"。
9. **`replace` 语义可指认**：代码路径上能指出"打断 → 更新 → 不恢复"三件事分别在哪实现（今天三件事全在 `robot.rs:315-319` 一行里）。

---

## 十、建议的分阶段（**待人类确认**；阶段 0 的第一步已拆成具体 step，见 §十二）

> 仅供评审时讨论优先级；本章表格**不含 step 编号**，具体 step 划分见 §十二。

| 阶段 | 内容 | 为什么这样排 |
|---|---|---|
| 0 | `Goto` 一棵树，**行为等价迁移**（不加新语义，`get_path()` 拆三终态）| 先把接口边界钉死，回归风险最小；此阶段验收 = "完全不变" |
| 1 | 加恢复结构（`Timeout` + `Retry` + 上报落点）| 消灭"失败即丢弃"，是方案的主要动机（结论 10）|
| 2 | `Circle/Encircle` 树 + 带终止条件的长任务 | 验证结论 6 的表达力；sim 的到达精度差异在此阶段暴露 |
| 3 | 队列新语义（`append` / `preempt` / `update`）+ 抢占是否恢复 | ✅ 已定（§十一 #15~#17），**实施提前到 Step 1**（只做结构与 API，消费方切换仍在行为树 step）；`update` 仍未定 |
| 4 | 协议升版（若第 6 条结论为"需要"）+ 回执 / 状态查询 | 影响仓外地面站，必须最后做 |

**跨阶段前置**：§七 第 4（黑板）、第 5（子树复用）、第 8（代码归属 + 三 crate 同步策略）三条**必须在阶段 0 之前定**，否则阶段 0 就得返工。

---

## 十一、追加定稿结论（2026-10-27 讨论）

> 与 §三 并列；两者冲突时**以本节为准**（本节后出）。编号接 §三 的 1~10。

| # | 结论 | 要点 |
|---|---|---|
| 11 | **一任务一树** | 队列元素 = 一个任务；一种任务类型 ↔ 一棵**顶层**树；新增任务 = 加 `Mission` 变体 + 加一棵树 + 重新编译（重编译不是问题）。 |
| 12 | **树定义可复用，实例不复用** | `Goto` 树可被反复使用、也可被别的树当子树挂载；复用 = **每次重新构建实例**（工厂函数），不共享节点对象（节点带 RUNNING 位置 / 重试计数 / 计时状态）。配套引擎规则：节点被**重新进入**时先 reset，不假设"上次结束时它是干净的"。 |
| 13 | **任务 → 树的分派必须编译期穷尽** | 用 `match Mission` 穷尽枚举；漏加编译不过，不得运行时查表查不到就静默跳过。分派点落在设备端（每个设备 crate 一处，见 #26）。 |
| 14 | ~~每棵树有兜底时限~~ **（2026-10-27 作废 → 见 #33）** | 原设想：兜底放树机制里，防"永远 RUNNING 的树占住队首"。**现决定不做**——"一直巡逻直到被新命令覆盖"是正常任务，`replace` 本身就是它的结束条件。 |
| 15 | **队列三语义** | `preempt` 插队头（当前任务立即让位）/ `append` 插队尾（当前任务不受影响）/ `replace` 整队替换（打断 + 丢弃 + **不恢复**）。空列表 = `replace(vec![])` = 停车待命。 |
| 16 | **执行中的任务不离队** | 队首 = 正在执行的任务（peek 语义）；**只有拿到终值才出队**；出队只有一个时机、一条路径（与今天"先 `pop_next` 再判能不能做"相反）。 |
| 17 | **抢占可恢复，且从暂停点继续** | 被抢占任务带实例退到第二位；新任务出队后它回到队首、**继续 tick 原实例**。挂起 = 不 tick，时间按 tick 累积（挂起期间**冻结**，不用墙钟）。恢复时叶子必须**重新武装**（重新表态目标），保证"树在走 ⇔ 车收到目标"。抢占可嵌套；暂停中的实例按队列顺序天然构成栈，不需要额外的暂停栈结构。 |
| 18 | **队列目录化 + 元素归属** | 新建 `pleiades-base/src/Robot/core/mission_queue/`，把元素定义与全部操作收进去；元素 = **任务参数**（`Mission`）。元素是否携带树实例 → 行为树 step 决定，本 step 不预设。 |
| 19 | **引擎归 base，树与叶子归设备端** | `core/behavior_tree/` 只放**引擎**（一份）；树结构与叶子实现（读状态、调导航、调 YOLO）在 ugv / uav / sim 各一份。理由：同一任务在不同设备上**结构也可能不同**（uav 无 2D 急停，恢复链不一样），强行共享会变成参数化地狱。 |
| 20 | **引擎非泛型** | `trait Node` 无类型参数；上下文收缩为 `Frame { dt_ms, tick_no, out }`，**不含任何设备字段**。不引入 `Ctx` / `Obs` / `Clock` / `downcast`。 |
| 21 | **无观测快照、无黑板** | 删 `Obs`：`RobotState` 本身就是观测，节点直接读 `Arc<RwLock<RobotState>>` 或自己持有的能力端口——"需要哪个读哪个"。v1 不做黑板（§七 #4）。代价：同一拍内不同节点可能读到不同时刻的值 → 需"一处读完再用"的纪律。 |
| 22 | **`tick` 为 async** | 节点可直连设备 API（`lidar.get_scan().await`、相机抓帧）。代价：控制环时延与节点行为耦合 → **叶子里的 `await` 只能"拿现成的"，长过程必须拆成"发意图 → 下拍取结果"**。 |
| 23 | **`halt` 只清节点状态** | `halt` 不产生 `Stop`；驱动在 `halt` 之后统一发一次 `Stop` + 清意图。每个节点的 `halt` 必须**幂等**。 |
| 24 | **节点集** | `Sequence`（记忆）/ `ReactiveSequence` / `Fallback`（记忆）/ `ReactiveFallback` / `Repeat` / `RepeatForever` / `Timeout` / `Retry` / `Guard` / `Inverter` / `Wait` / `Action` / `Condition`。**记忆式与反应式必须分开**：反应式场景（"看到人才去跟，看不到就回去巡逻"）里用记忆式会让条件不再重判。 |
| 25 | **叶子直连导航，驱动只读输出** | 叶子持有导航句柄，直接 `set_goal` / 算下一格；**每拍把"下一格"写进 `Frame.out`**，驱动拿它去开车。**没有下一格就是停车**（不需要 `Stop` 意图，也不需要意图缓冲）。云台、拍照、蜂鸣等非运动外设同样由节点直连。 |
| 26 | **任务 → 树的分派在设备端** | 三个设备 crate 各一处 `match Mission`，编译期穷尽（加 `Mission` 变体漏改则编译不过）。代价：加一种任务改三处。 |
| 27 | **协同上移到指挥层** | 地面站逐车算出各自目标点、**逐车单独发帧**；每帧 `missions[0]` 就是"该车自己的点"。车端**不做分配**：删 `planning/assignment.rs`（×3）、删 `Mission::Circle`、`Mission` 去掉 `members`、删 `own_peer_id`。**保留** `cluster_obstacles`（他车位姿 → 动态障碍，属于单车自保，不是协同）。 |
| 28 | **`TASK_SET` 载荷升版** | 新布局 `mission_count(1) + 9×mission_count`；删 `member_count` / `members`。`mission_count == 0` = 取消（唯一入口）；`> 0` 但解析后全无效 → `warn` + **不动队列**（今天会误当取消）。车端 / Pictor / 协议文档**同批上线，无过渡期**；旧帧因长度校验必然被拒。 |
| 29 | **sim 与 ugv 的"复刻"定义** | **ugv 跑什么树，sim 就跑什么树**：`mission/` 下树文件与 ugv **逐字相同**（只差 `use` 路径），`nav/` 结构相同；差异只允许出现在**端口实现**与**常量**（到达判据、刹车模型等），且每处差异要登记。写树时不得把设备专有逻辑塞进树里，否则拷不过去。 |
| 30 | **验证分工** | ugv / uav 的实车验证由人类执行；Agent 侧只做**代码评审 + sim 端到端**。sim 排在第 6 步，因此 Step 3~5 的验收只能是"编译 + 单测 + 评审"。 |
| 31 | **失败带原因码** | `Status::Failure(FailCode)`。**通用码**（0 未指定 / 1 超时 / 2 重试用尽 / 3 无路 / 4 被堵）由 base 定义；设备用 `FailCode::Device(u16)`（1000+）扩展。**树里尽量只用通用码**——用了设备码，这棵树就搬不到别的设备。 |
| 32 | **算路在叶子里** | 叶子 tick 内：判到达 → 没到就调导航算下一格 → 写入 `Frame.out`。**每拍重算**（不能缓存"一格直到到达"，否则临时障碍反应不了）。条件判的始终是**最终目标**（各设备自己的到达兜底规则在导航器内部）。驱动只做三件事：门闩/急停 → `tree.tick` → 拿下一格开车。 |
| 33 | **不做引擎兜底时限** | 取消"每棵树最多 N 拍"。持续任务靠 `replace` 结束；"每个任务必须有结束条件"改为"**必须能被 halt，被覆盖也算结束**"。代价：写成永远 RUNNING 的树会占住队首，属 bug，靠评审与单测发现，不靠运行期兜底。 |
| 34 | **树实例懒创建、按编号存** | 任务执行到时才建树，未开始的任务没有实例。实例数 = "**已开始但未结束**的任务数"（单次抢占 = 2 棵；嵌套抢占会更多）。设备端用 `HashMap<TaskId, Tree>` 存；`TaskId` 由 base 在**入队时**分配。 |
| 35 | **建树失败 = 停住待命** | `build_task` 返回 `Err` → 记日志 + 上报 + **清空队列 + 停车**，等下一条命令；不继续拿后面的任务去试。 |
| 36 | **"该建树了"由 base 发现** | `main_loop` 每拍比较"队首 `TaskId`"与"当前任务 id"，不同就调 `device.build_task`。设备端不碰队列。`build_task` 必须**幂等**：该 id 已有实例（抢占恢复）→ 只激活、不重建。 |
| 37 | **sim 双栈 + 判据不做完全一致** | `sim/` 下分 `ugv_sim/` 与 `uav_sim/`，各自复刻对应真机的 `nav/` + `mission/`；**树文件与真机逐字相同**。**到达判据不要求一致**：sim 保留"预测停点 + 格内逼近"（因仿真底盘有 3 m/s² 惯性，1.2 m/s 时滑行 0.3 m），真机仍按世界距离 0.3 m。理由：判据属导航内部，树的行为一致性才是目标。 |
| 38 | **`reset` 与 `suspend` 分开** | `replace` / 切模式 → `reset()`：halt 树 + **清实例表** + 清 nav + 停车。`preempt` → `suspend()`：**不停树**（保留节点进度）+ 清 nav + 停车 + **保留实例表**。配套：叶子在 `nav.status() == Idle` 时也要重新 `set_goal`——否则暂停期间目标被清，恢复后会空转。 |

---

## 十二、实施计划

> step 之间**严格串行**；每个 step 走"规划 → 实现 → 评审"（评审 FAIL 重跑，最多 3 轮），见 `.github/instructions.md`。后续 step 在讨论定稿后再追加，本节只写**已定稿**的 step。

### Step 1 — mission queue 重构（结构重构 + API 补全，**行为零变化**）

#### 12.1.1 目标

1. `Mission`（元素定义）与 `MissionQueue`（队列 + 全部操作）收进 `pleiades-base/src/Robot/core/mission_queue/`。
2. 队列 API 覆盖 §十一 #15~#17 已定语义：`replace` / `append` / `preempt` / `clear` / `head` / `pop_next`。
3. 现有调用方**行为不变**：本 step 只搬结构、补方法，**不切消费时机**。

#### 12.1.2 非目标（硬边界）

| 不做 | 原因 / 归属 |
|---|---|
| 不改 `GoalService::get_path()` 里 `pop_next()` 的调用时机 | "执行中不离队"的**消费方切换**属于行为树 step（驱动接管任务切换，§4.2） |
| 不新增 `AutoCmd` 变体、不改协议 | `preempt` / `append` 的协议表达是独立的后续 step；**协议升版见 Step 3**；本 step 只让能力就位 |
| 不给元素加字段、不引入树实例 | §十一 #18 留待行为树 step |
| 不动 `ExecuteState` / 主循环节拍 / 急停 | 与队列无关 |

#### 12.1.3 改动文件清单

| # | 文件 | 动作 | 内容 |
|---|---|---|---|
| 1 | `pleiades-base/src/Robot/core/mission_queue/mod.rs` | 新增 | 模块声明 + re-export：`pub use element::Mission;` / `pub use queue::MissionQueue;` |
| 2 | `pleiades-base/src/Robot/core/mission_queue/element.rs` | 新增 | `Mission` 定义从 `command.rs:50-64` **逐字搬迁**（含注释与 derive）；`TaskId`、`QueuedTask` 也放这里 |
| 3 | `pleiades-base/src/Robot/core/mission_queue/queue.rs` | 新增 | `MissionQueue`（`VecDeque<QueuedTask>` + `next_id`）+ 8 个方法 + 单测 |
| 4 | `pleiades-base/src/Robot/core/mission.rs` | **删除** | 内容全部迁入 #3 |
| 5 | `pleiades-base/src/Robot/core/mod.rs` | 改 | `:25` `pub mod mission;` → `pub mod mission_queue;` |
| 6 | `pleiades-base/src/Robot/core/command.rs` | 改 | 删 `Mission` 定义；加 `pub use super::mission_queue::Mission;`（**保持 `command::Mission` 旧路径可用 → 设备端零改动**） |
| 7 | `pleiades-base/src/Robot/core/robot.rs` | 改 | `:21` `use super::mission::MissionQueue` → `use super::mission_queue::MissionQueue` |
| 8 | `pleiades-base/src/Robot/mod.rs` | 改 | `:15` re-export：`Mission` 从 `core::mission_queue::Mission` 引出（`AutoCmd` / `Command` / `ManualCmd` / `ModeCmd` 仍从 `command`） |
| 9 | `pleiades-ugv/src/ugv/goal.rs` | 改 1 行 | `:20` `core::mission::MissionQueue` → `core::mission_queue::MissionQueue` |
| 10 | `pleiades-uav/src/uav/goal.rs` | 改 1 行 | `:20` 同上 |
| 11 | `pleiades-sim/src/sim/goal.rs` | 改 1 行 | `:22` 同上 |
| 12 | `pleiades-sim/src/sim/emergency_stop.rs` | 改 1 行 | `:170`（`#[cfg(test)]` 内）同上 |

> **不需要改**的 `Mission` 导入（由 #6 的 re-export 兜住）：`uav/goal.rs:19`、`sim/goal.rs:21`、`sim/control.rs:39`、`command_decode.rs:10` 与 `:102`（测试）、`ugv/goal.rs:19`。
> **需要同步的文档目录树**（纯文字同步，可留到最后一起做）：`Architecture/robot_arch.md:58`、`Architecture/Pleiades_Architecture.md:512`、`docs/design_doc/robot_controller.md:364`。

#### 12.1.4 API 定稿

```rust
pub struct TaskId(pub u64);                     // 单调递增，永不复用
pub struct QueuedTask { pub id: TaskId, pub mission: Mission }

pub struct MissionQueue {
    queue: VecDeque<QueuedTask>,
    next_id: u64,                               // 入队时分配
}

impl MissionQueue {
    pub fn replace(&mut self, missions: Vec<Mission>);  // 清空 + 装入；逐条分配 id
    pub fn append(&mut self, missions: Vec<Mission>);   // 追加队尾，批内顺序保持
    pub fn preempt(&mut self, missions: Vec<Mission>);  // 插入队头，批内 missions[0] 最优先
    pub fn clear(&mut self);                            // 清空（next_id 不回退，避免 id 复用）
    pub fn head(&self) -> Option<&QueuedTask>;          // 只读队首（= 执行中的位置），不移除
    pub fn pop_next(&mut self) -> Option<QueuedTask>;   // 唯一出队口（终值之后调用）
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

`TaskId` 的必要性：设备端要按 id 存"挂起的树实例"（§十一 #34），而 `Mission` 是纯协议参数、不该带 id，所以 id 属于队列元素。

三个决定与理由：

- **`push` 删除**：由 `append` 取代，且原 `push` 全仓无调用者（`docs/清理文档计划.md:83` 已登记为死代码）。
- **`peek` 更名 `head`**：原 `peek` 全仓无调用者（同上），避免两个同义方法并存。
- **容器 `Vec` → `VecDeque`**：`preempt` 插头、`head` / `pop_next` 取头，语义更直白（n ≤ 255，性能不是理由）。

#### 12.1.5 详细步骤

1. 建 `mission_queue/` 目录与 `mod.rs`；三个新文件按规范加文件头（`Presented by KeJi` / `Created Date ： 2026-10-27` / `Modified Date ： 2026-10-27`）。
2. 从 `command.rs` **逐字**搬 `Mission` 到 `element.rs`；`command.rs` 加 re-export。
3. `cargo check -p pleiades-base` —— 先确认"旧路径 `command::Mission` 仍可用"这一步单独成立，再往下动。
4. 新建 `queue.rs`：`VecDeque` + 8 个方法（`#[derive(Debug, Clone, Default)]`）；模块注释写明"`append` / `preempt` / `head` 的**消费方切换**在行为树 step，本 step 只提供能力"。
5. 删除 `core/mission.rs`；改 `core/mod.rs` 模块清单。
6. 改 import：`robot.rs:21` + 三个设备 crate 各 1 行（ugv/uav/sim 的 `goal.rs`）+ `sim/emergency_stop.rs:170`；`Robot/mod.rs:15` re-export 消歧。
7. `cargo check --workspace`；再 `cargo test -p pleiades-base`（现存的 `command_decode` 7 个 `TASK_SET` 用例是回归网）。
8. 补 `queue.rs` 单测（下表）并跑 `cargo test -p pleiades-base mission_queue`。
9. 同步三处文档目录树里的 `mission.rs` 行。

#### 12.1.6 单测（`queue.rs` 的 `#[cfg(test)]`）

| # | 用例 | 断言 |
|---|---|---|
| 1 | `replace` | 旧队列被清空；新队列顺序保持 |
| 2 | `replace(vec![])` | 队列空、`head() == None`、`is_empty()` |
| 3 | `append` | 追加在尾部；已有元素位置不变 |
| 4 | `preempt` 单条 | 新元素在队首，原队首退到第二位 |
| 5 | `preempt` 多条 | 批内 `[0]` 在队首，整批顺序保持 |
| 6 | 连续两次 `preempt` | 后一次在前（后进先出）；原始队列尾部不变 |
| 7 | `head` 不移除 | 连调两次结果相同、`len` 不变 |
| 8 | `pop_next` | 取队首并移除；空队列返回 `None` |
| 9 | `len` / `is_empty` | 三种操作后计数正确 |
| 10 | `TaskId` 分配 | 入队顺序递增；`replace` 后新元素拿到更大的 id；`clear` 后 id 不回退（不复用） |

#### 12.1.7 验收标准

1. `cargo check --workspace` 通过；`cargo test -p pleiades-base` 全绿（含既有 7 个 `TASK_SET` 解码用例）。
2. **行为零变化**：`ORION_TASK_SET` 仍走 `replace`（`robot.rs:318` 语义未改）；三处 `goal.rs` 的 `pop_next` 调用点行为与改造前逐字等价（即"开始时取走队首"这一旧语义**原样保留**）。
3. `grep` 结果：全仓不再出现 `core::mission::` 路径；`core/mission.rs` 不存在；`push` / `peek` 两个方法名消失。
4. 新增的 `append` / `preempt` / `head` 有单测覆盖，且**本 step 内没有生产调用者**（能力就位，接线在后续 step）。

#### 12.1.8 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | `command.rs` 的 re-export 造成 `Mission` 两条路径并存，后续出现两种 import 风格 | 本 step 允许并存（换来设备端零改动）；后续 step 统一到 `mission_queue::Mission` 并删掉 `command.rs` 的 re-export |
| 2 | `append` / `preempt` 有实现无调用者，被误当成"已接线" | `queue.rs` 模块注释写明"消费方切换在行为树 step"；验收第 4 条明确这点 |
| 3 | 删 `mission.rs` 漏改 import | 不会静默：`cargo check --workspace` 直接编译失败（步骤 7 挡住） |
| 4 | 把"补 API"误做成"顺手切消费方"，导致行为变化混进结构重构 | 非目标第 1 条写死；评审对齐验收第 2 条 |

### Step 2 — behavior_tree 引擎（base 内新建，**非泛型 + async + 零设备字段**）

#### 12.2.1 目标

1. 在 `pleiades-base/src/Robot/core/behavior_tree/` 新建引擎：三值语义 + 失败码、`Node` trait、组合子、装饰器、通用叶子。
2. 引擎**非泛型**（`trait Node` 无类型参数）、**async**（`#[async_trait]`）、**零设备字段**（不出现 pose / nav / detection / camera）。
3. 提供单测：三值语义、失败码传递、记忆式 vs 反应式差异、halt 幂等。

#### 12.2.2 非目标（硬边界）

| 不做 | 归属 |
|---|---|
| 不定义任何具体任务树（Goto / 巡逻 / 封控） | 设备端，后续 step |
| 不定义叶子实现（读 `RobotState`、调导航、调 YOLO） | 设备端，后续 step |
| 不改 `DeviceHandler` / `on_tick` 签名、不改 `main_loop` | 接线 step（未定稿） |
| 不做黑板、不做 `Obs` | §十一 #20 / #21 |
| 不做并行节点、不做 XML / 运行时热加载 | 结论 4 |
| 不做 `Mission → 树` 分派 | 设备端（§十一 #26），后续 step |

#### 12.2.3 改动文件清单

| # | 文件 | 动作 | 内容 |
|---|---|---|---|
| 1 | `pleiades-base/src/Robot/core/behavior_tree/mod.rs` | 新增 | 模块声明 + re-export（`Status` / `FailCode` / `Node` / `Frame` / `MotionOut` / 各节点） |
| 2 | `pleiades-base/src/Robot/core/behavior_tree/status.rs` | 新增 | `Status { Success, Running, Failure(FailCode) }`、`FailCode`（5 个通用码 + `Device(u16)`） |
| 3 | `pleiades-base/src/Robot/core/behavior_tree/frame.rs` | 新增 | `Frame { dt_ms, tick_no, out }`、`MotionOut { next_cell }` |
| 4 | `pleiades-base/src/Robot/core/behavior_tree/node.rs` | 新增 | `trait Node`（`async fn tick` + `fn halt`） |
| 5 | `pleiades-base/src/Robot/core/behavior_tree/composite.rs` | 新增 | `Sequence`（记忆）、`ReactiveSequence`、`Fallback`（记忆）、`ReactiveFallback` |
| 6 | `pleiades-base/src/Robot/core/behavior_tree/decorator.rs` | 新增 | `Timeout`、`Retry`、`Guard`、`Inverter`、`Repeat`、`RepeatForever` |
| 7 | `pleiades-base/src/Robot/core/behavior_tree/leaf.rs` | 新增 | `Wait(ms)`、`Action`（同步闭包）、`Condition`（同步闭包） |
| 8 | `pleiades-base/src/Robot/core/mod.rs` | 改 | 加一行 `pub mod behavior_tree;`（`:19-29` 的模块清单） |

> 无需新增依赖：`async-trait = "0.1"`、`tokio = { features = ["full"] }` 已在 `pleiades-base/Cargo.toml`（`:51`、`:63`）。
> 文档同步（可留到最后一起）：`Architecture/robot_arch.md:54-71` 的目录树加 `behavior_tree/`。

#### 12.2.4 API 定稿

```rust
// status.rs
pub enum FailCode {
    Unspecified,     // 0：条件不成立 / 未指定
    Timeout,         // 1：Timeout 装饰器到期
    RetryExhausted,  // 2：Retry 用尽
    Unreachable,     // 3：导航——地图上无路
    Blocked,         // 4：导航——暂时被堵
    Device(u16),     // 1000+：设备自定义（语义由设备端定义）
}

pub enum Status { Success, Running, Failure(FailCode) }

// frame.rs
pub struct MotionOut {
    /// 本拍叶子算出的"下一格"；None = 这一拍不动（执行器会自然停车）
    pub next_cell: Option<(i32, i32)>,
}

pub struct Frame<'a> {
    pub dt_ms: u32,                  // 驱动传入；挂起时不推进（时间冻结）
    pub tick_no: u64,
    pub out: &'a mut MotionOut,
}

// node.rs
#[async_trait]
pub trait Node: Send {
    async fn tick(&mut self, f: &mut Frame<'_>) -> Status;
    fn halt(&mut self);              // 只清自身状态；必须幂等；不发 Stop
}
```

> **没有 `Tree` 包装**：树就是一个 `Box<dyn Node>`。驱动持有它，每拍 `tick`、需要时 `halt`。也不设兜底时限（§十一 #33）。

**语义约定（实现与评审都按这张表）**

| 节点 | 语义 |
|---|---|
| `Sequence`（记忆） | 子 `Success` → 下一个；子 `Running` → 记住位置并返回 `Running`；子 `Failure` → 清记忆并返回 `Failure`；全过 → 清记忆返回 `Success` |
| `ReactiveSequence` | 每拍从第 0 个子重判；不存位置。**条件类节点必须用它，否则条件不再重判** |
| `Fallback`（记忆） | 子 `Failure` → 下一个；子 `Running` → 记住位置；任一 `Success` → 清记忆返回 `Success` |
| `ReactiveFallback` | 每拍从第 0 个子重判；高优先级分支转 `Success`/`Running` 时，**先 `halt` 上一拍在跑的低优先级分支** |
| `Timeout(ms)` | `dt_ms` 累加；超限 → `child.halt()` + `Failure(Timeout)`；`halt` 时计数清零 |
| `Retry(n)` | 子 `Failure(_)` → 计数 +1，`child.halt()` 后下拍重试；达到 n → `Failure(RetryExhausted)`；子 `Success` → `Success` 并清计数 |
| `Repeat(n)` / `RepeatForever` | 子 `Success` → 计数 +1，`child.halt()` 后下拍重 tick；子 `Failure` → `Failure`；`Repeat(n)` 计满 → `Success` |
| `Guard(cond)` | `cond == false` → `Failure`（**不 tick 子**）；否则 tick 子 |
| `Inverter` | `Success ↔ Failure` 互换，`Running` 原样 |
| `Wait(ms)` | `dt_ms` 累加；未到 `Running`，到了 `Success`；`halt` 清零 |
| `Action` | 同步闭包 `FnMut(&mut Frame) -> Status`（需要 I/O 的叶子请直接 `impl Node`） |
| `Condition` | 瞬时返回 `Success` / `Failure(Unspecified)`，**从不 `Running`** |

**失败码传递规则**（实现与评审都按这条）：

| 节点 | 传什么码 |
|---|---|
| `Sequence` / `ReactiveSequence` | 原样传失败子节点的码 |
| `Fallback` / `ReactiveFallback` | 换分支时丢弃上一个分支的码；最后失败时传最后一个分支的码 |
| `Timeout` | 用自己的 `Timeout`（超时是它造成的） |
| `Retry` | 用 `RetryExhausted`（各次尝试的码进日志） |
| `Guard` / `Inverter` / `Condition` | `Unspecified` |

#### 12.2.5 详细步骤

1. 建目录与 7 个文件（`mod` / `status` / `frame` / `node` / `composite` / `decorator` / `leaf`），按规范加文件头（`Presented by KeJi` / Created 2026-10-27 / Modified 2026-10-27）。
2. `status.rs` → `frame.rs` → `node.rs`（依赖顺序，先编译过再往上叠）。
3. `composite.rs`：先写记忆式两个，再写反应式两个（反应式要复用"切分支先 halt 旧支"的公共逻辑）。
4. `decorator.rs`：`Timeout` / `Retry` / `Repeat` / `RepeatForever` / `Guard` / `Inverter`。
5. `leaf.rs`：`Wait` / `Action` / `Condition`。
6. `core/mod.rs` 加模块声明。
7. 每文件内 `#[cfg(test)]` 写单测（下表），跑 `cargo test -p pleiades-base behavior_tree`。
8. `cargo check --workspace`（确认没有污染设备端）。

#### 12.2.6 单测（目标 16 个）

| # | 用例 | 断言 |
|---|---|---|
| 1 | `Sequence` 记忆 | 子 2 `Running` 后，下一拍**不再** tick 子 1（用计数器验证） |
| 2 | `Sequence` 失败清记忆 | 子 2 `Failure` 后下一拍从子 0 重来 |
| 3 | `ReactiveSequence` | 子 1 已 `Success`、子 2 `Running`；本拍子 1 变 `Failure` → 整个 `Failure`（证明条件每拍重判） |
| 4 | `ReactiveFallback` 抢回 | 低优先级分支 `Running` 时高优先级转 `Success` → 低优先级被 `halt`（计数验证），返回 `Success` |
| 5 | `Fallback` 记忆 | 子 1 `Running` 时**不去** tick 子 2 |
| 6 | `halt` 传播 | 父 `halt` 后，正在 `Running` 的子收到 `halt` |
| 7 | `Timeout` | `dt_ms=50`、limit=100 → 两次后 `Failure`，且子被 `halt` |
| 8 | `Retry` | 子连续 `Failure`，第 n 次后返回 `Failure`；计数正确；子成功时清计数 |
| 9 | `Repeat(3)` | 子 3 次 `Success` 后 `Success` |
| 10 | `RepeatForever` | 一直 `Running`，不会自己结束；外部 `halt` 后状态清零 |
| 11 | `Wait` | `dt_ms=50`、wait=100 → `Running`,`Running`,`Success` |
| 12 | `Guard` | `cond=false` 时子**未被** tick |
| 13 | `Inverter` | `Success ↔ Failure`，`Running` 不变 |
| 14 | `halt` 幂等 | 连续 `halt` 两次不 panic、状态一致（每个节点都测） |
| 15 | `Frame.out` | 叶子写入的 `next_cell` 驱动读得到；没写时是 `None` |
| 16 | 失败码传递 | `Sequence` 原样传；`Fallback` 换分支丢弃、最后传末支的码；`Timeout` 传 `Timeout`；`Retry` 传 `RetryExhausted` |

> 全部用 `#[tokio::test]`（`tick` 是 async）；不需要串口、网络、`RobotState`。

#### 12.2.7 验收标准

1. `cargo check --workspace` 通过；`cargo test -p pleiades-base behavior_tree` 全绿（16 个用例）。
2. **引擎非泛型**：`trait Node` 无类型参数；`behavior_tree/` 内除闭包泛型外不出现 `<C`。
3. **引擎零设备词汇**：`behavior_tree/` 下不出现 `RobotState` / `pose` / `nav` / `detection` / `camera` / `stm32` / `mavlink`（`grep` 可验）。
4. **每个节点都有 halt 幂等测试**（用例 14 覆盖全部节点）。
5. **失败码可传递**：用例 16 证明四类节点各自的码传递规则。
6. **零外部影响**：不新增依赖、不动设备端一行、不改 `main_loop`。

#### 12.2.8 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | 记忆式 / 反应式选错 → 条件不再重判（"看到人走了还在跟"） | 两个类型名字显式区分；用例 3 / 4 锁住语义；设备端写树时按"有条件的循环一律用 Reactive"这条规则 |
| 2 | `async_trait` 每拍装箱（每个节点一次 `Box::pin`） | 20 Hz × 十余节点，可忽略；若真成瓶颈，后续手写 `Future` 实现替换 |
| 3 | `halt` 语义靠各节点自觉，容易漏"清计数/清记忆" | 用例 14 逐节点覆盖；`halt` 幂等写进 `trait Node` 的文档注释 |
| 4 | 节点里 `await` 长任务拖慢 50 ms 环 | 引擎不提供长任务节点；`Frame` 文档写明"叶子里的 await 只能拿现成的"；评审叶子时检查 |
| 5 | `Frame` 词汇在接线 step 发现不够 | 现在只有 `dt_ms` / `tick_no` / `out.next_cell`；扩展只加字段，不动 `trait Node` |
| 6 | **不设兜底时限** → 写成永远 RUNNING 的树会占住队首 | 已知风险（§十一 #33）：持续任务是合法形态，不能一棍子打死；靠评审 + 单测发现死循环，不靠运行期兜底 |

### Step 3 — 协议升版 + 全仓删协同（**类型手术，行为仅"旧帧被拒"变化**）

#### 12.3.1 目标

1. `ORION_TASK_SET` 载荷去掉 `member_count` / `members`，改为 `mission_count(1) + 9×mission_count`（§十一 #28）。
2. `Mission` 只剩 `Goto { x, y }`，删 `Circle`（§十一 #27）。
3. 车端删除协同：三份 `planning/assignment.rs` 删除，分配调用点移除。
4. 修掉"解析后全无效 → 误取消整个队列"的旧坑（§十一 #28）。

#### 12.3.2 非目标（硬边界）

| 不做 | 归属 |
|---|---|
| 不建 `nav/`、不建 `mission/`、不接树、不改 `on_tick` | Step 4~6 |
| 不改 Pictor（GDScript） | Step 7 |
| 不新增 `Mission` 变体（`Patrol` 等） | §七 #11 |
| 不动 `frame.rs` 帧头、不动 `DeviceHandler` | 不变 |
| 不改 `cluster_obstacles` / `core/cluster/*` | 属于单车自保与数据面 |

#### 12.3.3 改动文件清单

**base**

| # | 文件 | 动作 | 内容 |
|---|---|---|---|
| 1 | `core/mission_queue/element.rs` | 改 | `Mission::Goto { x, y }`；删 `Circle` 与 `members` |
| 2 | `core/protocol/messages.rs:59-71` | 改 | `TaskSetPayload { missions }`（删 `members`） |
| 3 | `core/protocol/messages.rs:265-286` | 改 | `encode_task_set`：`count(1) + 9n`，去 member 段 |
| 4 | `core/protocol/messages.rs:291-327` | 改 | `decode_task_set`：去 member 循环；校验 `payload.len() == 1 + 9*count` |
| 5 | `core/protocol/command_decode.rs:27-70` | 改 | 三分支 → 单分支（见 12.3.4） |
| 6 | `core/protocol/messages.rs` 测试（5 例） | 改 | 帧长断言 `1+9n`；旧布局帧 → `None` |
| 7 | `core/protocol/command_decode.rs` 测试（7 例） | 改 | 与 12.3.6 对齐 |

**设备端（编译级，不接树）**

| # | 文件 | 动作 | 内容 |
|---|---|---|---|
| 8 | `pleiades-ugv/src/ugv/goal.rs` | 改 | `match` 去 `members` / `Circle`；删两处 `assignment` 调用与失败分支（`:96-101`、`:112-116`）；删 `own_peer_id` 字段与构造参数 |
| 9 | `pleiades-uav/src/uav/goal.rs` | 改 | 同上（行号同构） |
| 10 | `pleiades-sim/src/sim/goal.rs` | 改 | 同上（`:206-229` 区域） |
| 11 | `planning/assignment.rs` ×3 | **删** | 425 行 ×3 |
| 12 | `planning/mod.rs` ×3 | 改 | 去掉 `assignment` 的 re-export |
| 13 | `pleiades-sim/src/sim/control.rs` | 改 | `:403/:414` 只构造 `Goto`；`:663-680` 删 members 的 JSON 校验；`:687` 断言同步 |
| 14 | 三个 `robot_handler.rs` | 改 | 只改 `GoalService::new(...)` 的实参（少一个 `own_peer_id`） |

**文档**

| # | 文件 | 内容 |
|---|---|---|
| 15 | `docs/design_doc/orion_protocol.md:272-310` | §3.5 载荷布局、总大小、三分支表 → 新单分支语义 |
| 16 | `Architecture/robot_arch.md:398`、`:415` | "members 自动填充"与"三分支"两处描述 |

#### 12.3.4 新载荷布局与语义

```text
ORION_TASK_SET (msgid 5) payload
  mission_count u8
  missions[mission_count] × 9B： type u8 | x f32 BE | y f32 BE

总长 = 1 + 9 × mission_count
```

| 条件 | 语义 |
|---|---|
| `mission_count == 0` | **取消**（空队列替换，停车待命）——唯一取消入口 |
| `mission_count > 0`，`type == 0` | `Mission::Goto { x, y }` 逐条入队 |
| `mission_count > 0`，`type != 0` | 该条 `warn` + 丢弃；**若全部被丢 → 整帧不生效（不动队列）** |
| 长度 ≠ `1 + 9×count` | `decode_task_set` 返回 `None`（旧帧即在此被拒） |

#### 12.3.5 详细步骤

1. `element.rs`：`Mission` 改单变体（先改类型，让编译器列出所有消费点）。
2. `messages.rs`：`TaskSetPayload`、`encode`、`decode` 三处。
3. `command_decode.rs`：三分支合一 + "全无效不动队列"。
4. 按编译错误逐个修设备端（三个 `goal.rs`、三个 `robot_handler.rs`、`sim/control.rs`）。
5. 删三份 `assignment.rs` + 改 `planning/mod.rs`。
6. 重写 base 的两组测试（12 个用例）。
7. `cargo check --workspace` + `cargo test --workspace`。
8. 同步两处文档。

#### 12.3.6 单测

| # | 用例 | 断言 |
|---|---|---|
| 1 | 新布局 roundtrip | `count=1 / 3` 编解码一致 |
| 2 | 取消 | `count=0` → `Command::Auto(Set(vec![]))` |
| 3 | 旧布局帧（含 members） | `decode_task_set → None`（长度不符） |
| 4 | 长度不足 / 多 1 字节 | `None` |
| 5 | 全未知 `type` | `parse_orion_frame → None`（**不动队列**） |
| 6 | 部分未知 `type` | 只保留合法条目，其余 `warn` |
| 7 | 上限 255 条 | 编解码一致（不 panic） |

#### 12.3.7 验收标准

1. `cargo check --workspace`、`cargo test --workspace` 全绿。
2. `grep -rn "members" pleiades-*/src` 无残留（Pictor 目录除外）。
3. `grep -rn "assignment" pleiades-*/src` 无残留；三份 `assignment.rs` 不存在。
4. 单车 `Goto` 链路行为与改造前逐字等价（除"旧帧被拒"）。
5. `Mission` 只有一个变体；`Circle` 全仓消失。

#### 12.3.8 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | Step 3~7 之间车 ↔ Pictor 命令链路断（旧帧必被拒） | 预期行为（同批上线）；sim 走直接注入不受影响；Step 7 恢复 |
| 2 | 误把"解析后为空"当取消 | 12.3.4 写死：只有 `count == 0` 是取消；用例 5 锁住 |
| 3 | 删 `own_peer_id` 后构造参数变化波及三个 handler | 编译器带路；改动只是删一个实参 |
| 4 | 文档没同步，Pictor 按旧布局发 | 文档与代码同批；Step 7 一并改 Pictor |

### Step 4 — ugv 迁移（`nav/` + `mission/` + base 接线）

#### 12.4.1 目标

1. **base 接线**（W1~W4）：`on_tick` 返回 `TickOutcome`；新增 `build_task`；`main_loop` 负责"发现队首变化 → 建树 → 收终值 → 出队"。
2. **ugv**：`goal.rs` → `nav/navigator.rs`；`planning/{pathfinder,cluster_obstacles}` → `nav/`；新增 `mission/`（Goto 树）；`robot_handler` 改成驱动。
3. **uav / sim**：只做编译级 stub（`build_task` 空操作、`on_tick` 恒 `Running`），**旧链路一行不动**。

#### 12.4.2 非目标（硬边界）

| 不做 | 归属 |
|---|---|
| 不迁 uav / sim 的导航与树 | Step 5 / Step 6 |
| 不改 `executor.rs` / `stm32/` / `lidar/` / `slam/` | 一行不动 |
| 不新增任务类型、不改协议 | §七 #11 / Step 3 |
| 不做黑板、不做速度控制 | v1 不做 |

#### 12.4.3 base 的接口（W1~W4）

```rust
// robot.rs
pub enum TickOutcome {
    Running,
    Terminated(Option<FailCode>),   // None = 成功；Some(code) = 失败 + 原因
}

#[async_trait]
pub trait DeviceHandler: Send + Sync {
    // ... 原有 start / handle_manual_cmd / reset / stop / shutdown
    /// 任务开始：为该任务建树。
    /// **幂等**：该 TaskId 已有实例（抢占恢复）→ 只激活，不重建。
    async fn build_task(&self, task: &QueuedTask) -> Result<(), String>;
    async fn on_tick(&self, rs: &RobotState, es: &Arc<RwLock<ExecuteState>>) -> TickOutcome;
}
```

`main_loop` 的 `auto_tick` 分支（伪代码）：

```text
let head = mission_queue.read().head().cloned()      // 队首不离队
if let Some(task) = head && Some(task.id) != current_id {
    match device.build_task(&task).await {
        Ok(())  => current_id = Some(task.id),
        Err(e)  => {                                 // 建树失败 = 停住待命（§十一 #35）
            mission_queue.clear(); device.reset().await; current_id = None;
        }
    }
}
match device.on_tick(&rs, &es).await {
    Running          => {}
    Terminated(code) => { mission_queue.pop_next(); current_id = None; }   // 唯一出队口
}
```

- `current_task_id` 是 `main_loop` 的局部变量；`SwitchToManual` 与 `AutoCmd::Set` 两个分支已经调用 `device.reset()`，**同时置 `None`**。

#### 12.4.4 ugv 目录与文件

```text
pleiades-ugv/src/
├── main.rs / config.rs / bootstrap.rs     不变
├── device/lg290p/                         不变
└── ugv/
    ├── mod.rs                   改：planning/goal → nav/mission
    ├── types.rs                 不变
    ├── stm32/ lidar/ slam/      不变
    ├── executor.rs              不变
    ├── emergency_stop.rs        改 1 处：goal_service → nav
    ├── nav/                     ★ 新增
    │   ├── mod.rs               NavStatus / BlockedCause
    │   ├── navigator.rs         原 goal.rs（去 pop、去 assignment）
    │   ├── pathfinder.rs        原 planning/pathfinder.rs（原样搬）
    │   └── cluster_obstacles.rs 原 planning/cluster_obstacles.rs（原样搬）
    ├── mission/                 ★ 新增
    │   ├── mod.rs               任务 → 树的分派 + TaskDeps
    │   └── goto.rs              Goto 树 + GoTo 叶子
    ├── planning/                ✗ 删除
    ├── goal.rs                  ✗ 删除
    └── robot_handler.rs         改：驱动接线
```

#### 12.4.5 `Navigator`

```rust
pub enum BlockedCause { Dynamic, NoPath }
pub enum NavStatus { Idle, Navigating, Arrived, Blocked(BlockedCause) }
/// 障碍策略：三处 `navigator.rs` 保持同一份代码，差异只走这个参数
pub enum ObstacleMode { Respect, Ignore }     // ugv/sim = Respect；uav = Ignore（Task 27）

impl Navigator {
    pub fn new(
        robot_state, grid, cluster_table,
        obstacle_mode: ObstacleMode,          // 见上
        obstacle_inflation_radius, arrival_threshold_m,
    ) -> Self;
    pub fn set_goal(&mut self, x: f32, y: f32);        // 清终态、建 D*
    pub fn clear_goal(&mut self);
    pub fn step(&mut self, rs: &RobotState) -> Option<(i32, i32)>;   // 每拍推进，产出下一格
    pub fn status(&self) -> NavStatus;                 // 终态锁存到下一次 set_goal
    pub async fn mark_obstacle(&mut self, cell: (i32, i32));         // 急停修补，保留 goal
}
```

> **建图增量**一律做成 `Option`：ugv / uav 传 `None`，sim 传 `Some`（Step 6）。这样三份 `navigator.rs` 可以逐字相同，差异全在构造参数上。

行为要点：

| 点 | 约定 |
|---|---|
| 每拍 | `step` 内部：到达判定（世界距离）→ 格级兜底 → 动态障碍 + 建图增量 + `next_step` |
| **无路** | **不清 goal**，继续每拍重算；**连续 `BLOCKED_AFTER_TICKS = 20`（1 s）** 仍无路 → `status = Blocked(cause)`，期间保持 `Navigating` |
| `cause` | 失败那一拍把动态障碍清空再算一次：能走通 → `Dynamic`；仍不行 → `NoPath` |
| 到达 | `status = Arrived` 并清 goal/pathfinder，**锁存**到下一次 `set_goal` |
| 急停 | `mark_obstacle` 只打补丁，**保留 goal**（让恢复后能继续走） |

#### 12.4.6 Goto 树

```text
GoTo(x, y)                                   ← 每任务一棵（mission::build）
└─ Timeout(60_000 ms)                        ← 业务超时
   └─ Fallback
      ├─ GoTo(x, y)                          ← 主路
      └─ Sequence                            ← 恢复路（只重试一次）
         ├─ Wait(3_000 ms)
         └─ GoTo(x, y)
```

叶子 `GoToLeaf { x, y, armed, nav, robot_state }`：

```text
tick(f):
  if !armed || nav.lock().status() == NavStatus::Idle {    // 未武装，或目标被清过（抢占恢复）→ 重设
      nav.lock().set_goal(x, y); armed = true
  }
  let rs = robot_state.read().await;
  let cell = nav.lock().step(&rs);
  match nav.lock().status() {
      Arrived            => { f.out.next_cell = None; Success }
      Blocked(Dynamic)   => { f.out.next_cell = None; Failure(FailCode::Blocked) }
      Blocked(NoPath)    => { f.out.next_cell = None; Failure(FailCode::Unreachable) }
      _                  => { f.out.next_cell = cell; Running }
  }
halt(): armed = false
```

- **上报不在树里**：失败码随 `Status` 回到驱动，由驱动记录/上报（§十一 #32）。
- 树的默认参数（60 s / 3 s）写在 `goto.rs` 顶部常量，便于后续调。

#### 12.4.7 `on_tick` 五步

| 步 | 做什么 | 返回 |
|---|---|---|
| ① 门闩 | 无底盘 / 未装配 → 写 `ExecuteState{Idle,None}`，什么都不做 | `Running` |
| ② 急停 | 命中 → `stop()` + 写 `Idle`，**保留 goal**；**树这一拍不 tick**（时间冻结） | `Running` |
| ③ 树 | 当前任务树 `tick(&mut frame)`；无树则跳过 | — |
| ④ 决策 | `decide(frame.out.next_cell, …)` → 写 `ExecuteState` → `apply_action` | — |
| ⑤ 终值 | 树返回终值 → `halt` 树、从实例表删除、返回 `Terminated(code)` | `Terminated` |

`reset()` = halt 当前树 + 清实例表 + `nav.clear_goal()` + `stm32.stop()` + 写 `Idle/None`。

> **抢占要用另一个入口 `suspend()`**（Step 8 引入）：**不停树**（保留节点进度）→ 清 `nav` → 停车 → 写 `Idle` → **保留实例表**。区别：`reset` 丢弃实例（`replace`/切模式），`suspend` 保留实例（`preempt`）。

#### 12.4.8 详细步骤

1. base：加 `TickOutcome`、trait 两个方法、`auto_tick` 逻辑；`TickOutcome` 先让三个 handler 返回 `Running` 以便编译。
2. uav / sim：`build_task` 空实现（`Ok(())`）、`on_tick` 恒 `Running`——**它们的旧 `goal.rs` 继续自己出队**，base 的出队分支对它们是死路径。
3. ugv：`goal.rs` → `nav/navigator.rs`（删 pop / 删 assignment 调用 / 加 status / 加 20 拍去抖）。
4. ugv：`planning/{pathfinder,cluster_obstacles}.rs` → `nav/`；删 `planning/`。
5. ugv：`mission/mod.rs` + `mission/goto.rs`。
6. ugv：`robot_handler.rs` 接线（`CarInner` 加 `nav` / `trees`；`on_tick` 五步；`reset`；`build_task`）。
7. 单测（12.4.9）与 `cargo check/test --workspace`。

#### 12.4.9 单测

| # | 用例 | 断言 |
|---|---|---|
| 1 | `Navigator` 世界距离到达 | 目标 0.3 m 内 → `Arrived`，goal 被清 |
| 2 | `Navigator` 格级兜底 | 已入目标格但距离 > 0.3 → `Arrived` |
| 3 | `Navigator` 去抖 | 无路 19 拍 → 仍是 `Navigating`；第 20 拍 → `Blocked` |
| 4 | `Navigator` cause | 清空动态障碍后可走 → `Dynamic`；仍不行 → `NoPath` |
| 5 | 叶子重新武装 | `halt` 后再 tick → 重新 `set_goal` |
| 6 | Goto 树主路 | 假端口第 3 拍 `Arrived` → 树 `Success` |
| 7 | Goto 树恢复路 | 主路 `Blocked` → 等 3 s → 第二次成功 |
| 8 | Goto 树超时 | 一直 `Navigating` → 60 s 后 `Failure(Timeout)` |
| 9 | base 建树时机 | 队首 id 变化才调 `build_task`；同 id 不重复调 |
| 10 | base 出队 | `Terminated(None)` → `pop_next` 一次，且只有一次 |
| 11 | base 建树失败 | `build_task` 返回 `Err` → 队列被清空、`reset` 被调用 |

#### 12.4.10 验收标准

1. `cargo check --workspace`、`cargo test --workspace` 全绿。
2. `grep`：ugv 下无 `assignment`、无 `goal.rs`、无 `planning/`。
3. **无双弹**：ugv 的 `on_tick` 里不出现 `pop_next`；出队只在 `main_loop`。
4. **单点写**：`ExecuteState` 的写入仍只在 `robot_handler.rs`。
5. **急停不进树**：`on_tick` ② 命中时树未被 tick（用计数或 mock 验证）。
6. **幂等建树**：同一 `TaskId` 连续两次 `build_task` → 只有一个实例（抢占恢复的基础）。

#### 12.4.11 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | double-pop（设备还留着旧的 pop） | 验收第 3 条 grep 卡死 |
| 2 | 短路期 uav/sim 的 `build_task` 是空实现 → 若它们的 `on_tick` 误返回 `Terminated` 会多弹一次 | stub 恒 `Running`；验收第 3 条只查 ugv，但评审要看到 uav/sim 的 stub |
| 3 | `Navigator` 行为变了（永不放弃 + 20 拍去抖） | 实车由人类验证；单测 1~4 锁住语义 |
| 4 | 急停时树不 tick → `Timeout` 的计时被冻结 | 这是有意的（急停不算执行时间）；若实车发现"堵久了任务不再超时"，再议 |
| 5 | `build_task` 非幂等 → 抢占恢复时重建树、丢进度 | 验收第 6 条 + `mission::build` 只在新 id 时调用 |

### Step 5 — uav 迁移（与 ugv **同构**）

#### 12.5.1 目标

把 Step 4 在 ugv 上跑通的结构**照搬**到 uav：`nav/`（navigator + pathfinder + cluster_obstacles）、`mission/`（Goto 树），`robot_handler` 接线。base 在 Step 4 已改完，本步**不动 base**。

#### 12.5.2 必须保留的 uav 差异（登记在案）

| 差异 | 现状 | 本步怎么处理 |
|---|---|---|
| **喂空 grid** | `uav/goal.rs:139-156` 注释掉动态障碍、用空 `OccupancyGrid`（Task 27"天上没人"） | 用 Step 4 已备好的 `ObstacleMode::Ignore` 参数，**不 fork 代码**。⚠️ **过渡参数**：uav 的导航模式后续会改版，届时整体替换（不要把它当最终形态） |
| **无 2D 急停** | uav 无 `emergency_stop.rs`、无 `lidar` | `on_tick` 少第 ② 步（直接 ①③④⑤） |
| **无建图增量** | uav 无 `slam` | `Navigator` 的建图增量订阅为 `Option`，uav 传 `None` |
| **执行前门闩** | 无 `mavlink` → 写 Idle | 原样保留，返回 `Running` |

#### 12.5.3 改动文件

```text
pleiades-uav/src/
├── main.rs / config.rs / bootstrap.rs   不变
├── device/camera_*                      不变
└── uav/
    ├── mod.rs                  改：planning → nav / mission
    ├── mavlink/                不变
    ├── executor.rs             不变（三份逐字节相同）
    ├── nav/                    ★ navigator.rs / pathfinder.rs / cluster_obstacles.rs / mod.rs
    ├── mission/                ★ mod.rs（分派 + TaskDeps）/ goto.rs（**与 ugv 逐字相同**）
    ├── planning/               ✗ 删除
    ├── goal.rs                 ✗ 删除
    └── robot_handler.rs        改：接线（无急停）
```

#### 12.5.4 与 ugv 的一致性要求

| 项 | 要求 |
|---|---|
| `mission/goto.rs` | **与 ugv 逐字相同**（只允许 `use` 路径不同），验收用 `diff` |
| `mission/mod.rs` | 分派结构相同；`TaskDeps` 字段相同 |
| `nav/navigator.rs` | **结构与 Step 4 逐字相同**；差异只允许出现在构造参数（`ObstacleMode::Ignore`、`map` 订阅 `None`），不允许代码分叉 |
| `nav/pathfinder.rs` | 与 ugv **逐字节相同**（当前 ~401 行版本） |
| `executor.rs` | 不动（本来就逐字节相同） |

#### 12.5.5 验收

1. `cargo check --workspace`、`cargo test --workspace` 全绿。
2. `diff pleiades-ugv/src/ugv/mission/goto.rs pleiades-uav/src/uav/mission/goto.rs` 只差 `use` 行。
3. `diff` 两份 `pathfinder.rs` 为空。
4. 实车验证由人类执行（§十一 #30）。

### Step 6 — sim 迁移（`ugv_sim/` + `uav_sim/` 双栈）

#### 12.6.1 目标

1. `pleiades-sim/src/sim/` 下新增 **`ugv_sim/`** 与 **`uav_sim/`**，各自复刻对应真机的 `nav/` + `mission/`。
2. `sim_handler` 按底盘类型选择当前活动栈，`build_task` / `on_tick` / `reset` 转发给它。
3. **树的文件与真机逐字相同**（§十一 #29）；`navigator` 保留 sim 现有的到达判据（§十一 #37）。
4. **端到端验收在本步**（这是整个改造第一次能在仿真里跑完整链路）。

#### 12.6.2 非目标

| 不做 | 归属 |
|---|---|
| 不动 `chassis/` / `sensors/` / `world_link/` / `emergency_stop.rs` | 世界侧，与任务层无关 |
| 不动 `executor.rs` | 共用一份（本来就逐字节相同） |
| 不改 base | Step 4 已改完 |
| 不改 Pictor | Step 7 |

#### 12.6.3 目录

```text
pleiades-sim/src/sim/
├── ugv_sim/                      ★ 新增（复刻 pleiades-ugv）
│   ├── mod.rs                    Stack：持有 nav + 实例表，实现 build/tick/halt/reset
│   ├── nav/  mod.rs / navigator.rs / pathfinder.rs / cluster_obstacles.rs
│   └── mission/  mod.rs / goto.rs        （与 ugv **逐字相同**）
├── uav_sim/                      ★ 新增（复刻 pleiades-uav）
│   ├── mod.rs
│   ├── nav/  ...
│   └── mission/  mod.rs / goto.rs        （与 uav **逐字相同**）
├── sim_handler.rs                改：持有 `enum TaskStack { Ugv(..), Uav(..) }` 并转发
├── executor.rs                   共用，不动
├── goal.rs                       ✗ 删除（能力并入两套 navigator）
└── chassis/ sensors/ world_link/ emergency_stop.rs   不动
```

#### 12.6.4 两套栈的差异（登记在案）

| 项 | `ugv_sim` | `uav_sim` |
|---|---|---|
| 树的文件 | 与 `pleiades-ugv/src/ugv/mission/` 逐字相同 | 与 `pleiades-uav/src/uav/mission/` 逐字相同 |
| `ObstacleMode` | `Respect` | `Ignore` |
| 到达判据 | **预测停点 + 逐轴 0.06 / 0.26 + 格内逼近**（sim 现状，与真机的 0.3 m 欧氏不同——§十一 #37 允许） | 同左 |
| 底盘 | 世界差速底盘 | 世界 UAV 零件 |
| 急停 | 有（世界雷达） | 无 |

> 两份 `navigator.rs` **彼此**逐字相同（都来自 sim 现有 `goal.rs` 的能力 + 真机的结构），差异只有构造参数。

#### 12.6.5 栈的选择

`chassis::Build(world_name, …)`（`sim_handler.rs:161`）已经拿得到底盘类型。二选一：

- 给 `Chassis` trait 加 `fn Kind(&self) -> ChassisKind`（推荐，显式）；
- 或在 `start()` 里按 `world_name` 判断（省一次 trait 改动）。

`sim_handler` 在 `start()` 里选定 `TaskStack`，之后 `build_task` / `on_tick` / `reset` 全部转发。

#### 12.6.6 详细步骤

1. 从 `pleiades-ugv/src/ugv/{nav,mission}` 拷到 `sim/ugv_sim/`，改 `use` 路径为 `crate::sim::...`。
2. 把 sim 现有 `goal.rs` 的到达判据（`Predict_Stop` / `Grid_Arrived` / 格内逼近 / 建图增量）并入两套 `navigator.rs`。
3. 从 `pleiades-uav/src/uav/{nav,mission}` 拷到 `sim/uav_sim/`。
4. `sim_handler`：加 `TaskStack`、按底盘类型选栈、转发三个方法。
5. 删 `sim/goal.rs`；`sim/mod.rs` 加两个模块。
6. `cargo check --workspace` → 修编译 → `cargo test --workspace`。
7. 端到端（12.6.8）。

#### 12.6.7 单测

| # | 用例 | 断言 |
|---|---|---|
| 1 | `ugv_sim` 栈的 Goto | 假底盘 + 序列位姿 → 到达后树 `Success` |
| 2 | `uav_sim` 栈的 Goto | 同上，且 `ObstacleMode::Ignore` 生效（动态障碍不影响路径） |
| 3 | 栈选择 | 差速底盘 → `Ugv`；UAV 零件 → `Uav` |
| 4 | 抢占恢复 | 同一 `TaskId` 两次 `build_task` → 只有一个实例 |

#### 12.6.8 端到端验收（本步唯一能真跑的）

| # | 场景 | 判据 |
|---|---|---|
| 1 | 单车 Goto | 到达后停稳，逐轴误差 ≤ 0.3 m（对齐 §九.1） |
| 2 | 两条 Goto 顺序执行 | 第一条到达后自动出队，第二条开始（验证终值→出队） |
| 3 | 不可达目标 | 60 s 内 `Timeout` 失败，队列前进到下一条，日志有失败码 |
| 4 | 中途 `replace` | 一个 tick 内停车、朝新目标（验证 halt） |
| 5 | 无任务 | 车静止、位姿照常广播（§十一 空转语义） |

#### 12.6.9 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | 两套栈的 `ObstacleMode` 传反 | 单测 3 卡住；评审看构造参数 |
| 2 | 拷过来的 `navigator.rs` 改了真机没有的代码 → 与真机漂移 | `diff` 校验 + 差异登记表（12.6.4） |
| 3 | 到达判据与真机不同 → “仿真过了真机不过” | 已由 §十一 #37 接受；停点精度由实车验 |
| 4 | 建图增量只接在 sim 的 navigator 上，真机没有 → 又是差异 | 登记；后续可考虑反哺真机 |

### Step 7 — Pictor（协议同步 + 逐车下发）

#### 12.7.1 目标

1. `Encode_Task_Set` / `Decode_Task_Set` 去掉 `member_count` / `members`（与 Step 3 的新布局对齐）。
2. `KernelBridge` 删掉"从 targets 自动填 members"的逻辑。
3. Goto / Circle 的命令改**逐车 emit**，每帧只发一辆车。
4. 新增"逐车算点"的位置：接口先留出，算法见 12.7.5。

#### 12.7.2 非目标

| 不做 | 归属 |
|---|---|
| 不改 Rust 桥 `pleiades-terminal` | 它只搬字节（`lib.rs:125-136`），无需改 |
| 不改 UI 布局 / 渲染 / 地图 | 与协议无关 |
| 不实现封控的几何算法 | 12.7.5 只留接口 + 占位实现 |

#### 12.7.3 改动文件

| # | 文件 | 动作 |
|---|---|---|
| 1 | `src/websocket/protocol/orion_messages.gd:163-199` | `Encode_Task_Set` 去 member 段 |
| 2 | 同文件 `:204-230` | `Decode_Task_Set` 去 member 段 + 长度校验改 `1 + 9n` |
| 3 | `src/kernel/kernel_bridge.gd:62-67` | 删 members 填充；`Build_Cmd` 直接组帧 |
| 4 | `src/control/auto_handler.gd:139-140`、`:155` | 改成"对每个 target 单独 `cmd_send([id], 该车的任务)`" |
| 5 | 新增（建议）`src/control/target_alloc.gd` | `compute_targets(targets: Array[String], anchor: Vector2, shape) -> Dictionary[String, Vector2]` |
| 6 | `src/test/test_orion_protocol.gd:297,304` | 断言改成新布局 |
| 7 | `docs/orion_protocol.md`（Pictor 仓内） | §TASK_SET 布局同步 |

#### 12.7.4 关键点

- **坐标系**：协议是 `x 东、y 南，米`；yaw `顺时针为正、0 = +x 东`。但 `auto_handler.gd:50` 和 `llm.gd:42` 的注释写的是"0 = 正北，90 = 正东"——**算偏移时以协议为准**，别照注释算，否则差 90°。
- **健全性门控**：`VehicleRegistry` 里"刚注册还没收到 pose"的车 `x/y = 0`，算落点会把它分到原点附近。算点前必须过滤：无位姿 / 位姿过期的车直接拒绝，并在 UI 上提示。
- **RTK 门控**：`VehicleRegistry` 不存 `rtk_fixed`，若将来落点精度依赖 RTK，需要补存。

#### 12.7.5 "逐车算点"的算法（**待定**）

第一版只做**管道**，算法占位：

```gdscript
# 占位：所有车去同一个锚点（仅保证管道跑通，不推荐长期使用）
func compute_targets(targets: Array[String], anchor: Vector2, shape: String) -> Dictionary:
    var out := {}
    for id in targets:
        out[id] = anchor
    return out
```

真正的分配（按形状铺开、按序号偏移、按 peer 排序）等封控需求明确后再实现。**接口先定**，避免下次又改 `auto_handler`。

#### 12.7.6 验收

1. 在 Pictor 里选 3 台车发 Goto → 抓到 3 帧、每帧 `member_count` 字段不存在、`mission_count = 1`。
2. 车端（Step 3 之后的解析器）能正常执行（与 Step 6 的 sim 联调）。
3. `test_orion_protocol.gd` 通过。
4. 旧格式帧（带 members）在 Pictor 侧已无法生成。

#### 12.7.7 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | 协议改动与车端不同批 → 联调失败 | Step 3 + Step 7 同批上线（§十一 #28） |
| 2 | 坐标系 / yaw 约定用错 → 点位偏移 90° | 12.7.4 写死"以协议为准"；联调用 sim 看实际落点 |
| 3 | 无位姿的车被分配 → 落点跑到原点 | 算点前过滤 + UI 提示 |
| 4 | 占位算法被误当正式实现 | 函数注释写明"占位"，封控任务里替换 |

### Step 8 — sim 端到端验证（队列语义 + 逐任务执行 + 双栈）

#### 12.8.1 目标

用 sim 把**队列语义**与**任务执行**跑一遍，`ugv_sim` 与 `uav_sim` 两套栈各跑一轮。覆盖：逐条执行 / `append` / `preempt`（含恢复与嵌套）/ `replace` / 取消 / 不可达超时。

#### 12.8.2 本步引入的前置改动

| # | 改动 | 为什么必须在本步做 |
|---|---|---|
| 1 | `AutoCmd` 增 `Append` / `Preempt`；`main_loop` 映射成队列操作 | 队列语义已实现，但没有命令入口，e2e 无法触发 |
| 2 | `DeviceHandler::suspend()`（与 `reset()` 区分） | `preempt` 要**保留实例**，`reset` 会丢弃 |
| 3 | sim 控制口增 `append` / `preempt` / `queue` 命令 | 测试注入通道（`TASK_SET` 只能表达 replace） |
| 4 | `status` 增 `queue_len` / `task_id` / `task_ticks` / `nav_status` | 没有这些字段就**无法判定**抢占是否从暂停点继续 |

> 协议**仍不暴露** `Append` / `Preempt`（`TASK_SET` 只有 replace）。这两个变体目前只由控制口/测试使用；将来地面站要用，需要新 msgid。

#### 12.8.3 测试矩阵

| # | 场景 | 操作 | 期望 |
|---|---|---|---|
| 1 | 逐条执行 | `replace [A(远), B(近)]` | A 到达 → 自动出队 → B 开始 → B 到达 → 队列空、停车 |
| 2 | append 不打断 | 发 A；行驶 3 s 后 `append B` | A **不停、不重来**（位置连续前进）；A 完成后 B 开始 |
| 3 | append 空队列 | 队列空时 `append A` | A 立即开始执行 |
| 4 | preempt 让位 | 发 A（远）；走一半 `preempt B` | **1 个 tick 内停车**；`task_id` 变 B；`queue_len` = 2 |
| 5 | **preempt 恢复** | 承接 #4，B 到达后 | `task_id` 回到 A；**`task_ticks` 从暂停时的值继续（不归零）** → 证明是同一实例，不是重建 |
| 6 | preempt 嵌套 | A 中 preempt B，B 中 preempt C | 恢复顺序 C → B → A；三者的 `task_ticks` 都不归零 |
| 7 | replace 丢弃 | A 执行中 `replace [C]` | A 丢弃、`task_id` = C；C 完成后队列空（A 不回来） |
| 8 | 取消 | `replace []` | 立即停车；`queue_len` = 0；`task_id` 为空 |
| 9 | 不可达 | Goto 到障碍格 | 恢复路重试一次 → 60 s `Timeout` → 失败上报 → 出队 → 下一条开始 |
| 10 | 双栈 | 1~9 在 `uav_sim` 上重跑 | 结果相同（到达判据按 sim 的预测停点，不卡厘米级） |

#### 12.8.4 观测手段

- **控制口 `status`**（扩展后）：每 200 ms 采一次 `queue_len` / `task_id` / `task_ticks` / `nav_status` / `x,y`；
- **日志**：任务开始/结束、出队、失败码。

判定 `preempt` 是否“从暂停点继续”，**只认 `task_ticks`**：暂停期间不 tick，所以恢复首拍的值应当等于暂停时的值（重建则从 0 开始）。

#### 12.8.5 脚本与产物

| 产物 | 位置 | 说明 |
|---|---|---|
| 测试脚本 | `Tool/sim_queue_test.py`（新增，入仓） | 连控制口，按矩阵发命令 + 采样 + 输出 PASS/FAIL 表 |
| 运行结果 | `RobotSim_TestEnv/results/`（gitignored） | 每次运行的 JSON/日志快照 |

#### 12.8.6 验收标准

1. 10 个用例**全部 PASS**，`ugv_sim` 与 `uav_sim` 各一轮。
2. 用例 5/6 的 `task_ticks` **不清零**（抢占恢复成立）。
3. 用例 4 的停车延迟 ≤ 1 个 tick（50 ms）。
4. 用例 7/8 之后 `queue_len` 与 `task_id` 与预期一致。
5. 全程日志无“任务被静默丢弃”的 `warn`。

#### 12.8.7 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | `append` / `preempt` 无协议入口，只能测试注入 | 文档写明“协议暂不暴露”；将来地面站要用需新 msgid |
| 2 | `suspend` 误用成 `reset` → 抢占后实例被清，恢复变重来 | 用例 5 用 `task_ticks` 卡死 |
| 3 | `task_ticks` 未实现 → 无法区分“恢复”与“重来” | 12.8.2 第 4 条列为前置 |
| 4 | UAV 栈的 sim 启动方式（世界绑定哪个零件）不明确 | 按 Step 6 的 `TaskStack` 选择实现；启动命令实现时确认 |
| 5 | 到达判据差异导致 UAV 用例判定口径不同 | 用例只判“到达 + 不超时”，不卡厘米 |

### 全部步骤总览

| Step | 内容 | 状态 |
|---|---|---|
| 1 | mission_queue（base） | 已定稿 |
| 2 | behavior_tree 引擎（base） | 已定稿 |
| 3 | 协议升版 + 全仓删协同 | 已定稿 |
| 4 | ugv 迁移 + base 接线 | 已定稿 |
| 5 | uav 迁移（同构） | 已定稿 |
| 6 | sim 迁移（`ugv_sim` / `uav_sim` 双栈）+ 端到端验收 | 已定稿 |
| 7 | Pictor（协议同步 + 逐车下发） | 已定稿 |
| 8 | sim 端到端验证（队列语义：append / preempt / replace / 逐条执行，双栈各一轮） | 已定稿 |
