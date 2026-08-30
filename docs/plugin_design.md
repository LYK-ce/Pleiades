# 插件化方案设计文档

Presented by KeJi
Date ： 2026-08-29

> 本文档沉淀对「异构设备插件化」方案的讨论，覆盖从「Lua 行为层」到「executor 用 .so 替代 Lua」的完整推演过程，作为后续架构演进的设计依据。

---

## 1. 背景与目标

### 1.1 诉求

系统为**异构设备**开发（当前是车 + 无人机，未来可能更多），但目前是**单一二进制**，设备装配在 `Robot::launch` 中硬编码。长期需要做到：

- 增加/更换设备**不重编主程序**；
- 异构设备的**行为差异**可扩展、可维护；
- 技术栈尽量收敛，避免维护多套扩展机制。

### 1.2 概念澄清

- **单一二进制 ≠ 单进程**。本方案统一在**单进程**前提下讨论：设备/决策作为 `.so` 动态库加载进主进程，共享内存空间，回调即函数指针调用，零拷贝。
- 单进程插件模型的参照：Nginx 模块、PostgreSQL 扩展、VLC 插件。

---

## 2. 问题分析

### 2.1 现状：单一二进制 + 硬编码装配

**`MotionDevice` 只统一了「最小语义交集」**（`Src/Robot/device.rs`）：

```rust
pub trait MotionDevice: Send + Sync {
    fn move_forward(&self) -> Result<(), String>;  // 前进 = 车头/机头
    fn move_backward(&self) -> Result<(), String>;
    fn turn_left(&self) -> Result<(), String>;     // 车差速转弯 vs 机航向调整
    fn turn_right(&self) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;          // 停车 vs 悬停
    fn shutdown(&self);
}
```

设备的差异**恰好在交集之外**：

| 设备 | 独有动作/状态 |
|---|---|
| 车（STM32） | `Beep`、`encoders[4]`（编码器）、差速 `SpinLeft/SpinRight` |
| 机（MAVLink） | `Takeoff/Land`、高度/姿态（`Telemetry`、`attitude`）、GUIDED 模式 |

`ManualCmd` 枚举里已混入 `Takeoff/Land/Beep` 等**超集指令**，说明动作层本身就不完全同构，只是 trait 把差异抹平了。

**`Robot::launch` 是 18 参数的「上帝函数」，且二选一**（`Src/Robot/core/robot.rs` L179-186）：

```rust
// 4.1 确定运动设备（车或机，二选一）
let motion: Arc<dyn MotionDevice> = if let Some(s) = &stm32 {
    s.clone() as Arc<dyn MotionDevice>
} else if let Some(m) = &mavlink {
    m.clone() as Arc<dyn MotionDevice>
} else {
    return Err("chassis 与 flight_ctrl 至少启用一个".to_string());
};
```

加第三类设备（船/四足）就得改这个 `if-else` + 加参数 + 重编主程序。

### 2.2 核心矛盾

设备驱动的接口「窄而深」（几个动作 + 一份配置，几乎不读主程序共享状态），适合插件化；而**决策层的接口「宽而复杂」**（要读写机器人状态、地图、集群表、任务队列），插件化代价高——这是全文最需要辨清的不对称。

---

## 3. 候选方案总览

| 方案 | 载体 | 热更新 | 主程序重编 | ABI 风险 | 适用 |
|---|---|---|---|---|---|
| A. Lua 行为/决策层 | `.lua` 脚本 | ✅ 秒级 | ❌ 否 | 🟢 低 | 行为编排 |
| B. `.so` 设备插件 | `.so`/`.dll` | ❌ 需重编插件 | ❌ 否 | 🔴 高 | 设备驱动 |
| C. 编译期 feature + registry | 主二进制 | ❌ | ✅ 是 | 🟢 无 | 设备驱动（起步） |
| D. executor 用 `.so` 替代 Lua | `.so` | ❌ 需重编插件 | ❌ 否 | 🟢 低（纯函数） | 决策层（**最终方向**） |

---

## 4. 方案 A：Lua 行为/决策层

### 4.1 定位

把异构设备的**行为差异**（动作语义、状态机、决策策略）下沉到 Lua 脚本，每类设备一个行为包（`car_behavior.lua` / `uav_behavior.lua`）。

