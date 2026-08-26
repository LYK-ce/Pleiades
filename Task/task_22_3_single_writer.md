# task_22_3_single_writer — 单写者收敛：Lua 纯决策 + main_loop 单点执行

> 状态：**已实施**（2026-08-21），编译 + robot 122 测试通过
> Created Date ： 2026-08-21
> Modified Date ： 2026-08-21
> 依赖：`Task/task_22_2_lua_state_machine.md`
> 分支：`Universal-Robot`（稳定分支 `Pleiades-Orion` / `Godot-Library` 不动）

---

## 一、目标

根治 task_22_2 三态机的**卡死回归**，以及其后暴露的**两个执行流并发竞争**。方案：**单写者收敛**——Lua 变成真正的纯决策函数（零副作用），main_loop 成为唯一执行者（含 `get_path` 的任务副作用、写状态、发命令、急停仲裁）。

本版相对上一版的修正：① generation 打标机制（R1）；② invalidate 助手统一四处调用（R2）；③ **get_path 移入 main_loop**，Lua 收参数、真正零副作用（R3-B）；④ `get_state` 为新增 caps（R4）；⑤ 决策线程看门狗（R5）；⑥ 命令去重由 Lua 返回 `action=none` 表达（R7）。

## 二、背景：两个层面的问题

1. **卡死回归（状态分离）**：task_22_2 状态机在 Lua upvalue，Rust 侧重置只清 Rust 侧 → 急停 / 任务替换 / 模式切换三个卡死路径（T22-1 / N-1）。
2. **并发竞争（两个执行流抢"做"）**：Lua 决策线程与 main_loop 都能写状态、发命令，无仲裁 → 陈旧 tick 顶掉急停 stop、在途决策丢失更新、车在障碍前 50ms 脉冲前拱。
3. **隐藏矛盾**：`world.get_path()` 有任务副作用（pop mission / set goal / set_goal / clear_goal），若由 Lua 调用，则「Lua 零副作用」不成立，`goal/mission_queue/pathfinder` 仍是双写者。

## 三、方案

### 3.1 核心：想与做分离（彻底版）

| | 角色 | 职责 | 副作用 |
|---|---|---|---|
| **Lua（car.lua）** | 导航员 | 读状态 + 收 `next_cell` 参数 → 算决策 → **返回 DecisionResult** | **零** |
| **main_loop（Rust）** | 唯一司机 | 调 `get_path`（任务副作用）+ 写状态 + 发命令 + 急停仲裁 + 过期丢弃 + 看门狗 | 全部集中于此 |

**关键变化（R3-B）**：`get_path` 从 Lua 侧移到 main_loop——main_loop 每 tick 自己调 `get_path` 拿「下一格」，作为参数传给 `on_tick(next_cell)`。这样任务副作用（pop/set goal）与 `reset`/`invalidate` 同源，都在 main_loop 单线程串行，**消除第二个写入流**。

### 3.2 数据结构（`state.rs`）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecisionState { #[default] Idle, Turning, Moving }

/// 动作（带载荷，语义清晰，R8）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionAction {
    MoveForward(i16),
    MoveBackward(i16),
    TurnLeft(i16),
    TurnRight(i16),
    Stop,
}

/// Lua on_tick 的返回值（不含 generation，R1）
#[derive(Debug, Clone)]
pub struct DecisionResult {
    pub state: DecisionState,
    pub sub_target: Option<(i32, i32)>,
    pub action: Option<MotionAction>,   // None = 保持、不发命令（命令去重，R7）
}

