# task_41_containment — sim 世界封控模拟实验

> Presented by KeJi
> Created Date ： 2026-10-30
> Modified Date ： 2026-10-30
> 状态：**Step 1 ~ Step 4 定稿**（2026-10-30，人类确认）。

## 一、目标

在 sim 世界中完成封控的运行。

---

## 二、已定口径（人类 2026-10-30 确认）

1. 封控与 `Goto` / `Follow` 同级：是一个 `Mission` 变体 + 一棵可运行的行为树，由 Pictor 下发。
2. **sim 机栈行为**：在正方形范围内巡逻；识别到人且算出的位置落在正方形内 → 跟随该人；人离开范围 → 回到正方形巡逻。
3. sim 飞机速度 **0.8 m/s**（只改 sim 实例配置；真机后议）。1.5 m/s 实测巡逻只有 0.33 m/s（0.5 m 前瞻 + 无 pass-through），已回退。
4. Pictor 端要加对应指令（协议常量 + 编码 + 解码 + CLI）。
5. `TASK_SET` 记录改为**按 type 变长**（不再固定 `1 + 9n`）。
6. 真车、真机、sim ugv 的 Containment 子树 Step 1 用 **naive**：建树即一拍 `Success`，不动。
7. 变长解码的必然结果：**未知 mission type → 整帧拒**（warn，队列不动）；旧端遇到含新类型记录的帧也整帧拒。
8. sim 机栈未启用 detector 时，Containment = **原地 Hold（`Running`）+ warn**（不得一拍 `Success`，否则任务静默消失）。
9. **协助请求**：UAV 在 containment 的 `Condition` 里发；首次看到立即发，之后目标移动 **> 1 m** 再发；人离开/丢失清状态（再进来立即发）。topic = `pleiades/robot/assist_request`，msgid = `7`，payload = `x f32 | y f32`。
10. **Pictor 协同控制模块**（新增）：`robot_frame` → `kernel_bridge` → EventBus → 模块；模块算人四周 3 m 的 4 个点，按**最近一对一**分配，**每次给 4 车都发** Goto（接受 reset 顿挫）。
11. **步骤划分**：Step 3 = 行为树 v2 + 新 topic + Pictor 实现；Step 4 = 对 Step 3 的验证。**Step 2 只验证 tree v1**，v2 必须在 Step 4 复验。
12. **Step 4 车速**：UGV 2 m/s（`[sim_world].chassis_params max_speed = 2.0` + `[chassis] forward_speed = 100`），`turn_speed = 35`（90 × 35% = 31.5 °/s，r ≈ 3.6 m；死区 8° 的过冲上限约 33 °/s，35% 在界内，不改代码常量）。真机不管。
13. **行人速度 0.5 m/s**（UAV 0.8 m/s 追得上；Step 2/4 的等待窗相应放宽）。
14. **detector 换 `yolov8s.safetensors`**：来源 HF `lmz/candle-yolo-v8`（355 keys，与 `yolov8n` 完全一致；已拷入 `RobotSim_TestEnv/simq_uav_containment/Pleiades_Workspace/` 与本仓 `Pleiades_Workspace/`；本机实测可加载/可检测，单帧端到端约 0.67 s）。用于压 phantom；CPU 上检测率可能低于 2 Hz，Step 2 的 P6 按 ≥1 Hz 记，GPU 环境期望 ≥2 Hz。
15. **detector 目标过滤（Step 2 第三轮前修复，人类批准）**：`Target` 只收 `class_name == "person"`（`pleiades-sim/src/sim/detector/mod.rs` 与 `pleiades-uav/src/device/detector/mod.rs` 两份副本）；非 person 仍进 `Detections`/日志，但标 `判定=非person（忽略）`。起因：Step 2 实测 bench（conf 0.73）被当成入侵者，UAV 停在椅子旁 2.3 m。单测 `test_is_person_only_person`。

> Step 1 的实现口径（细节，如有异议改这里）：
> - 多目标时沿用 `follow.rs` 的「取 `range_m` 最近的一个」；不区分该目标在不在范围内（最近的在范围外即视为无入侵者）。
> - 「人离开范围」= 本拍目标位置出正方形，或目标丢失（`age > STALE_MS`）→ 回巡逻；Step 1 不加边界迟滞。
> - 正方形按世界轴对齐；巡逻角点顺序见 §3.2。

---

## 三、Step 1 — 封控指令贯通（base 变体 + 变长 TASK_SET + Pictor + sim 机栈树 + 三端 naive）

### 3.1 要做什么（五条）

| # | 项 | 内容 |
|---|---|---|
| 1 | `Mission` 第三变体 | `Mission::Containment { x, y, half_side }`：`x/y` = 正方形中心，`half_side` = 半边长（米，>0）。 |
| 2 | `TASK_SET` 变长 | 记录长度按 type：`0/2 → 9 B`、`3 → 13 B`；总长 `1 + Σlen`；未知 type / 长度不符 / `half_side` 非法 → 整帧拒。 |
| 3 | Pictor 端 | `MISSION_TYPE_CONTAINMENT = 3`；编码/解码按 type；`half_side` 进 missions 字典；CLI 加 `containment` 子命令；未知 type 由「降级为 Goto」改为**报错不发帧**。 |
| 4 | sim 机栈树 | 新文件 `sim/uav_sim/mission/containment.rs`：`ReactiveFallback[ ReactiveSequence[ InsideSquare, Follow子树 ], Patrol ]`。 |
| 5 | 三端 naive | 真机 ugv、真机 uav、sim ugv：`warn` + `Action` 恒 `Success`（照 `Follow` 现成写法），并各加一条单测。 |

