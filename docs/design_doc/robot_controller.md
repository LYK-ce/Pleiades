# Robot Controller 设计文档

> 创建日期：2026-07-28
> 状态：设计中

---

## 1. 概述

Robot Controller 是 Orion 机器人的自主控制层，将当前"遥控器直通"的命令体系升级为模式分离（Manual/Auto）+ 任务执行架构。

### 1.1 当前问题

主循环是对所有命令无差别转发到硬件，没有内部状态、没有自主决策：

```
cmd_rx.recv() → dispatch → stm32.forward/spin/stop
```

### 1.2 目标

```
cmd_rx.recv() → Command Router
  Manual 模式: 低层指令直发硬件（和现在一致）
  Auto 模式:   高层任务入队 → Executor 自主执行
```

---

## 2. 架构

```
┌──────────────────────────────────────────────┐
│              Robot Controller                 │
│                                              │
│  ┌──────────────────────────────────────┐   │
│  │ Command Router                       │   │
│  │  Mode(m) → op_mode                   │   │
│  │  Manual(m) → (Manual) stm32.xxx()    │   │
│  │  Auto(a)  → (Auto) MissionQueue      │   │
│  └──────────────────────────────────────┘   │
│                                              │
│  ┌──────────┐   ┌───────────────────┐       │
│  │ OpMode   │   │ MissionQueue      │       │
│  │ Manual   │   │ [goto(A), goto(B)]│       │
│  │ Auto     │   └────────┬──────────┘       │
│  │ Paused   │            │ pop              │
│  └──────────┘   ┌────────┴──────────┐       │
│                 │    Executor        │       │
│                 │ ┌────────────────┐ │       │
│                 │ │ Step:          │ │       │
│                 │ │ ① 感知         │ │       │
│                 │ │ ② 决策         │ │       │
│                 │ │ ③ 执行         │ │       │
│                 │ └────────────────┘ │       │
│                 └────────┬──────────┘       │
│                          │                  │
│  ┌───────┐ ┌──────┐ ┌───┴────┐ ┌───────┐  │
│  │ odom  │ │LiDAR │ │ D* Lite│ │ Grid  │  │
│  └───────┘ └──────┘ └────────┘ └───────┘  │
└──────────────────────────────────────────────┘
```

---

## 3. Command 系统

### 3.1 三层分类

```rust
pub enum Command {
    Mode(ModeCmd),      // 模式控制（不受当前模式限制）
    Manual(ManualCmd),  // 手动模式下有效
    Auto(AutoCmd),      // 自动模式下有效
}

pub enum ModeCmd {
    SwitchToManual,
    SwitchToAuto,
    Pause,
    Resume,
}

pub enum ManualCmd {
    Forward(i16),  Backward(i16),
    SpinLeft(i16), SpinRight(i16),
    Stop, Beep(u16),
    StartLidarScan, StopLidarScan,
}

pub enum AutoCmd {
    Push(Vec<Mission>),
    Cancel,
}

pub enum Mission {
    Goto(f32, f32),
    // 未来: Patrol(Vec<(f32, f32)>, bool), Explore, ...
}
```

### 3.2 模式切换

无论从哪个模式切到哪个模式，必须先 `stm32.stop()`：

```
SwitchToManual → stop() → op_mode = Manual
SwitchToAuto   → stop() → op_mode = Auto
```

Auto → Manual：MissionQueue 保留不动（暂停），切回 Auto 后继续。
Cancel（仅 Auto 下）：清空队列 + 停车。

### 3.3 WebSocket 协议

```json
// 模式
{"cmd":"mode",  "action":"switch_to_auto"}
{"cmd":"mode",  "action":"switch_to_manual"}

// 手动
{"cmd":"manual","action":"forward","speed":50}
{"cmd":"manual","action":"stop"}

// 自动
{"cmd":"auto",  "action":"push",
 "missions":[{"type":"goto","x":1.0,"y":2.0}]}
{"cmd":"auto",  "action":"cancel"}
```

---

## 4. 主循环

```rust
let mut auto_interval: Option<Interval> = None;

loop {
    select! {
        cmd = cmd_rx.recv() => {
            match cmd {
                Command::Mode(m)    => handle_mode(m),
                Command::Manual(m)  => if is_manual { handle_manual(m) },
                Command::Auto(a)    => if is_auto { handle_auto(a) },
            }
        }
        _ = auto_tick.tick() => {
            // 仅 Auto 模式触发
            executor.step(&mission_queue, &odom, &lidar, &dstar, &stm32);
        }
        _ = cancel.cancelled() => break,
    }
}

fn handle_mode(m: ModeCmd) {
    stm32.stop();
    op_mode = match m {
        SwitchToManual => (Manual, auto_interval = None),
        SwitchToAuto   => (Auto,   auto_interval = Some(interval(tick_ms))),
        ...
    };
}
```

- 被动部分：`cmd_rx.recv()` 处理外部命令（一直活跃）
- 主动部分：`auto_tick` 驱动 Executor（仅 Auto 激活）
- 一条 select! 同时处理，互不阻塞

---

## 5. Executor

### 5.1 双层 Target

```rust
struct Executor {
    goal: (f32, f32),         // goto 任务最终目标（不变）
    sub_target: (i32, i32),   // D* 给的当前要走的下一格
    state: ExecState,
}

enum ExecState { Idle, Turning, Moving }
```

- `goal`：goto 命令参数，全程不变
- `sub_target`：D* 输出紧邻位置的单格，走到了就换下一个

