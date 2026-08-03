# Task 7: Robot State Machine

> 状态：设计中
> 创建日期：2026-07-27
> 最后更新：2026-07-28

## 目标

为 Robot 提供高层任务 API（goto 等），实现模式分离（Manual/Auto），替代当前纯遥控器级别的命令体系。

---

## ✅ 已决策

### Command 三层分类

```rust
pub enum Command {
    Mode(ModeCmd),      // 模式控制
    Manual(ManualCmd),  // 手动模式（仅 Manual 下有效）
    Auto(AutoCmd),      // 自动模式（仅 Auto 下有效）
}

pub enum ModeCmd { SwitchToManual, SwitchToAuto }

pub enum ManualCmd { Forward(i16), Backward(i16), SpinLeft(i16),
    SpinRight(i16), Stop, Beep(u16), StartLidarScan, StopLidarScan }

pub enum AutoCmd { Push(Vec<Mission>), Cancel }
pub enum Mission { Goto(f32, f32) /* 未来: Patrol, Explore... */ }
```

### WebSocket JSON

```json
{"cmd":"mode",  "action":"switch_to_auto"}
{"cmd":"manual","action":"forward","speed":50}
{"cmd":"auto",  "action":"push","missions":[{"type":"goto","x":1,"y":2}]}
```

### 底层同步化 — STM32Device 命令去 async

当前 `forward()` / `stop()` 等方法标注 `async`，但实际只是往 `mpsc` 通道 `send().await`——等价于队列 push，不涉及任何串口 I/O。串口写入在独立的 TX task 中异步完成。

**决策**：STM32Device 所有运动控制方法改为同步（`try_send`），`dispatch` 也改为同步：

```rust
// 之前：假 async
pub async fn forward(&self, speed: i16) -> Result<(), String> {
    self.cmd_tx.send(bytes).await  // 只等通道空位，不等串口
}

// 之后：同步
pub fn forward(&self, speed: i16) -> Result<(), String> {
    self.cmd_tx.try_send(bytes).map_err(|e| format!("TX 通道满: {e}"))
}
```

```rust
// dispatch 也去 async
fn dispatch(stm32: &STM32Device, lidar: &Option<LidarDevice>, cmd: Command) {
    match cmd {
        Command::Manual(ManualCmd::Forward(s)) => { let _ = stm32.forward(s); }
        // ...
    }
}
```

**收益**：select! 中 dispatch 不阻塞。executor 发命令也走同一路径，主循环保持纯路由。

**注意**：`try_send` 在通道满时失败。容量 32 足够（TX task 写入串口 < 1ms），满意味着串口物理故障——此时应打 error 日志，上层做退避。

### 主循环（同步 dispatch + auto_tick）

```rust
loop {
    select! {
        cmd = cmd_rx.recv() => {
            match cmd {
                Some(Command::Mode(m)) => {
                    stm32.stop();             // 同步，不阻塞
                    *op_mode.write() = ...;
                }
                Some(Command::Manual(m)) if *op_mode.read() == Manual => {
                    dispatch(&stm32, &lidar, m);  // 同步
                }
                Some(Command::Auto(a)) if *op_mode.read() == Auto => {
                    match a {
                        Push(list) => mission_queue.push(list),
                        Cancel     => { stm32.stop(); mission_queue.clear(); }
                    }
                }
                _ => warn!("模式不匹配或通道关闭"),
            }
        }

        // auto_tick：仅在 Auto 模式下激活
        _ = auto_timer.tick(), if *op_mode.read() == Auto => {
            executor.step(&stm32, &robot_state, &lidar_state, &grid, &mission_queue);
        }

        _ = cancel.cancelled() => break;
    }
}
```

**auto_timer 动态生灭**：用 `tokio::time::sleep_until(next_tick)` 替代 `interval`——切换 Auto 时设 `next_tick = now + 100ms`，切 Manual 时通过 `op_mode` 条件跳过。

**executor.step() 通过 cmd_tx 发命令**（不直接调 STM32，保持主循环纯路由）：

```rust
impl Executor {
    fn step(&mut self, cmd_tx: &Sender<Command>, ...) {
        match self.state {
            Idle => { /* 感知 → 决策 → cmd_tx.send(Manual(Forward/Stop/Spin)) */ }
            Turning => { /* 读 yaw → 对齐了 → cmd_tx.send(Manual(Stop)) */ }
            Moving => { /* 读 odom/LiDAR → 到了/障碍 → cmd_tx.send(Manual(Stop)) */ }
        }
    }
}
```

切换模式必须先 `stop()`。Auto → Manual 清空队列并停车，Manual → Auto 从空队列开始。

### 已决策补充

| 决策 | 说明 |
|------|------|
| yaw 角度归一化 | `角偏差 = (target - current + π).rem_euclid(2π) - π`，限制到 [-π, π] |
| 实时障碍急停 | executor.step() 内部第一步读 LiDAR 前方距离 < 0.3m → `cmd_tx.send(Manual(Stop))` + 通知 D* |
| Idle 重入保护 | executor 内部 `stepping: bool` 标志位，防止同一 tick 重入 D* 查询 |
| Turning→Moving 过渡 | Turning 对齐后发 stop → 下个 tick 确认 |角速度| < 阈值 → 再发 forward。避免残余角速度导致走弧线 |
| 不合法命令 | 模式不匹配时静默丢弃，打 warn 日志，不回复 WS 客户端 |
| D* 失败 | 打 error 日志，sub_target 不更新，executor 停在 Idle。用户手动切 Manual 处理 |
| 角度过冲 | 单一阈值 `turn_align_threshold_deg = 5`。进阈值即停，过冲由下个 tick 自然修正 |

### Robot 结构