/// ExecuteState：决策状态机（唯一写者 = main_loop）
#[derive(Debug, Clone, Default)]
pub struct ExecuteState {
    pub state: DecisionState,
    pub sub_target: Option<(i32, i32)>,
}
```

### 3.3 Lua 纯函数（`car.lua`）

`on_tick` 收 `next_cell` 参数、只读状态、返回结果表；**不调 `get_path`、不调 `action.*`、不写状态**：

> 实施注意（伪代码为简洁省略字段展开，落地必须逐字段访问）：`next_cell`/`st.sub_target` 是 `{gx,gy}` 表；`angle_to_target(gx, gy, x, y, yaw)` 与 `cell_center(coord)` 是 helper，调用时写 `angle_to_target(next_cell.gx, next_cell.gy, x, y, yaw)`、`cell_center(st.sub_target.gx)`；判断“目标变了”用字段比较 `next_cell.gx ~= st.sub_target.gx or next_cell.gy ~= st.sub_target.gy`，**勿用 `~=` 比较表引用（恒 true 会死循环）**。

```lua
function on_tick(next_cell)   -- next_cell = {gx=, gy=} 或 nil（main_loop 传入）
    local st = self.get_state()          -- 读共享状态（新增 caps，R4）
    local x, y = self.get_position()
    local _, _, yaw = self.get_attitude()

    if st.state == "Idle" then
        if next_cell == nil then
            return { state = "Idle", sub_target = nil, action = "stop" }
        end
        local delta = angle_to_target(next_cell, x, y, yaw)
        if math.abs(delta) > TURN_ALIGN_RAD then
            return { state = "Turning", sub_target = next_cell,
                     action = (delta > 0) and "turn_right" or "turn_left", arg = TURN_SPEED }
        else
            return { state = "Moving", sub_target = next_cell, action = "move_forward", arg = MOVE_SPEED }
        end

    elseif st.state == "Turning" then
        if next_cell == nil then return { state = "Idle", sub_target = nil, action = "stop" } end
        if 目标变了(next_cell ~= st.sub_target) then
            return { state = "Idle", sub_target = next_cell, action = "none" }  -- 回 Idle 重新决策
        end
        local delta = angle_to_target(st.sub_target, x, y, yaw)
        if math.abs(delta) <= TURN_ALIGN_RAD then
            return { state = "Idle", sub_target = st.sub_target, action = "stop" }
        end
        return { state = "Turning", sub_target = st.sub_target, action = "none" }  -- 未对齐，保持

    elseif st.state == "Moving" then
        if next_cell == nil then return { state = "Idle", sub_target = nil, action = "stop" } end
        local tx, ty = cell_center(st.sub_target)
        local dist = sqrt((x-tx)^2 + (y-ty)^2)
        if dist < SUB_TARGET_THRESHOLD_M then          -- 保留 0.2m 距离门
            local nd = angle_to_target(next_cell, x, y, yaw)
            if math.abs(nd) < STRAIGHT_ALIGN_RAD then  -- 直行连续化 10°
                return { state = "Moving", sub_target = next_cell, action = "move_forward", arg = MOVE_SPEED }
            else
                return { state = "Idle", sub_target = next_cell, action = "stop" }  -- 停车，下 tick 转向
            end
        end
        return { state = "Moving", sub_target = st.sub_target, action = "none" }  -- 未到 0.2m，保持
    end
end
```

> 注意：Lua 脚本内**不得保留 `local state / local sub_target` upvalue**（R14），状态全部读 `self.get_state()`。

### 3.4 执行流（`robot.rs`）

**main_loop（auto_tick，每 50ms）**：

```rust
let emergency = check_emergency_stop(&ls, &rs, &*stm32, &world).await;