### 5.2 三状态

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
| Idle | 停下来、思考：pop 任务 / 问 D* / 算角度 | 决定 Turning 或 Moving |
| Turning | `spin` 对准目标角度 | 角偏差 < 5° → stop → Idle |
| Moving | `forward` 向 sub_target 直行 | sub_target 到 / 障碍 → stop → Idle |

### 5.3 感知 → 决策 → 执行

每个 tick（100ms）执行一次 `step()`：

```
① 感知
  odom → 世界坐标 → 网格坐标
  current_yaw

② 决策（Idle 状态下）

  何时问 D*？两种触发：

  a) sub_target 走到了
     distance(odom, sub_target 中心) < 0.2m
     → 问 D*: "我在 (gx,gy)，下一格？"
     → sub_target = D* 的返回值

  b) 前方障碍
     LiDAR 距离 < obstacle_threshold
     → 通知 D*: "这格 Occupied"
     → D* 局部修补
     → 问 D*: "新方向？"
     → sub_target 更新

  翻译:
     sub_target(gx, gy) → 世界中心(wx, wy)
     target_yaw = atan2(wy - odom_y, wx - odom_x)
     角偏差 = target_yaw - current_yaw
     |角偏差| > 5° → need_turn

③ 执行
     need_turn → spin_left/spin_right → Turning
    !need_turn → forward(speed) → Moving
```

**sub_target 切换条件**：不是"网格坐标变了"，而是"距离 sub_target 中心 < 阈值"。避免跨格边缘时错误转向。

### 5.4 闭环控制

**Turning 闭环**：

```
tick 0: Idle 决策"需要转" → spin_left(30) → Turning
tick 1: Turning 读 yaw, 还没到 → 不操作
tick 2: 读 yaw, 对齐了  → stop → Idle
```

**Moving 闭环**：

```
tick 0: Idle 决策"直走" → forward(30) → Moving
tick 1: Moving 读 odom, 还在路上 → 不操作
tick 2: sub_target 到了? → stop → Idle（问 D* 拿下一个）
        LiDAR 有障碍?   → stop → Idle（告诉 D* 这格不能走）
```

命令只发一次，后续 tick 只监测，不重复发。

---

## 6. D* Lite 规划器

### 6.1 接口

```rust
impl DStarLite {
    fn set_goal(gx: i32, gy: i32);
    fn next_step(current_gx: i32, current_gy: i32) -> Option<(i32, i32)>;
    fn update_cell(gx: i32, gy: i32, state: u8);
}
```

### 6.2 特点

- 输出**单格方向**，不输出路点列表
- 内部维护距离场，障碍时局部修补
---

## 7. 传感器与坐标系约定

### 8.1 STM32 yaw

| 项目 | 实际行为（2026-08-03 实测） |
|------|---------------------------|
| 初始值 | **开机当前朝向 = 0°**（非磁北） |
| 正方向 | **顺时针为正**（右转 → yaw 增大） |
| 数据来源 | STM32 固件 AHRS，约 10Hz 主动上报 `RPT_IMU_ATT` (0x0C) |
| 解析 | `i16 ÷ 10000 → 弧度` |

> ⚠️ 此前假设"yaw=0=北"是错误的。每次启动小车朝向不同，yaw 从当前朝向的 0° 开始。

### 8.2 spin_left / spin_right

| 命令 | 物理动作 | yaw 变化 | 代码调用 |
|------|---------|---------|---------|
| `spin_left` | 左转（逆时针） | yaw **减小** | delta < 0 时 |
| `spin_right` | 右转（顺时针） | yaw **增大** | delta > 0 时 |

经过实车测试确认 spin_left/spin_right 方向后已在 `890e2fc` 对调完成。

### 8.3 里程计

odom 累积使用数学约定（`(cos, sin)`），Pictor 可视化一致。坐标转换代码未修改。

### 8.4 LiDAR 投影

`laser_to_world` 使用数学约定（`(cos, sin)`），与 odom 一致。Pictor 地图显示正确。

---

---

## 8. 障碍检测

双层保障：

| 层 | 机制 | 延迟 | 作用 |
|:---:|------|:---:|------|
| 实时 | LiDAR 前方距离 < 0.3m → stop | ~100ms | 急停，碰撞预防 |
| 规划 | OccupancyGrid → D* 局部修补 | ~0.4s | 路径重规划 |

实时层不等建图，直接读数。

---

## 9. 配置

```toml
[Robot]
stm32_port = "/dev/myserial"
stm32_baud = 115200
lidar_port = "/dev/rplidar"
lidar_baud = 230400
ws_bind = "0.0.0.0:9090"

auto_tick_ms = 50
arrival_threshold_m = 0.3
sub_target_threshold_m = 0.2
obstacle_threshold_m = 0.3

turn_speed = 10
move_speed = 30
turn_align_threshold_deg = 5

unknown_as_free = true
```

---

## 10. 文件结构



```
Src/Robot/
├── core/
│   ├── command.rs      ← 三层 Command
│   ├── mode.rs         ← OpMode
│   ├── mission.rs      ← MissionQueue
│   ├── robot.rs        ← 主循环 + auto_tick
│   └── executor.rs     ← Executor + 三状态 Idle/Turning/Moving
├── slam/
│   └── pathfinder.rs   ← D* Lite
└── ...

Src/Config/
    └── config.rs       ← RobotConfig
```

## 11. 已知限制

- 不拆分 Global/Local Planner（后续可升级）
- 仅单 Chunk 内路径规划
- 仅处理 Goto 任务类型