### 4.2 边界

- Lua **不直接碰硬件**、**不做高频 I/O**；
- Rust 提供能力原语，Lua 只编排（对应 instructions.md「闭包只做薄胶水、业务逻辑在 Rust」）。

### 4.3 结论

方向正确，但**Lua 的落点应是「行为/决策层」，不是「硬件驱动层」**。此方案后来被方案 D 部分取代（见 §8）。

---

## 5. 方案 B：`.so` 设备插件（单进程）

### 5.1 可行性

Rust 动态加载 `.so`/`.dll` 有两条成熟路线：

1. `libloading` + 手写 `extern "C"` FFI；
2. `abi_stable` crate（封装不安全细节，专为 Rust 插件设计）。

### 5.2 核心难点：Rust 无稳定 ABI

- `String`/`Vec`/`dyn Trait` 的 vtable、结构体布局**不保证跨版本稳定**；
- 不能直接跨 `.so` 边界传 `String`/`Vec`/`Result<String,_>`（未定义行为，会崩）；
- 必须用 `#[repr(C)]` + `extern "C"` 的 C ABI，或 `abi_stable` 的稳定类型（`RString`/`RVec`/`RBox`）；
- 主程序和插件**必须同工具链、同依赖版本**编译，否则布局对不上。

### 5.3 三套方案对比

| 维度 | `.so` 插件 | Lua 脚本插件 | 编译期 feature + registry |
|---|---|---|---|
| 加设备是否重编主程序 | ❌ 否 | ❌ 否 | ✅ 是 |
| 部署物形态 | 主程序 + N 个 `.so` | 主程序 + N 个 `.lua` | 单一二进制 |
| ABI/unsafe 风险 | 🔴 高（版本锁死 + unsafe） | 🟢 低 | 🟢 无 |
| 性能 | 原生 | 有解释开销 | 原生 |
| 跨平台（Jetson aarch64 / PC x86_64） | 🔴 每平台各编一套 | 🟢 脚本跨平台 | 🟢 编译期处理 |
| 现场热更新 | 需重载机制 | 天然支持 | 需重启 |

### 5.4 结论

`.so` 是**重武器**：ABI 锁死 + unsafe + 双平台交叉编译的成本要长期背。适用于「不碰主程序 + 原生性能 + 第三方交付二进制插件」的场景。

---

## 6. 设备分类抽象（运动 / 感知 / 执行器）

### 6.1 三类抽象

按**接口能力**分类，而非按物理设备分类：

| 类别 | 数据方向 | 典型设备 | 现有对应 |
|---|---|---|---|
| 🟦 运动类 Locomotion | 命令下行（主→设备） | STM32 底盘、MAVLink 飞控 | `MotionDevice` |
| 🟩 感知类 Perception | 数据上行（设备→主） | YDLIDAR、IMU、摄像头、编码器 | `LidarState`/`Telemetry` |
| 🟨 执行器类 Effector | 命令下行（离散/参数化） | 蜂鸣器、舵机、RGB、机械臂 | `ManualCmd::Beep`/`Servo`/`RGB` |

**关键洞察**：一个物理设备可同时是多类（STM32 既是运动类又是感知类）。分类按「能力」，一个 `.so` 可实现多个接口（对应 Rust 一类型多 trait）。

### 6.2 接口契约（语义层）

```rust
// 运动类：主程序主动调用
pub trait MotionDevice: Send + Sync {
    fn capabilities(&self) -> MotionCapabilities;
    fn move_forward(&self) -> Result<(), String>;
    fn turn_left(&self) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
}

// 感知类：设备主动产生数据，主程序注册回调
pub trait SensorDevice: Send + Sync {
    fn sensor_kind(&self) -> SensorKind;              // Lidar / IMU / Camera / Encoder
    fn set_data_callback(&self, cb: SensorCallback);  // 数据上行通道
    fn start_scan(&self) -> Result<(), String>;
    fn stop_scan(&self) -> Result<(), String>;
}

// 执行器类：离散/参数化动作
pub trait EffectorDevice: Send + Sync {
    fn effector_kind(&self) -> EffectorKind;          // Beep / Servo / Rgb / Arm
    fn apply(&self, action: EffectorAction) -> Result<(), String>;
}
```

生命周期统一收敛：