if emergency {
    invalidate(&execute_state, &stm32, &mut generation, &mut result_rx).await;  // R2
} else {
    // R3-B：main_loop 自己调 get_path（任务副作用在此，与 reset/invalidate 同源）
    let next_cell = goal_service.lock().await.get_path().await;   // Option<(i32,i32)>

    decision_tx.try_send((generation, next_cell));                // R1：带 generation 请求决策

    match result_rx.try_recv() {
        Ok((gen, result)) if gen == generation => {               // R1：代际校验，过期丢弃
            *execute_state.write().await = ExecuteState {
                state: result.state, sub_target: result.sub_target,
            };
            if let Some(action) = result.action {                 // R7：None 保持，Some 才发
                apply_action(&stm32, action);
            }
            last_result_at = Instant::now();
        }
        Ok(_) => { /* 过期结果，丢弃 */ }
        Err(TryRecvError::Empty) => { /* 决策线程还在算，保持 */ }
        Err(TryRecvError::Disconnected) => { /* 决策线程死亡，走看门狗 */ }
    }

    // R5：看门狗——决策线程卡死/死亡兜底
    if last_result_at.elapsed() > Duration::from_millis(500) {
        let _ = stm32.stop();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        error!("[Robot] 决策线程超时未产出，已强制停车");
    }
}
```

**invalidate 助手（R2）**——急停 / `AutoCmd::Set` / `SwitchToManual` / `SwitchToAuto` 四处统一调用：

```rust
async fn invalidate(
    execute_state: &Arc<RwLock<ExecuteState>>,
    stm32: &STM32Device,
    generation: &mut u64,
    result_rx: &mut mpsc::Receiver<(u64, DecisionResult)>,
) {
    let _ = stm32.stop();                                        // 停车
    *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
    *generation += 1;                                            // 旧代际结果全部过期
    while result_rx.try_recv().is_ok() {}                        // drain 陈旧结果
    last_result_at = Instant::now();                             // 🟠 重置看门狗，避免持续急停误触发
}