### 3.2 线格式与接口（Step 1 的契约）

```
TASK_SET payload = mission_count u8 | mission[0] | ... | mission[count-1]

type 0  Goto        : type u8 | x f32 BE | y f32 BE                    = 9 B
type 2  Follow      : type u8 | x f32 BE | y f32 BE                    = 9 B
type 3  Containment : type u8 | cx f32 BE | cy f32 BE | half f32 BE    = 13 B
```

- `count == 0` 仍是唯一取消入口。
- 解码：读 `count` → 逐条先读 `type` → 查长度表 → 读该条 → 循环结束后要求「偏移 == payload 长度」，否则 `None`。
- 未知 type：无法知道长度 → **整帧 `None`**（调用方 warn，队列不动）。
- `half_side`：必须 `is_finite()` 且 `> 0.0`，否则整帧拒。
- `1` 仍是退役的旧 `Circle`，**不得复用**；`3` 为 Containment。
- 兼容性：旧接收端按 `1+9n` 校验，含 13 B 记录的帧总长不符 → 整帧拒；车/机/终端/Pictor 必须同批升级。
- 正方形判定：`|x − cx| ≤ half_side && |y − cy| ≤ half_side`（世界轴对齐）。
- 巡逻角点（顺序即初始航向，写死）：`(cx−h, cy−h) → (cx+h, cy−h) → (cx+h, cy+h) → (cx−h, cy+h)`。

**树形（sim 机栈）**

```
ReactiveFallback
├─ ReactiveSequence
│  ├─ InsideSquare         // 有目标 && 未过期 && 目标 (x,y) 在正方形内
│  └─ follow::build(...)   // 复用现有 Follow 子树（TargetGate / Step / Hold）
└─ RepeatForever
   └─ Sequence [ goto::build(角1), goto::build(角2), goto::build(角3), goto::build(角4) ]
```

- 必须用 `Reactive*`：本仓 `Fallback` 是记忆式、`Follow` 的 Hold 恒 `Running`，组合会锁死（task_37 已踩过）。
- `InsideSquare` 与 `follow` 子树**共享同一个 `Arc<TargetSource>`**，避免两个快照互相矛盾。
- 人离开 → `ReactiveSequence` 失败并 halt Follow → `ReactiveFallback` 切巡逻；人再进入 → 每拍从高优先级重判，halt 巡逻回跟随。
- 巡逻每到一个角返回 `Success`；`RepeatForever` 在整串 `Success` 后 halt 子树并回 `Running`，下一拍从角 1 重新开始。
- 持续任务：正常永远 `Running`，只有巡逻某角超时（`goto` 自带 `Timeout(60 s)`）才 `Failure`。

### 3.3 涉及文件

| 文件 | 动作 | 内容 |
|---|---|---|
| `pleiades-base/src/Robot/core/mission_queue/element.rs` | 改 | `Mission` 加 `Containment { x, y, half_side }` + 注释（type=3）。 |
| `pleiades-base/src/Robot/core/protocol/messages.rs` | 改 | `MISSION_CONTAINMENT = 3`；`MissionItem` 加 `half_side`（0/2 恒 0.0）；`encode_task_set` / `decode_task_set` 按 type 变长；长度表；单测。 |
| `pleiades-base/src/Robot/core/protocol/command_decode.rs` | 改 | type 3 → `Mission::Containment`；未知 type 由「逐条丢」改为整帧拒（`decode_task_set` 已拒，这里保留兜底 warn）。 |
| `pleiades-sim/src/sim/uav_sim/mission/containment.rs` | **新增** | `build(...)` + `InsideSquare` 条件节点 + 巡逻角点 + 单测。 |
| `pleiades-sim/src/sim/uav_sim/mission/mod.rs` | 改 | type 3 分派：有 detector → `containment::build`；无 detector → warn + 原地 Hold（`Action` 恒 `Running`）。 |
| `pleiades-sim/src/sim/ugv_sim/mission/mod.rs` | 改 | naive：warn + 一拍 `Success` + 单测（照 `Follow`）。 |
| `pleiades-ugv/src/ugv/mission/mod.rs` | 改 | 同上（真车）。 |
| `pleiades-uav/src/uav/mission/mod.rs` | 改 | 同上（真机）。 |
| `Pictor/src/websocket/protocol/protocol_def.gd` | 改 | `MISSION_TYPE_CONTAINMENT := 3`；`Mission_Type_From()` 认 `"containment"` / `"contain"`；更新注释。 |
| `Pictor/src/websocket/protocol/orion_messages.gd` | 改 | `Encode_Task_Set` / `Decode_Task_Set` 按 type 变长；`half_side` 字段；未知 type 报错不发帧。 |
| `Pictor/src/websocket/protocol/message_builder.gd` | 改 | `build_task_set` 允许 `half_side`；`_Normalize_Mission_Type` 不变（委托 `ProtocolDef`）。 |
| `Pictor/src/kernel/ws_command_server.gd` | 改 | `task_set` 校验：type=3 缺 `half_side` / `<=0` → `ok:false`，一帧不发。 |
| `Pictor/src/test/test_orion_protocol.gd` | 改 | containment 金标、混合列表、未知 type 报错。 |
| `Pictor/tool/pictor_ws_ctl.py` | 改 | `expected_task_set_payload` 按 type；`containment` 子命令；金标；`--type` 未知探针改判「明确报错」。 |
| `Pictor/docs/orion_protocol.md` | 改 | TASK_SET 变长布局、type 3、兼容性说明。 |
| `Architecture/robot_arch.md` | 改 | `Mission` 两变体 → 三变体；§4.3 的 type 列表加 3；记录变长协议。 |