```rust
pub struct Robot {
    pub cmd_tx: Sender<Command>,
    pub robot_state: Arc<RwLock<RobotState>>,
    pub lidar_state: Arc<RwLock<LidarState>>,
    pub grid: Arc<RwLock<OccupancyGrid>>,
    pub mission_queue: Arc<RwLock<MissionQueue>>,
    pub op_mode: Arc<RwLock<OpMode>>,
    pub pose_tx, pub map_tx, cancel ...
}
```

### MissionQueue

```
Robot 持有 → 主循环驱动 Executor 消费

入队: Auto(Push(list)) → mission_queue.extend()
取任务: executor.step() → pop_next()
清空: Auto(Cancel) → mission_queue.clear() + 停车

生命周期:
  ① 入队 → ② pop → ③ 执行中(executor) → ④ Arrived/Failed → ② pop 下一个
  队列空 → executor 等待
```

### Executor 三状态

```
Idle     → 停下来、思考：pop 任务 / 问 D* / 算角度
Turning  → 旋转中对准目标角度
Moving   → 直行中
```

状态转移：

```
           pop mission / 问 D*
Idle ─────────────────────────────→ Turning
  ↑                                      │
  │ pop 下一个                        角度对齐
  │                                      ↓
Arrived ←── Idle ←──(stop)───── Moving
              │                      ↑
              │ 方向不变 / 无障碍      │
              └──────────────────────┘
```

| 状态 | 做什么 | 退出条件 |
|------|------|------|
| Idle | 停下思考：pop 任务 / 问 D* / 算角度 | 决定 Turning 或 Moving |
| Turning | spin 对准目标角度 | 角偏差 < 5° → stop → Idle |
| Moving | forward 向 sub_target 直行 | sub_target 到 / 障碍 → stop → Idle |

### 双层 target

```rust
struct Executor {
    goal: (f32, f32),           // goto 任务最终目标（不变）
    sub_target: (i32, i32),     // D* 给的当前要走的下一格
}
```

- `goal` = goto 命令参数，全局不变
- `sub_target` = 紧邻当前位置的单格，走到了就问 D* 拿下一个
- `sub_target` 只在**两种时机**更新，不是每 tick 都问 D*

### 执行流程：感知 → 决策 → 执行

auto_tick 每 100ms 触发一次 `executor.step()`：

```
step():
  ① 感知: odom → 世界坐标 → 网格坐标
          current_yaw

  ② 决策 (Idle 状态下):
     什么时候问 D*？不是每 tick —— 仅以下时机:

       a) sub_target 走到了: distance(odom, sub_target中心) < 0.2m
          → 问 D*: "我在 (gx,gy)，下一格？"
          → sub_target = D* 返回的下一格

       b) 前方障碍: LiDAR 距离 < obstacle_threshold
          → 通知 D*: "这格 Occupied"
          → D* 局部修补
          → 问 D*: "新方向？"
          → sub_target 更新

     然后翻译:
       sub_target (gx, gy) → 世界坐标 (wx, wy)
       target_angle = atan2(wy - odom_y, wx - odom_x)
       角偏差 = target_angle - current_yaw

       if |角偏差| > 阈值 → need_turn = true
       else              → need_turn = false

  ③ 执行:
       need_turn → spin_left/spin_right → Turning
       !need_turn → forward(30) → Moving
```

### 闭环控制

**Turning**: spin 发一次 → 每 tick 读 yaw → 对齐了 stop → Idle
**Moving**: forward 发一次 → 每 tick 读 odom/LiDAR → sub_target 到/障碍 stop → Idle

### D* Lite 规划器

- 输出**下一格方向**（不输出路点列表）
- 查询时机：仅 sub_target 到达或障碍触发
- 内部维护梯度场，障碍时局部修补
- 同时承担 Global + Local Planner（现阶段不拆分）
- Unknown 格子视为 Free（乐观）

### 障碍检测

| 层 | 机制 | 延迟 | 作用 |
|:---:|------|:---:|------|
| 实时 | LiDAR 前方距离 < 0.3m | ~100ms | 急停，防撞 |
| 规划 | OccupancyGrid → D* 修补 | ~0.4s | 重规划 |

### Robot 配置化

```toml
[Robot]
stm32_port = "/dev/myserial"
stm32_baud = 115200
lidar_port = "/dev/rplidar"
lidar_baud = 230400
ws_bind = "0.0.0.0:9090"
auto_tick_ms = 100
arrival_threshold_m = 0.3
sub_target_threshold_m = 0.2
obstacle_threshold_m = 0.3
turn_speed = 30
move_speed = 30
turn_align_threshold_deg = 5
unknown_as_free = true
```

---

## ✅ 已决策（全部完成）

> 所有设计问题已决断：Idle 重入保护、auto_timer 动态生灭、底层同步化、yaw 归一化、实时急停、Turning→Moving 过渡、不合法命令、D* 失败、角度过冲。详见上方各章节。

## 文件计划

```
Src/Robot/
├── core/
│   ├── command.rs      ← [修改] 三层 Command
│   ├── mode.rs         ← [新增] OpMode
│   ├── mission.rs      ← [新增] MissionQueue
│   ├── robot.rs        ← [修改] 主循环 + auto_tick
│   └── executor.rs     ← [新增] Executor + Idle/Turning/Moving
├── control/device/stm32/
│   └── mod.rs          ← [修改] 运动控制方法去 async (send→try_send)
├── slam/
│   └── pathfinder.rs   ← [新增] D* Lite
└── ...

Src/Config/
    └── config.rs       ← [修改] RobotConfig
```

## 已知限制

- 不拆分 Global/Local Planner
- 仅单 Chunk 内路径规划
- 仅处理 Goto（Patrol/Explore 后续）

## 人类评审

<!-- 在此区域写下评审意见 -->