```rust
// 每个 .so 导出同一工厂 + 生命周期方法
pub extern "C" fn create_device(kind: DeviceKind, config: *const c_char) -> *mut DeviceHandle;
pub extern "C" fn start(handle) -> c_int;   // 配置以序列化字节传入
pub extern "C" fn stop(handle) -> c_int;    // 或 shutdown
```

> `config` 传**序列化字节**（TOML/JSON），插件自解析——跨 `.so` 边界不能直接传 Rust 结构体。这承接 `device.rs` 注释「start 不进 trait、参数是各自 config 类型」的思路，统一为「字节流 + 插件自解析」。

### 6.3 三个设计难点

1. **感知类数据上行**（最大坑）：运动/执行器是「主程序调方法」，感知类要**持续高频**推数据（雷达 10Hz+）。跨边界要么主程序传回调指针（C 风格 callback），要么插件导出拉取接口（主程序轮询）。回调性能好但要注意「回调内不阻塞」+ 插件线程生命周期。
2. **Rust ABI 稳定性**：`String`/`Vec` 不能跨边界，用 `abi_stable` 或 C ABI + 手动内存管理。
3. **单进程下插件崩溃 = 整个进程崩**：无隔离，插件 segfault 会带崩机器人主循环。对自研可控插件非致命，但 unsafe 部分要极度克制，急停等安全逻辑不依赖插件存活。

---

## 7. 注册到 Lua 能力

### 7.1 形态

mlua 的 userdata + method 注册即可把插件能力暴露给 Lua。Rust 侧：

```rust
// 1. 加载 .so → Arc<dyn MotionDevice> → 包成适配器
let motion: Arc<dyn MotionDevice> = plugin_loader::load("stm32.so")?;
let adapter = MotionAdapter(motion);

// 2. 注册成 Lua 方法 —— 闭包只做薄胶水
let methods = lua.create_table()?;
methods.add_method("move_forward", |_, a: MotionAdapter, ()| {
    a.0.move_forward().map_err(|e| mlua::Error::runtime(e))
});
```

Lua 侧：

```lua
local motion = robot.capability("motion")
local lidar  = robot.capability("lidar")
local beep   = robot.capability("beep")

if motion:capabilities().can_forward then
    motion:move_forward()
end
beep:apply({ kind = "beep", param = 1000 })
```

### 7.2 三个设计点

1. **注册粒度**：起步按「类别固定接口」生成固定形态 userdata（简单、类型安全）；进阶才做「插件自描述」（导出方法名 + schema，主程序动态生成绑定）。
2. **跨 ABI 双重转换**：`Lua 类型 → Rust String/Value → 稳定 ABI(RString) → .so`，两层转换逻辑放独立 `impl` 方法，闭包只留最外层 `map_err`。
3. **感知数据上行不走 Lua 实时回调**：高频数据 Rust 侧直接消费（建图/状态更新/急停），Lua 只查快照（`grid:query(x,y)`、`state:pose()`）；低频事件（障碍/任务完成）才走事件通道。

### 7.3 四层架构

```
🟣 Lua 行为脚本（决策/编排层）
   car_behavior.lua / uav_behavior.lua —— robot.capability("motion")
        ▲ 薄胶水闭包（Lua ↔ Rust 类型转换）
🔵 能力注册层（Rust 主程序）
   加载 .so → MotionAdapter/SensorAdapter → userdata；高频数据 Rust 消费、Lua 查快照
        ▲ 稳定 ABI（abi_stable / C ABI）
🩵 设备插件层（.so，按类别分）
   MotionDevice / SensorDevice / EffectorDevice；一插件可实现多类
        ▲ 串口 / 网口
⬛ 硬件层
   STM32 / MAVLink / YDLIDAR / 蜂鸣器 / 舵机
```

### 7.4 现状缺口

`programs/user/robot_test.lua` 引用的 `robot.*` 绑定**当前未注册**（`Src/VM/capability_binding.rs` 只有 `caps.*`/`ml.*`/`network.*` 等），是跑不起来的遗留脚本。注册层无论走编译期还是 `.so`，**Lua 侧接口形态一致**，可先编译期做扎实，切 `.so` 时 Lua 脚本零改动。

---

## 8. 方案 D：executor 用 `.so` 替代 Lua（最终方向）