> Pictor 是独立 git 仓（根 `.gitignore` 排除 `/Pictor/`），单独提交。

### 3.4 详细步骤（按序）

**A. base 协议层**

1. `element.rs`：`Mission` 加第三变体：
   ```rust
   Containment { x: f32, y: f32, half_side: f32 },
   ```
   注释写清：type=3、`x/y` 中心、`half_side` 半边长（米、>0、轴对齐）、只有 sim 机栈真实现。
2. `messages.rs`：
   - 加 `pub const MISSION_CONTAINMENT: u8 = 3;`（注释写明 `1` 仍是退役 Circle）。
   - `MissionItem` 加 `half_side: f32`（注释：仅 type=3 编码，其余恒 0.0）；同步现有所有结构体字面量（测试）。
   - 加内部函数 `mission_wire_len(t: u8) -> Option<usize>`：`0|2 → 9`、`3 → 13`、其余 `None`。
   - `encode_task_set`：容量按 `Σlen` 算；逐条写；`count` 仍截断到 `u8::MAX`；type 不在表内时记 warn 并跳过该条（编码端不应出现）。
   - `decode_task_set`：先读 `count`；循环内先读 `type` 再查长度；越界/未知/`half_side` 非法 → `None`；循环后偏移必须等于 `payload.len()`。
   - 单测（新增/改）：
     * Goto / Follow 编码字节**逐字不变**（现有金标保留）；
     * 单条 Containment = `1+13`，字节序/浮点大端正确；
     * 混合 `[Goto, Containment, Follow]` 解析顺序与值；
     * 截断 1 B / 多 1 B → `None`；
     * `count` 与实际条数不符 → `None`；
     * 未知 type（如 9）→ `None`；
     * `half_side = 0` / 负数 / NaN → `None`；
     * `count=255` 容量与旧上限行为。
3. `command_decode.rs`：`MISSION_CONTAINMENT => Some(Mission::Containment { x: m.x, y: m.y, half_side: m.half_side })`；`filter_map` 的未知分支保留但应不可达（`decode_task_set` 已拒），warn 文案同步。

**B. sim 机栈 containment 树**

4. 新建 `pleiades-sim/src/sim/uav_sim/mission/containment.rs`：
   - 常量：巡逻角点顺序、`STALE_MS` 复用 `follow::FollowParams`。
   - `build(nav, robot_state, source: Arc<TargetSource>, center: (f32,f32), half: f32) -> Box<dyn Node>`：
     * `InsideSquare` 节点：读 `robot_state` 取 `me`；`source.sample(Instant::now(), me)`；`Success` 条件 = `!follow::lost(...) && |s.x−cx| ≤ h && |s.y−cy| ≤ h`；否则 `Failure`。
     * Follow 子树 = `follow::build(nav.clone(), rs.clone(), source.clone(), FollowParams::default())`。
     * Patrol = `RepeatForever::new(Sequence::new(vec![goto::build(c1…), goto::build(c2…), goto::build(c3…), goto::build(c4…)]))`。
     * 外层 `ReactiveFallback::new(vec![ReactiveSequence::new(vec![inside, follow]), patrol])`。
   - 单测（用 `follow.rs` 已有的 `TargetFeed`/`FakeFeed` 与替身 nav）：
     * 目标在正方形内 → 走 Follow（断言 `nav.set_goal` **未**被调用、`next_cell` 来自几何步）；
     * 目标在正方形外 → 走 Patrol（断言 `set_goal` 为角 1）；
     * 目标从内到外 → 切回巡逻；
     * 无目标 / `age > STALE_MS` → 巡逻；
     * 巡逻到角返回 `Success` 后，下一拍重新从角 1 开始（`set_goal` 序列 `c1,c2,c3,c4,c1`）。
5. `sim/uav_sim/mission/mod.rs`：加 `pub mod containment;`；`Mission::Containment { x, y, half_side }` 分支：
   - `deps.detector = Some(dev)` → `Ok(containment::build(deps.nav.clone(), deps.robot_state.clone(), Arc::new(TargetSource::new(Some(dev.clone()))), (*x,*y), *half_side))`；
   - `None` → `warn!` + `Action` 恒 `Running` + `next_cell = None`（照 Follow 的退化写法）。

**C. 三端 naive**

6. `pleiades-sim/src/sim/ugv_sim/mission/mod.rs`、`pleiades-ugv/src/ugv/mission/mod.rs`、`pleiades-uav/src/uav/mission/mod.rs`：
   - 加分支：`warn!("[X] Containment 未实现，跳过（平凡树：一拍 Success）");` + `Action::new(|_| Status::Success)`；
   - 各加一条单测：一拍 `Success`、`next_cell=None`、不返回 `Err`。
   - **不得**返回 `Running`（会永久占队首）。

**D. Pictor**

7. `protocol_def.gd`：`MISSION_TYPE_CONTAINMENT := 3`；`Mission_Type_From()` 认 `"containment"`/`"contain"`；注释保留「`1` 不得复用」。
8. `orion_messages.gd`：
   - `Encode_Task_Set`：按 type 算每条长度（0/2 = 9，3 = 13），总长 `1+Σ`；未知 type → `printerr` + 返回空/`false`，**不降级、不发帧**；
   - `Decode_Task_Set`（若有）：按 type 变长；末尾长度必须精确；
   - 单测金标相应更新。