// 注意：invalidate 只统一「停车 + 清意图 + 换代 + drain」；mission/goal 清理按调用点保留：
//   SwitchToManual = invalidate + mission_queue.clear() + goal_service.reset()
//   SwitchToAuto   = invalidate + goal_service.reset()
//   AutoCmd::Set   = invalidate + mission_queue.replace(list) + goal_service.reset()
//   急停           = 仅 invalidate（保留 goal 供绕行，勿清 goal）
```

**决策线程（独立线程，持有 LuaContext，非 Send）**：

```rust
// tick 载荷 = (generation, next_cell)；纯计算，回传 (gen, result)
loop {
    tokio::select! {
        r = tick_rx.recv() => {
            let (gen, next_cell) = match r { Some(v) => v, None => break };
            // 🔴 元组 (i32,i32) 不实现 mlua IntoLua，需转成命名键表
            let arg: Option<mlua::Table> = match next_cell {
                Some((gx, gy)) => { let t = lua.create_table()?; t.set("gx", gx)?; t.set("gy", gy)?; Some(t) }
                None => None,
            };
            let result: DecisionResult = match on_tick.call_async::<DecisionResult>(arg).await {
                Ok(v) => v,
                Err(e) => {                                       // R9：脚本错误回安全结果，不杀线程
                    warn!("[Robot] on_tick 失败: {e}");
                    DecisionResult { state: DecisionState::Idle, sub_target: None, action: Some(MotionAction::Stop) }
                }
            };
            if result_tx.send((gen, result)).await.is_err() { break; }   // R11：通道关闭则退出
        }
        _ = cancel.cancelled() => break,
    }
}
```

### 3.5 关键机制汇总

1. **generation 打标（R1）**：tick 载荷 = `(generation, next_cell)`；决策结果回传 `(generation, DecisionResult)`；main_loop 应用时校验 `gen == generation`。generation 是 Rust 侧记账，不进 `DecisionResult`（Lua 不可见）。
2. **invalidate 助手（R2）**：`stop` + 写 Idle/None + `generation += 1` + drain，急停 / 任务替换 / 切手动 / 切自动四处统一调用。
3. **get_path 移入 main_loop（R3-B）**：任务副作用（pop/set goal/set_goal/clear_goal）与 reset/invalidate 同源，main_loop 单线程串行，消除第二个写入流；顺带消解「决策线程 get_path 卡锁拖慢急停」。
4. **看门狗（R5）**：`last_result_at` 超时（500ms）→ stop + Idle + `error!`；`TryRecvError::Disconnected` 视为决策线程死亡。
5. **命令去重（R7）**：由 Lua 返回 `action = none` 表达「保持、不发命令」，main_loop 只对 `Some(action)` 发命令。
6. **保留 0.2m 距离门**：Moving 分支沿用 `dist < 0.2m` 换格门、直行连续化 10°、转向 5°、先停再转 + 隔 tick。

## 四、涉及文件

| 文件 | 改动 |
|---|---|
| `Src/Robot/core/state.rs` | 新增 `DecisionState` / `MotionAction`（带载荷）/ `DecisionResult`；`ExecuteState` 加 `state`；更新单一写入者注释 |
| `Src/VM/capability_binding.rs` | `RobotCapsContext` 加 `execute_state`；**新增** `self.get_state`（R4）；**删** `action.*` 五个；`get_path` caps 删除（改 main_loop 调用）；保留 `get_position`/`get_attitude` |
| `Src/Robot/core/goal.rs` | 删 `sub_target` 字段 + `sub_target()` + `clear_sub_target()`；`get_path` 只返回格 |
| `Src/Robot/core/robot.rs` | main_loop：`get_path` + `invalidate` 助手 + generation 打标/校验 + `apply_action` + 看门狗；决策线程：tick 载荷 `(u64, Option<cell>)` + 回传 `(u64, DecisionResult)`；删「同步 goal_service.sub_target」旧逻辑 |
| `programs/robot/car.lua` | 纯函数化：`on_tick(next_cell)` 返回结果表；删 upvalue、删 `get_path`/`action.*` 调用 |

## 五、详细实施步骤

### 步骤 1：`state.rs` — 数据结构
新增 `DecisionState` / `MotionAction`（带载荷枚举）/ `DecisionResult`；`ExecuteState` 加 `state`；更新注释（写者 = main_loop，读者 = state_notifier + Lua get_state）。

### 步骤 2：`capability_binding.rs` — caps 改造
`RobotCapsContext` 加 `execute_state: Arc<RwLock<ExecuteState>>`；新增 `self.get_state`（`create_async_function`，返回 `{state=..., sub_target=...}`，字符串↔枚举映射放独立 `fn`）；删 `action.*` 五个与 `get_path`。

### 步骤 3：`goal.rs` — 删 sub_target 维护
删字段 + `sub_target()` + `clear_sub_target()` + `get_path`/`start_goal` 内 `self.sub_target` 读写；保留 `self.world.clear_goal()` 副作用。

### 步骤 4：`robot.rs` — 单点执行
1. main_loop：调 `get_path` 拿 `next_cell`；`decision_tx` 载荷改 `(u64, Option<(i32,i32)>)`；`result_rx` 收 `(u64, DecisionResult)` + 代际校验；`apply_action`（match MotionAction）；`invalidate` 助手；看门狗（`last_result_at` 500ms）。
2. 决策线程：`on_tick.call_async::<DecisionResult>(next_cell)` + 错误回安全结果 + `select!` 带 cancel。
3. 四处重置/急停统一调 `invalidate`；删「同步 goal_service.sub_target」逻辑。

### 步骤 5：`car.lua` — 纯函数化
按 §3.3 重写；删 upvalue；`on_tick(next_cell)` 返回结果表；保留 0.2m 门、命令去重（`action="none"`）、先停再转 + 隔 tick、直行连续化 10°、转向 5°。

### 步骤 6：`DecisionResult::from_lua` 手写（R10）
mlua 未启用 `macros`/`serialize`，需手写 `impl FromLua for DecisionResult`：读 `Table` 的 `state`(String→枚举)、`sub_target`(Option<Table>→gx/gy)、`action`(String→MotionAction + arg)；字符串→枚举映射放独立 `fn`（可单测）。

### 步骤 7：编译 + 测试 + 实车
1. `./build.sh check` 通过。
2. 单测：`FromLua` 解析、`invalidate` 逻辑、generation 打标/校验。
3. mock 冒烟：`get_state`/`get_position`/`get_attitude` 只读 + `on_tick(next_cell)` 返回结果可解析。
4. 实车：三卡死路径恢复 + 急停不被拱 + 决策线程死亡兜底 + 行为一致性。

## 六、验证方式

| 验证项 | 方式 |
|---|---|
| 编译 | `./build.sh check` 通过 |
| 决策结果解析 | 单测：`DecisionResult::from_lua` 解析 on_tick 返回表 |
| generation 打标/校验 | 单测：急停后旧代际结果被丢弃 |
| invalidate 统一调用 | 代码审查 + 单测：四处调用点覆盖 |
| 任务替换/模式切换恢复 | 实车：下发新任务、Auto↔Manual 切换后正常 |
| 急停不被拱 | 实车：障碍出现时车停稳，无 50ms 脉冲前拱 |
| 决策线程死亡/卡死兜底 | 模拟：脚本失败/线程卡死 → 车停 + 意图 `valid=false` |
| 行为一致性 | 走格子 / 转向 / 直行 / 到达 与稳定分支一致 |

## 七、风险与注意

1. **改动面大**：5 个文件、Lua 接口从「有副作用」改为「纯函数 + 返回结果」，结构性重构，小步提交。
2. **P3（D* 无迭代上限）升级为实施前提**：get_path 移入 main_loop 后，D* `next_step` 在 main_loop 同步执行，极端地图会拖慢命令/急停响应。实施时给 `compute_shortest_path` 加 MAX_ITERS/时间预算（见 `robot_review_problem.md` P3）。
2. **决策延迟**：决策线程算完回传 main_loop 应用，可能延迟一个 tick（50ms），车控可接受，需实车确认。
3. **`apply_action` 与急停 stop 的关系**：急停直发的 `stm32.stop()` 不进入命令去重轨迹，避免「急停后首个决策被误判相同而漏发」。
4. **Manual 模式例外（R12）**：`dispatch` 手动命令由 main_loop 直发（Auto 决策流停用），是单写者框架内的唯一例外，需文档注明。
5. **退出清理（R13）**：main_loop 对 `TryRecvError::Disconnected` 按「线程已死」处理；退出顺序 stop → LiDAR → stm32.shutdown 不变。
6. **result 通道（R11）**：`mpsc::channel::<(u64, DecisionResult)>(1)`，容量 1 形成背压，决策线程 `send` 与 `cancel` 用 `select!` 并列。
7. **文档同步**：`universal_robot_design.md`、`wb_22`、`Architecture/robot_arch.md:85`、`docs/design_doc/orion_protocol.md:147` 同步改口（「Lua 纯决策 + Rust 单点执行」）。
8. **每次改动小步提交**：每个步骤单独 commit，便于回滚。

## 八、附：架构定位（参考 ROS2）

- **Lua = 决策节点**（类比 ROS2 Python planner）：纯产出「意图」（DecisionResult），零副作用，车/机各写一个。
- **Rust main_loop = 控制节点**（类比 ROS2 C++ 底盘驱动）：唯一执行者，消费意图 → 写状态 + 发命令；generation 过期丢弃 ≈ ROS2 cmd_vel 超时；看门狗 ≈ 决策节点掉线即停车。

## 九、实施记录（2026-08-21）

**已完成**：
- 5 个文件落地：`state.rs`（`DecisionState`/`MotionAction`/`DecisionResult` + `ExecuteState.state`）、`goal.rs`（删 `sub_target` 维护）、`capability_binding.rs`（`get_state` + 手写 `FromLua` + 删 `action.*`/`get_path`）、`robot.rs`（main_loop 单点执行 + `invalidate` + generation 打标/校验 + 看门狗 + 决策线程回传 `DecisionResult`）、`car.lua`（纯函数 `on_tick(next_cell)`）。
- **W1 修复**：看门狗分支改调 `invalidate`（换代 + drain），根治「决策线程慢而未死 → 旧 gen 结果重动车」。
- **W2 修复**：`compute_shortest_path` 加 `MAX_COMPUTE_ITERS = 10000`，`iter` 统计 pop 次数，超限降级（不 panic、不返回障碍格）。
- 编译 `./build.sh check` 通过（25 warning 历史遗留）；robot 单测 122 passed / 0 failed。

**遗留**（见 `Task/robot_review_problem.md` 第十节 T23-1~T23-10）：
- 🟠 关键新路径无单测（FromLua / invalidate / generation 过期丢弃 / 迭代上限降级）。
- 🟡 若干：tick 通道 drain、日志刷屏、arg 吞错、文件头日期、文档漂移等，可后补。