### 8.1 关键发现：executor 已是无状态纯函数

`Src/Robot/core/executor.rs` 头部注释：

```
//! 决策执行器（Task 22_5 方案二：决策层回退 Rust）
//! 逻辑等价于原 car.lua 的 on_tick 与 Godot-Library 的 executor.rs 三状态机。
```

**决策层原本是 `car.lua`，在 Task 22_5 已回退为 Rust**。「不需要 Lua」是团队已经做出的方向选择。

`decide()` 接口（L38-45）：

```rust
pub fn decide(
    &self,
    next_cell: Option<(i32, i32)>,  // GoalService 寻路结果
    x: f32, y: f32, yaw: f32,       // 当前位置/航向
    current: &ExecuteState,         // 当前执行状态
) -> DecisionResult
```

**无状态、纯决策、零副作用**：决策所需状态全部由参数传入，不读 `RwLock<RobotState>`、不读 `Grid`、不读 `ClusterInfoTable`。这消解了「跨 ABI 共享状态」这个最大难点——进出全是标量和小结构。

### 8.2 C ABI 形态（全标量，零内存管理）

```c
struct Decision {
    uint8_t state;                 // Idle/Turning/Moving
    int32_t sub_gx, sub_gy; bool has_sub;
    uint8_t action;                // MoveForward/TurnLeft/...  bool has_action;
};

struct Decision exec_decide(
    int32_t next_gx, int32_t next_gy, bool has_next,
    float x, float y, float yaw,
    uint8_t cur_state, int32_t cur_sgx, int32_t cur_sgy, bool cur_has_sub
);
```

**比设备插件还好做**——设备插件要处理串口/回调/生命周期，executor 插件就是一个「输入标量 → 输出决策」的纯函数，每次 tick 调一次。

### 8.3 收益

1. **技术栈统一为纯 Rust**：去掉 Lua 解释器、mlua 绑定层、第二语言心智负担。
2. **一套插件机制**：设备与 executor 都走 `.so`，一套加载器、一套 ABI 约定。
3. **类型安全 + 性能**：决策逻辑编译期检查，50ms tick 零解释开销、零 GC。
4. **改决策不重编主程序**：车/机各一个 `executor.so`，改谁的决策就重编谁。

### 8.4 代价与对策

1. **热更新丧失**：`.so` 仍要重编（几十秒~几分钟）。`executor.rs` 里 `TURN_ALIGN_RAD`/`STRAIGHT_ALIGN_RAD`/`SUB_TARGET_THRESHOLD_M` 恰是**最需现场调**的阈值。对策：**决策骨架留 `.so`，阈值参数外置**（TOML 配置或 Lua 参数表），实现「调参不重编、改逻辑才重编」。
2. **确认 Lua 无其他用途**：Lua 潜在用途仅剩「运行时热更新行为」「低门槛脚本接口」。若都不需要，则干净移除。

---

## 9. 结论与演进路线

### 9.1 最终形态

- **设备层**：运动/感知/执行器三类 trait + `.so` 插件 + `DeviceRegistry`（`device_type → factory`）装配。
- **决策层**：executor 作为无状态纯函数 `.so` 插件，阈值参数外置。
- **安全层**：急停等硬安全逻辑留在 Rust 最内层，不可被任何插件绕过。
- **技术栈**：纯 Rust + `.so`，去掉 Lua。

### 9.2 演进节奏（三步，接口形态前后一致）

1. **现在**：`MotionDevice` 拆成三类 trait + `DeviceRegistry`（编译期装配，纯 Rust 重构，不引入 FFI 复杂度，先把接口契约定下来）。
2. **接着**：executor 抽 trait + 阈值外置配置。
3. **将来**：接口契约稳定后，把注册表工厂从「编译期函数指针」换成「`libloading` 加载 `.so`」，几乎无缝迁移。

---

## 10. 待定决策点

1. 感知类数据上行通道的最终选型：**回调 vs 轮询 vs 共享环形缓冲**（这是抽象里唯一有实质分歧的点，会反向影响 `Robot::launch` 重构方式）。
2. Lua 的去留：确认「运行时热更新」「低门槛脚本」是否仍被需要。
3. 能力与行为脚本的匹配方式（若保留 Lua）：`device_type → script` 固定映射 vs 能力声明驱动。