9. `message_builder.gd`：missions 字典支持 `{type, x, y, half_side}`；`build_task_set` 不再按 3 键拷贝。
10. `ws_command_server.gd`：`task_set` 逐条校验；type=3 缺 `half_side` 或 `<=0` → `{"ok":false,"error":"bad half_side"}`，整帧不发。
11. `pictor_ws_ctl.py`：`expected_task_set_payload` 按 type 生成；新增 `containment --cx --cy --half` 子命令；金标（单条 14 B、混合 23 B）；`--type 9` 改为断言「明确报错/不发帧」。
12. `test_orion_protocol.gd`：containment 单条/混合/未知 type 用例。

**E. 文档**

13. `Pictor/docs/orion_protocol.md`：TASK_SET 变长布局、type 表（0/2/3）、未知 type 整帧拒、兼容性（同批升级）。
14. `Architecture/robot_arch.md`：`Mission` 三变体；§4.3 的 type 列表；记录「变长 TASK_SET」与 naive 桩现状。

### 3.5 单测与验收（Step 1 不跑封控实验）

1. 编译：`cargo check -p pleiades-base --all-targets`、`-p pleiades-ugv`、`-p pleiades-uav`、`-p pleiades-sim` 全 `Finished`、0 error、新增代码 0 warning。
2. base：`cargo test -p pleiades-base --lib protocol`（或按 target 实际选择）；新增变长用例全绿，Goto/Follow 旧金标逐字不变。
3. 三端 naive：`cargo test -p <crate> --bins mission`，各端「一拍 Success」用例绿。
4. sim 机栈树：`cargo test -p pleiades-sim --bins containment`（建树/切换/巡逻循环）全绿；原有 `follow` / `goto` 用例不回归。
5. Pictor：`godot --headless --path Pictor -s src/test/test_orion_protocol.gd` 全绿；Python `expected_task_set_payload` 的金标与 Godot 编码一致。
6. 字节契约：单条 Goto = 10 B（`count+9`）、单条 Follow = 10 B、单条 Containment = 14 B、`[Goto,Containment]` = 23 B；旧端遇 14/23 B 整帧拒。
7. `git diff --stat` 只含 §3.3 表内文件（Pictor 单独仓单独看）。

### 3.6 不做（Step 1）

- 不跑封控实验、不启动 Godot/sim 长跑（Step 2 再跑）；
- 不做真车/真机/sim ugv 的真子树（只 naive）；
- 不做 Pictor UI 按钮、不做画框、不做多区域/旋转正方形；
- 不做多目标筛选策略、不做边界迟滞、不做跟踪平滑；
- 不改 `follow.rs` / `goto.rs` / `nav/` 的行为；
- 不改 sim 实例的 `[flight_ctrl].vel_fwd = 1.5`（属于实验运行前置，写在 Step 2）；
- 不做多机协同（这是 sim uav 单机行为）。

---

## 四、Step 2 — sim 机栈 containment 验证

> 执行方式：执行引擎串行推进（`Tool/execution_engine_task41.ts`，待建）。**本步 FAIL → 程序立即终止**（不进入 Step 3+），按 §4.6 发邮件汇报；PASS → 继续。
> **注意**：本步验证 **tree v1**（巡逻 + 跟随，不含协助请求）；Step 3 升级到 v2 后，Step 4 必须复验 P1~P4。

### 4.1 目标

在 sim 世界端到端验证 Step 1 的封控指令：Pictor 下发 type=3 → sim 机栈建树 → 正方形巡逻 → 人进入范围后跟随 → 人离开后回巡逻。**只验不改**：不调阈值、不改判据、不改被测代码。

### 4.2 前置（环境与配置）

- Step 1 全部验收项通过（编译 / 单测 / Pictor 自检 / 字节契约）。
- `cargo build -p pleiades-sim --release` 重建，二进制拷入实例目录，sha 与 `target/release/pleiades-sim` 一致。
- 新建实例 `RobotSim_TestEnv/simq_uav_containment/`（由脚本从 `simq_s4_uav` 模板生成，**不改旧实例**）：
  - `peer_name = "simq_uav_containment"`、`listen_port = 9125`、`[control].port = 9122`（避开 task 40 的 9121/9124）；
  - `[flight_ctrl] vel_fwd = 0.8`、`yaw_rate_deg = 30.0`；
  - `[sim_world].chassis = "quadrotor_uav"`、`[[sim_world.sensor]] camera`；
  - `[detector] enabled = true`、`model = "yolov8s.safetensors"`，`[detector.camera]` 与 `[[sim_world.sensor.mount]]` 数值一致（x=0.20 / y=0 / z=0.045 / yaw=1.5708 或 90° / pitch=−15）；
  - 权重 `yolov8s.safetensors` 在实例 `Pleiades_Workspace/`。
- 世界：`RobotSim_TestEnv/world_fast/`（`msaa_3d=0`）+ `xvfb-run -a`；启动前 `diff -r` 与 `orion-simulation-world/`（除 `override.cfg`）比对，确认副本未过期。
- Pictor：`godot --headless --path Pictor -- --ws-port=9100`，**CWD = `RobotSim_TestEnv/pictor_ws`**（kernel 读 CWD 的 `.config`）。
- 行人：`orion-simulation-world/tool/pedestrian/pedestrian_client.py`，必须 `python3 -u`。
- 端口前置检查：9999（世界 WS）、9122（sim 控制口）、9100（Pictor WS）、9125（libp2p）全部空闲。

### 4.3 用例（三段；含放人等待上限，总时长约 300~360 s）

| 段 | 场景 | 做什么 |
|---|---|---|
| S2-A 巡逻 | 无行人 150 s | 发 `{"cmd":"task_set","missions":[{"type":3,"x":64,"y":64,"half_side":8}]}`，观察无人机绕 (64,64)、半边长 8 m 的正方形巡逻。 |
| S2-B 进入跟随 | 行人（0.5 m/s）从 (64,80) 走到**南边界内 (64,72)** 停 20 s → 再走到中心 (64,64)（约 20 s） | 观察 detector 是否识别、是否切到跟随、是否跟得住。**放人时机**：等 UAV 位于南边（`|y−72| ≤ 0.5 m`）、航向朝 +x 且 `x ∈ [56,60]` 时再放（保证人在正前方 4~8 m 的 FOV 内）；等待上限 120 s。 |
| S2-C 离开回巡逻 | 行人从 (64,64) 走向 (64,50) 并离开范围 | 观察是否在 20 s 内回到正方形巡逻。 |

> 正方形范围 56 ≤ x,y ≤ 72；该区域为空旷区（pillar_e 在 (78,78) 之外），无人机 2.5 m 高度无遮挡。
> **为什么不让人停在中心**：相机前视、水平半 FOV 约 43°，巡航沿边飞时中心偏离航线约 8 m ≈ 90°，基本不在画面内；停在边上、且保证人在机头前方，才能稳定触发「识别到人」。

### 4.4 判据（P1~P7）

| # | 判据 | 口径 |
|---|---|---|
| P1 | 任务贯通 | sim 日志出现 `[Robot] 开始任务` + `[Sim/uav] 任务 … 建树完成`（Containment）；**无** `未知 mission type`；Pictor `ack.ok = true`。 |
| P2 | 巡逻 | S2-A 150 s 内，UAV 位姿距**至少 2 个角点** ≤ 1.0 m；`next_cell` 属于角点集合；无 `任务失败` / `Timeout`。 |
| P3 | 进入跟随 | **从行人 spawn 之后**首次出现「person 目标且 56 ≤ x,y ≤ 72」起算：5 s 内 `next_cell` 离开角点集合；随后 10 s 内 UAV 水平位移 **≥ 2.5 m**；末 5 s 的 UAV-人距离 ≤ 首 5 s 距离 + 2 m。 |
| P4 | 离开回巡逻 | 人出范围（`x<56 或 x>72 或 y<56 或 y>72`）后 **20 s 内**，`next_cell` 回到角点集合，且朝该角点位移 ≥ 2 m。 |
| P5 | 安全 | UAV `0 < x < 128`、`0 < y < 128`；单拍位置突跳 > 1 m 的次数 = 0（撞墙/瞬移）；无持续卡死（连续 5 s 位置变化 < 0.05 m 且任务未结束）。 |
| P6 | 配置生效 | `[Sim] UAV` 行出现 `v=0.8`（±0.05）；`位姿龄` 中位 ≤ 120 ms；detector 帧率 ≥ 1 Hz（CPU + yolov8s；GPU 环境期望 ≥2 Hz）。 |
| P7 | 无异常 | 无 panic、无 `任务失败`、无 `Blocked`；退出后无残留进程/端口；日志无新增 `ERROR`。 |

> 判据阈值是 Step 2 口径；**不得为了变绿而改判据、改脚本口径或调参数**。若判据本身不可测（取不到实测值），该条记 `FAIL(env)` 并说明原因。

> **人类裁决（2026-10-30）**：Step 2 第三轮实测 P3b = **2.87 m**（原判据 ≥3 m）。人类认定「这个验证已经通过」，不因 0.13 m 卡住；原 3 m 是拍脑袋阈值，已改为 **≥2.5 m**（本批数据 2.87 m 通过），后续 Step 4 复验同样按 2.5 m。其余 P1/P2/P4/P5/P6/P7 均为真实 PASS。

### 4.5 取数与产物

- 脚本：`Tool/sim_task41_containment_test.py`（起停/控制口/日志解析复用 `Tool/sim_task40_ugv_test.py`、`Tool/sim_follow_test.py` 的既有设施）。
- 产物目录：`RobotSim_TestEnv/results/task41_s2_<UTC>/`：
  - `verdict.md`（P1~P7 逐条实测值 + PASS/FAIL + 结论）
  - `sim.log` / `world.log` / `ped.log` / `pictor.log`
  - `trace.csv`（t_world, x, y, z, yaw, v, omega, next_cell, person_x, person_y, dist）
  - `cases.json`（用例时间窗与配置快照）
- 结论与关键数字写 `Workbook/wb_41_containment.md`（若不存在则新建）。

### 4.6 失败处理与汇报（人类要求）

1. **P1~P7 任一 FAIL（或 `FAIL(env)` 导致本步无法判定）→ 立即终止执行程序**，不推进 Step 3+，不做"再试一组参数"。
2. 用邮件向人类汇报（脚本见 §4.7），**附件 = `verdict.md`**。
3. 邮件发送失败（退出码非 0）：重试 **1 次**；仍失败则把标题/正文写入结果目录 `EMAIL_FAILED.txt`，并在程序输出里打印全文；**无论邮件成败，程序都必须终止**。
4. PASS → 继续 Step 3（**不发邮件**）。
5. 汇报正文至少包含：

```
Step 2（sim 机栈 containment 验证）FAIL，执行程序已终止。
失败判据：P3（实测：……；阈值：……）
产物目录：/…/RobotSim_TestEnv/results/task41_s2_<UTC>/
关键日志：<≤10 行>
已做的：只验不改（未调参、未改判据、未重跑）。
下一步：等人类裁决。
```

邮件标题：`[Orion][task_41] Step 2 FAIL — sim uav containment`。

### 4.7 邮件工具（已核实存在）

- **路径**：`/vepfs-mlp2/c20250205/240804016/tool/mailer.py`（脚本 docstring 里写作 `email.py`，实际文件名是 `mailer.py`；位于本仓库上两级目录的 `tool/` 下）。
- **用法**：`python3 /vepfs-mlp2/c20250205/240804016/tool/mailer.py "标题" "正文" [附件路径]`
- **固定收件人** `liyongkang_ce@163.com`；发件 `columba_keji@163.com`；SMTP `smtp.163.com:465`（SSL，授权码已内置）。
- **退出码**：`0` 成功 / `1` 用法错 / `2` 授权码为空 / `3` 附件读取失败 / `4` 认证失败 / `5` 发送失败。
- **限制**：一次只能带 1 个附件；正文纯文本；需要外网 465（**已实测本机可达**）。
- **已核实**（2026-10-30）：文件存在；`python3 tool/mailer.py` 能正常导入并打印用法（退出码 1）；**未实际发信**（避免打扰）。

### 4.8 不做（Step 2）

- 不调阈值/不改判据/不改被测代码；
- 不测多机、多目标、多区域、旋转正方形；
- 不测真机、不做 Pictor UI；
- 不改 `follow.rs` / `nav/` / `goto.rs`。

---

## 五、Step 3 — 行为树 v2 + 协助请求链路 + Pictor 协同控制（实现）

> 本步只做实现与单测，不跑端到端；端到端在 Step 4。

### 5.1 目标

把「UAV 发现人 → 请求地面站协助 → 地面站指挥 4 车包围」这条链路实现；行为树从 v1（巡逻 + 跟随）升到 v2（`Condition` 内发协助请求）。**不改变** v1 的巡逻/跟随决策。

### 5.2 要做什么（五条）

| # | 项 | 内容 |
|---|---|---|
| 1 | Network 新 topic | `pleiades/robot/assist_request`（`TOPIC_ROBOT_ASSIST_REQUEST`），加入 `swarm_events` 的转发白名单。**人类已批准本任务破例修改 Network 层**。 |
| 2 | 协议 | `MSGID_ASSIST_REQUEST = 7`；payload `x f32 BE | y f32 BE`（8 B，严格）。 |
| 3 | UAV 树 v2 | `Condition` 内：在正方形内且（首次 / 移动 > 1 m）→ 发 assist request；离开/丢失清状态。巡逻与 Follow 子树一行不改。 |
| 4 | Pictor | 解析 msgid 7 → EventBus → 新 `coop_controller.gd`：4 点（3 m 环）→ 最近一对一 → 逐车发 Goto。 |
| 5 | 单测/文档 | 协议 8 B 金标、树发送逻辑、Pictor 点位/分配纯函数；`orion_protocol.md` 与 `robot_arch.md`。 |

### 5.3 线格式

```
topic  = "pleiades/robot/assist_request"
msgid  = 7
payload = x f32 BE | y f32 BE        (8 B，长度必须严格 8)
frame   = encode_frame(7, UAV_peer_id, COMPID_ROBOT(1), payload)
```

- 只有地面站订阅（终端 `[Network].subscribe_topics` 加该 topic）；车/机/sim 不订阅。
- 帧的 `sysid` = UAV 的 PeerId，Pictor 侧据此知道请求者。

### 5.4 涉及文件

| 文件 | 动作 | 内容 |
|---|---|---|
| `pleiades-base/src/Network/Gossipsub/mod.rs` | 改 | `TOPIC_ROBOT_ASSIST_REQUEST` 常量。 |
| `pleiades-base/src/Network/swarm_events.rs` | 改 | 转发分支加该 topic（与 POSE/MAP/RTCM 一样原样进 `robot_bus`）。 |
| `pleiades-base/src/Network/mod.rs` | 改 | re-export。 |
| `pleiades-base/src/Robot/core/protocol/mod.rs` | 改 | `MSGID_ASSIST_REQUEST = 7`。 |
| `pleiades-base/src/Robot/core/protocol/messages.rs` | 改 | `AssistRequest { x, y }` + encode/decode（严格 8 B）+ 单测。 |
| `pleiades-sim/src/sim/uav_sim/mission/containment.rs` | 改 | Condition 内发请求（1 m 阈值/离开重置）+ `AssistPublisher` trait + 单测。 |
| `pleiades-sim/src/sim/uav_sim/mission/mod.rs` | 改 | `TaskDeps` 加 publisher；`Containment` 分支透传。 |
| `pleiades-sim/src/sim/uav_sim/mod.rs` | 改 | `Stack::new` 接收 publisher。 |
| `pleiades-sim/src/sim/sim_handler.rs` | 改 | 用 `node_handle` 构造 publisher，传给 uav 栈。 |
| `Pictor/src/websocket/protocol/protocol_def.gd` | 改 | `MSGID_ASSIST_REQUEST := 7`。 |
| `Pictor/src/websocket/protocol/orion_messages.gd` | 改 | `Decode_Assist_Request`。 |
| `Pictor/src/websocket/protocol/message_parser.gd` | 改 | 新 msgid 分支。 |
| `Pictor/src/event_bus/event_bus.gd` | 改 | `signal assist_request_received(requester_id, x, y)`。 |
| `Pictor/src/kernel/kernel_bridge.gd` | 改 | msgid 7 → 发 EventBus 信号。 |
| `Pictor/src/cooperation/coop_controller.gd` | **新增** | 点位 / 最近一对一 / 逐车下发。 |
| `Pictor/src/main/main.tscn` | 改 | 挂 `CoopController`（排在 `VehicleRegistry` 之后）。 |
| `Pictor/src/test/test_coop_controller.gd` | **新增** | 点位与分配纯函数单测。 |
| `Pictor/docs/orion_protocol.md` / `Architecture/robot_arch.md` | 改 | 新 topic、msgid 7、模块说明。 |

### 5.5 详细步骤

**A. Network（已获准破例）**

1. `Gossipsub/mod.rs`：`pub const TOPIC_ROBOT_ASSIST_REQUEST: &str = "pleiades/robot/assist_request";`
2. `swarm_events.rs`：`TOPIC_ROBOT_POSE | TOPIC_ROBOT_MAP | TOPIC_RTK_RTCM | TOPIC_ROBOT_ASSIST_REQUEST =>` 原样 `robot_bus.Publish(StreamRaw)`。
3. `Network/mod.rs` re-export；在任务文档/Workbook 记录「经人类批准修改 Network 层」。

**B. 协议**

4. `protocol/mod.rs` 加 `MSGID_ASSIST_REQUEST = 7`（注释：UAV → 地面站的协助请求，非下行命令）。
5. `messages.rs`：
   - `pub struct AssistRequest { pub x: f32, pub y: f32 }`；
   - `encode_assist_request` / `decode_assist_request`（长度严格 8，非有限值拒绝）；
   - 单测：8 B 金标、roundtrip、7/9 字节拒。

**C. UAV 树 v2**

6. `containment.rs`：
   - 定义 `pub trait AssistPublisher: Send + Sync { fn publish(&self, x: f32, y: f32); }`；
   - `InsideSquare` 增加状态 `last_sent: Option<(f32, f32)>`：
     * 条件失败（无目标 / 过期 / 出正方形）→ `last_sent = None`，`Failure`；
     * 条件成功且（`last_sent == None` 或距离 > `ASSIST_MOVE_M = 1.0`）→ `publisher.publish(x, y)`，`last_sent = Some`；
     * `halt()` 清 `last_sent`。
   - `build(...)` 增加 `publisher: Option<Arc<dyn AssistPublisher>>`；`None` 时只判条件、不发。
7. 生产实现（sim_handler 或 containment 内部）：`NodeHandlePublisher { node_handle, local_peer_id }`：
   ```
   payload = encode_assist_request(x, y)
   frame = encode_frame(MSGID_ASSIST_REQUEST, local_peer_id.as_bytes(), COMPID_ROBOT, &payload)
   let _ = node_handle.Gossipsub_Publish_Try(TOPIC_ROBOT_ASSIST_REQUEST, frame)
   ```
   失败只 warn（限流），不影响树。
8. 单测（用记录型 `FakePublisher`）：首次发、移动 0.9 m 不发、1.1 m 发、离开后重置再进立即发、publisher=None 不发、坐标非有限值不发。

**D. Pictor**

9. `protocol_def.gd` + `orion_messages.gd` + `message_parser.gd`：msgid 7 解码为 `{x, y}`；长度严格 8。
10. `event_bus.gd` 加信号；`kernel_bridge.gd` 加分支（`vid` = 请求者 sysid）。
11. `coop_controller.gd`：
    - `_ready` 连 `EventBus.assist_request_received`；
    - 取在线 UGV（`VehicleRegistry`，`node_type == "car"`），拿 `vid` 与当前位姿；
    - `N = 车数`（演示 N=4）；点：`angle_i = 2π·i/N + offset`，`p_i = (x + 3·cos, y + 3·sin)`；
    - 分配：**一对一**。N ≤ 6 时暴力枚举 `N!` 取总距离最小；更大用贪心；
    - 逐车发：`EventBus.cmd_send.emit([vid_i], {msgid: MSGID_TASK_SET, missions: [{type: 0, x: p_i.x, y: p_i.y}]})`（**一次一帧一车**）；
    - 接受 reset 顿挫，不做额外限流；记录一行 `[Coop] 指派 …`。
12. `main.tscn` 挂节点；`test_coop_controller.gd` 测：4 点等角、3 m 半径、两车/四车最近分配无重复、`N!` 最优性（构造一个明显更优的排列）。

**E. 文档**

13. `orion_protocol.md`：msgid 7 + payload；`robot_arch.md`：topic、模块、树 v2。

### 5.6 单测与验收（Step 3 不跑端到端）

1. `cargo check`：base / sim 全 `Finished`、0 error、新增代码 0 warning。
2. `cargo test -p pleiades-base --lib protocol`（含 8 B 金标）；`cargo test -p pleiades-sim --bins containment`。
3. `godot --headless --path Pictor -s src/test/test_coop_controller.gd` 全绿。
4. 字节契约：payload 恒 8 B；`[x=1.0,y=2.0]` 金标；7/9 B 拒。
5. `git diff` 只含 §5.4 表内文件（Pictor 独立仓单看）。

### 5.7 不做（Step 3）

- 不跑 sim 端到端（Step 4）；
- 不做“更新目标点不 reset”的新消息；
- 不做多 UAV、目标预测、UI 按钮；
- 不改 v1 的巡逻/跟随逻辑与 `follow.rs` / `nav/`；
- 不改真机 crate。

---

## 六、Step 4 — 对 Step 3 的端到端验证（1 UAV + 4 UGV）

> 本步验证 **tree v2** 与整条协助链路；Step 2 的结论只对 v1 有效，本步必须复验 P1~P4。

### 6.1 目标

在 sim 世界验证：UAV 巡逻 → 发现人 → 发协助请求 → Pictor 协同控制模块算 4 点 → 4 车各自收到 Goto → 围到人四周；人移动后重新分配；人离开后 UAV 回巡逻。

### 6.2 前置（环境与配置）

- Step 3 全绿；重建 `pleiades-sim --release` 并拷入各实例，sha 一致。
- 世界：`RobotSim_TestEnv/world_fast/` + `xvfb-run -a`；启动前 `diff -r` 确认副本未过期。
- 实例（全部新建，不改旧实例）：
  - 1 × UAV：`simq_uav_containment`（同 Step 2：detector/相机、`vel_fwd=0.8`、`yaw_rate_deg=30`、`model=yolov8s.safetensors`）。
  - 4 × UGV：`simq_ugv_c1..c4`：`[sim_world].chassis = "diff_drive_ugv"`、**`[sim_world.chassis_params] max_speed = 2.0`**、`[chassis] forward_speed = 100`、`turn_speed = 35`；各自的 `listen_port` / `[control].port` / `peer_name` 唯一。
  - **出生位置**：世界会把 4 车自动排在 (64,64) 的 2 m 环上（会挡住行人路线）。起跑后先用各车 control 口发 `goto` 到四角 **(52,52) / (76,52) / (76,76) / (52,76)**（正方形外、行人 x=64 路线外），等 15 s/全部停稳后再放人。
  - 为什么 `turn_speed=35`：90 °/s × 35% = 31.5 °/s，`r = 2.0/0.55 ≈ 3.6 m`；死区 8° 的过冲上限约 33 °/s，35% 刚好在界内（不改 executor 常量）。
- Pictor：`godot --headless --path Pictor -- --ws-port=9100`，CWD=`RobotSim_TestEnv/pictor_ws`；`[Network].subscribe_topics` 含 `pleiades/robot/assist_request`。
- 行人：`pedestrian_client.py`，0.5 m/s，`python3 -u`。
- 前置自检：世界 fps ≥ 5、位姿龄中位 ≤ 200 ms、5 个 sim 全部完成 assembly；不满足记 `FAIL(env)`。

### 6.3 用例

| 段 | 场景 | 做什么 |
|---|---|---|
| S4-A 巡逻 | 无人 30 s | UAV 绕正方形；4 车静止（无命令）。 |
| S4-B 包围 | 行人从 (64,80) 走到 (64,71) **停 120 s**（让 UAV 巡逻到南边、贴近到 ≤5 m 并被采纳；最多 120 s）→ 走到 (64,64) 停 20 s | 触发 assist → 4 车去 3 m 环；观察到达与分配。 |
| S4-C 跟随移动 | 行人从 (64,64) 移动到 (66,64)，停 10 s | 观察新的环、4 车重新分配、无同点。 |
| S4-D 离开 | 行人走到 (64,50) 离开 | UAV 回巡逻；4 车停在最后目标点（不再收到新 Goto）。 |

### 6.4 判据（Q1~Q8）

| # | 判据 | 口径 |
|---|---|---|
| Q1 | 链路贯通 | UAV 日志有 assist 发送；Pictor 有 `[Coop]` 收到与指派；4 车各出现 `[Robot] 开始任务`；无 `未知 msgid`。 |
| Q2 | 点位正确 | 4 个目标点两两不同；距**请求坐标** 3.0 ± 0.05 m；距人真实位置 3.0 ± 1.0 m。 |
| Q3 | 到达 | 请求后 40 s 内，每车距其目标点 ≤ 1.0 m；任意两车水平距离 ≥ 1.0 m。 |
| Q4 | 覆盖 | 4 车相对人的方位角覆盖四个象限（每象限 ≥ 1 台）。 |
| Q5 | 分配稳定 | S4-B+C 期间，同车目标点序号变化 ≤ 2 次；无两车目标点相同。 |
| Q6 | 车运动 | 全程速度 ≤ 2.05 m/s；无 `Blocked`；急停 ≤ 1 次/车；不越界。 |
| Q7 | UAV 回归 | Step 2 的 P1~P4 在 v2 上复验通过（巡逻/进入跟随/离开回巡逻）。 |
| Q8 | 无异常 | 无 panic；`[Sim]`/`[Nav]`/`[Coop]` 日志可解析；退出后无残留进程/端口。 |

### 6.5 取数与产物

- 脚本：`Tool/sim_task41_containment_test.py` 扩展（或新 `Tool/sim_task41_coop_test.py`），复用起停/解析设施。
- 产物：`RobotSim_TestEnv/results/task41_s4_<UTC>/{verdict.md, sim_uav.log, sim_ugv_c1..c4.log, pictor.log, ped.log, world.log, trace.csv, cases.json}`。
- `trace.csv`：t、UAV x/y、人 x/y、每车 x/y、每车目标点、assist 次数。
- 结论写 `Workbook/wb_41_containment.md`。

### 6.6 失败处理

同 Step 2 §4.6：任一 Q FAIL → **终止执行程序**，不调参、不改判据；用 `mailer.py` 发邮件（附件 `verdict.md`）；邮件失败重试 1 次，仍失败落盘 `EMAIL_FAILED.txt`，程序照样终止。

### 6.7 不做（Step 4）

- 不调参/不改判据/不改被测代码；
- 不测多 UAV、真机、UI；
- 不做“更新目标点不 reset”的新消息；
- 不因“车顿挫”判 FAIL（人类已接受）。

---

## 七、后续 Step

（待定）

## 八、完成判据（整体）

（待定）
