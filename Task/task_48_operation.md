# task_48_operation — 操作类任务：Face（到点朝向）+ Guard（值守警戒）

> Presented by KeJi
> Created Date ： 2026-11-05
> Modified Date ： 2026-11-05
> 状态：Step 1 ~ Step 7 待人类确认（未开始）。
> 上游结论：2026-11-05 全项目梳理（12 个子 agent）+ 与人类的讨论
> （「Face 与 Goto 平级」/「containment 车栈停距稍微调小」/「第三个演示场景 = 通道值守，命令叫 guard」）。

---

## 一、目标

本任务加两个**与 `Goto` 平级**的操作类任务，各自解决一件具体的事：

| 任务 | 解决的问题 | 一句话 |
|---|---|---|
| **`Face`** | 封控派车后，车到点就停、机头保持进场方向，不看目标 | 到点后**原地转到面向某个世界点** |
| **`Guard`** | 第三个演示场景需要一个「站岗 + 警戒区」 | 停在值守点朝一个方向；**人进 3 m 警戒区就上前**，人退出就归位 |

### 1.1 现状（代码事实）

| 事实 | 位置 |
|---|---|
| 封控树 = `ReactiveFallback[ ReactiveSequence[InsideSquare, follow], RepeatForever(4×goto) ]` | `pleiades-ugv/src/ugv/mission/containment.rs:117-140` |
| 人在界内 → `AssistPublisher.publish(x, y)`（msgid 7，payload 8 B） | 同上 `InsideSquare`；`sim_handler.rs` / `robot_handler.rs` 的 `NodeHandlePublisher` |
| 地面站收到 assist → 环点 → **每车一条** `build_auto_push_goto(p.x, p.y)` | `Pictor/src/cooperation/coop_controller.gd:218` |
| `build_auto_push_goto` 只造 `[{type:0, x, y}]` | `Pictor/src/websocket/protocol/message_builder.gd:45-49` |
| `TASK_SET` 变长表：`0|2 → 9`、`3 → 13`，未知 type 整帧拒 | `pleiades-base/src/Robot/core/protocol/messages.rs:318-323` |
| `Goto` 到点 → `Success` → `pop_next` → 下一拍无树 → `target() = None` → **停车，不转向** | `pleiades-ugv/src/ugv/mission/goto.rs:88-90`、`robot.rs:488-496` |
| 执行器的「只转不走」**已存在**：`MotionTarget::Face` → `rotate_only`（`v = 0`，`|Δθ| ≤ DEADBAND_DEG` 发 `(0,0)`，其余原地转 ±`cruise_yaw_rate`） | `base/.../behavior_tree/frame.rs:13-20`、`ugv/executor.rs:114-120` |
| 现成用例：Follow 停住时写 `face_point` 朝人 | `ugv/mission/follow.rs:307-313,347-353` |
| Follow 距离带：`d > D_FAR` 前进 / `d ≤ D_NEAR` 停 / 带内沿用上拍（迟滞） | `ugv/mission/follow.rs:126-134`；`D_FAR_M = 2.2`、`D_NEAR_M = 1.2`（`:57,59`） |
| `Containment` **明确不加边界迟滞**（出界即回巡逻） | `containment.rs:91-101` |
| `Goto` 自带 `Timeout(60 s)`，超时 = `Failure` → 出队、**任务消失** | `goto.rs:36-52`、`robot.rs:488-496` |
| 车端相机的地面可视范围（现状标定）：近 ≈0.33 m、远 ≈7.7 m（`cam_z=0.135`、`pitch=+1.7°`、`MIN_DEPRESSION=1°`） | `ugv/config.rs:83,89`、`projection.rs:43,302` |

### 1.2 为什么这样切（而不是别的形状）

| 方案 | 结论 |
|---|---|
| **`Face` 与 `Goto` 平级（本任务）** | 记录 9 B（与 Goto/Follow 同尺寸）；可单独用（只转身不移动）；队列天然给先后顺序，不用组合树；Goto 契约一行不动 |
| `GotoFace { x, y, fx, fy }`（17 B） | 记录变长、Goto 契约被改，且没法单独「转个身」 |
| 借 `Follow { x, y }` 的预留字段 | 零字节改动，但 Follow 变成「跟人 / 朝点」双语义靠魔法值区分，**不做** |
| **`Guard` 独立成任务（本任务）** | 语义是「持续值守 + 区域触发 + 归位」；`Containment` 装不下（区域形状、值守点、朝向都不同，且它的巡逻超时会**终结任务**，守卫不能这样死） |
| 把 Guard 做成 `Containment` 的小正方形参数 | 人类已否掉：Guard 是「看着面前一片区域」，不是「在小方框里绕圈」 |

---

## 二、已定口径（人类已确认，实施时不得改）

### Face（2.1 ~ 2.7）

| # | 项 |
|---|---|
| 1 | `Face` 是 `Mission` 的**兄弟变体**（与 `Goto` 平级），不是 Goto 的字段、不是 Follow 的变体 |
| 2 | 线上 `type = 4`（`1` 是退役的 Circle，不得复用）；记录 **9 B** = `type u8 + x f32 BE + y f32 BE`，`(x, y)` = **要朝向的世界点** |
| 3 | 车端语义：**原地转，不移动**（`v = 0`）；到点后的机头朝向由这一条任务负责，不修改 Goto。**四个栈都建真叶子**（`pleiades-ugv` / `pleiades-uav` / `sim/ugv_sim` / `sim/uav_sim`）—— Face 只依赖自身位姿，与底盘类型无关，且四个执行器**都已支持** `MotionTarget::Face`（task_46 加的） |
| 4 | 地面站派车时**同一帧发两条** `[Goto(环点), Face(人)]`；TASK_SET 是整队替换语义，**两条必须成对下发** |
| 5 | 对准判据：`\|Δθ\| <= DEADBAND_DEG`（8°，复用执行器同一阈值）→ `Success` |
| 6 | **不设时限**：一直转到 `\|Δθ\| ≤ 8°`；目标点与自身重合（无方位可言）视为到位 → `Success`。**不得**抛 `Failure`，也**不得**用固定拍数兜底（2026-11-05 修正：原「60 拍 = 3 s」没有依据，实测 3 s 只够转 86°，180° 转不过去 —— Step 7 的 F3/C1/C2 就是因此 FAIL） |
| 7 | `Face` 不碰 `nav`、不走 D\*、不做避障检查（原地转不位移） |

### Guard（2.8 ~ 2.17）

| # | 项 |
|---|---|
| 8 | `Guard` 与 `Goto`/`Follow`/`Containment`/`Face` 平级；线上 `type = 5`，记录 **17 B** = `type u8 + x f32 + y f32 + fx f32 + fy f32`（**值守点 + 注视点**，朝向 = `atan2(fy−y, fx−x)`） |
| 9 | **只有车端做真树**：`pleiades-ugv` 与 `pleiades-sim/src/sim/ugv_sim`；**UAV 全部 naive**（`pleiades-uav` 与 `sim/uav_sim`：一拍 `Success` + `warn`，与当初 Containment 的做法一致） |
| 10 | 行为三段：**值守**（停在值守点、朝注视点）→ **出动**（人进警戒区 → 复用 Follow 子树朝人开，`d ≤ 1.2 m` 停住并朝人）→ **归位**（人退出警戒区 → 回值守点、转回注视方向） |
| 11 | 警戒区 = 值守点前方**扇形**：半径 **3.0 m**、半角 **±30°**，绑定在**值守点（世界系固定）**，不随车移动 |
| 12 | Guard 的停距固定 **1.2 m**（人类指定），显式传 `d_far = 2.2 / d_near = 1.2`，**不跟 Follow / Containment 的任何常量** |
| 13 | 迟滞：**进 3.0 m / 出 3.5 m**（距离），角度固定 ±30°；不加时间迟滞 |
| 14 | 判定用 detector 已算好的世界点（`Target.x/y`），**不另做检测**；车「看得见但不在警戒区」时**不动**（这是正确行为） |
| 15 | **归位不得冒泡 `Failure`**：守卫是持续任务，归位失败（被堵/超时）只能重试或原地等，**不能**像 `Goto` 那样以 `Failure(Timeout)` 终结任务 |
| 16 | 无 detector（`[detector].enabled = false`）时，Guard **只值守不追**（`warn` 一次），**不得**一拍 `Success`（那会让守卫任务被队列弹走） |
| 17 | Guard 是**持续任务**（永不 `Success`）；只有新的 `TASK_SET`（整队替换）或 `mission_count = 0` 能结束它 |

### 通用（2.18 ~ 2.24）

| # | 项 |
|---|---|
| 18 | `mission::build` 的**穷尽 `match`** 在四个 crate 必须都有分支（漏改即编译失败） |
| 19 | **三种任务用三组距离，互不影响**（人类 2026-11-05 修正）：`Containment` 收紧 `2.0 / 1.0`（围控车靠得更近）、`Follow` 放宽 `2.4 / 1.4`（跟人的车留远一点）、`Guard` 固定 `2.2 / 1.2`；**机栈 `3.6 / 2.6` 一动不动** |
| 20 | 实现方式：`follow.rs` 里两组常量（`D_FAR_M`/`D_NEAR_M` = Follow，`CONTAINMENT_D_FAR_M`/`CONTAINMENT_D_NEAR_M` = Containment），**三处 build 显式传参**；不新增配置键、不加协议字段 |
| 21 | 协议三处长度表必须同口径：Rust `mission_wire_len` / Pictor `_Mission_Wire_Len` / Python 工具 |
| 22 | 旧接收端遇到未知 type（4 / 5）会**整帧拒** → 车 / 机 / 地面站 **同批升级**（与当初加 `type = 3` 同一规矩） |
| 23 | 不改编码侧「未知 type」的既有不对称：Rust `encode_task_set` 跳过该条、Pictor `Encode_Task_Set` 整帧拒 |
| 24 | sim 控制口新增 `{"cmd":"face",...}` 与 `{"cmd":"guard",...}`，与 `goto` 平级（便于不开地面站单测） |

### 已知限制（记录，本任务不修）

- **「边走边朝」做不到**：`0x12` 的 `vy` 硬编码 `0.0`（`ugv/stm32/mod.rs:165`），世界是差速模型。要平移+保朝向 = 固件 `vy` 标定 + Rust 开口 + 执行器双目标 + 世界底盘模型，独立任务。
- **8° 死区**就是「朝向」的精度上界（对 3 m 距离约 0.42 m 横向偏差）。
- **车头 0.33 m 内是盲区**（射线打到地面但被 `MIN_DEPRESSION` 拒答）：人贴到车前 30 cm 会丢目标；Guard 的 3 m 警戒区完全落在可视范围（0.33~7.7 m）内，不受影响。
- **不追旧点**：人丢失（`age > STALE_MS = 800`）时 Follow 停住、Guard 继续等（不主动去找）。

---

## 三、线格式（TASK_SET 变长表）

```text
payload = mission_count u8 | mission[0] | … | mission[count-1]      总长 = 1 + Σlen
type 0 Goto        : type u8 | x  f32 BE | y  f32 BE                         = 9 B
type 2 Follow      : type u8 | x  f32 BE | y  f32 BE                         = 9 B
type 3 Containment : type u8 | cx f32 BE | cy f32 BE | half f32 BE           = 13 B
type 4 Face        : type u8 | fx f32 BE | fy f32 BE                         = 9 B    ← 本任务
type 5 Guard       : type u8 | x  f32 BE | y  f32 BE | fx f32 BE | fy f32 BE = 17 B   ← 本任务
```

长度表三处同口径：`0|2|4 → 9`、`3 → 13`、`5 → 17`，其余 `None/0`（未知 type 整帧拒）。

**金标**（三方单测硬编码同一串）：

| 用例 | 字节 | 长度 |
|---|---|---|
| 单条 Face（64, 64） | `01044280000042800000` | 10 B |
| 单条 Face（1.0, 2.0） | `01043f80000040000000` | 10 B |
| `[Goto(64,64), Face(64,64)]` | `02004280000042800000044280000042800000` | 19 B |
| 单条 Guard（值守点 64,64，注视点 64,128） | `010542800000428000004280000043000000` | 18 B |

---

## 四、涉及文件

| 文件 | 改动 |
|---|---|
| `pleiades-base/src/Robot/core/mission_queue/element.rs` | `Mission` 加 `Face { x, y }`、`Guard { x, y, fx, fy }` |
| `pleiades-base/src/Robot/core/protocol/messages.rs` | `MISSION_FACE = 4`、`MISSION_GUARD = 5`、长度表、encode 两分支（9 B / 17 B）、decode 分派 + 金标单测 |
| `pleiades-base/src/Robot/core/protocol/mod.rs` | re-export `MISSION_FACE` / `MISSION_GUARD`（`MISSION_CONTAINMENT` 现在也没导出，顺手补） |
| `pleiades-ugv/src/ugv/mission/face.rs`（新） | `FaceLeaf`：写 `face_point`、`\|Δθ\| ≤ 8°` → `Success`、**无时限**（目标与自身重合 → `Success`） |
| `pleiades-ugv/src/ugv/mission/guard.rs`（新） | Step 2：`in_zone` 纯函数 + `ZoneGate`（带迟滞）+ 复用 `follow::build` + `HomeLeaf`（无 Timeout）+ 复用 `face` |
| `pleiades-ugv/src/ugv/mission/mod.rs` | 两个新变体的分派（Guard → `guard::build`） |
| `pleiades-sim/src/sim/ugv_sim/mission/{face.rs,guard.rs,mod.rs}` | 与上逐字相同（只差 `use` 与文件头） |
| `pleiades-sim/src/sim/uav_sim/mission/{face.rs,mod.rs}` | Face：机栈同款（无距离参数，可复用）；Guard：**naive** |
| `pleiades-uav/src/uav/mission/face.rs`（新） | 与车端逐字相同（只差 `use` 与文件头） |
| `pleiades-uav/src/uav/mission/mod.rs` | Face → `face::build`；Guard 仍 **naive**（一拍 `Success` + `warn`） |
| `pleiades-ugv/src/ugv/mission/follow.rs` | `D_FAR_M 2.2 → 2.4`、`D_NEAR_M 1.2 → 1.4`（Follow 放宽）；新增 `CONTAINMENT_D_FAR_M = 2.0` / `CONTAINMENT_D_NEAR_M = 1.0`（围控收紧）；注释同步 |
| `pleiades-sim/src/sim/ugv_sim/mission/follow.rs` | 同上（保持逐字一致） |
| `pleiades-ugv/src/ugv/mission/containment.rs` | `follow::build` 前显式传 `CONTAINMENT_*`（带内仍 `stale_ms`） |
| `pleiades-sim/src/sim/ugv_sim/mission/containment.rs` | 同上（逐字一致） |
| `pleiades-ugv/src/ugv/mission/mod.rs` | `Mission::Follow` 分支显式传 `FollowParams{ D_FAR_M, D_NEAR_M, .. }`（不依赖 `default()`） |
| `pleiades-sim/src/sim/ugv_sim/mission/mod.rs` | 同上 |
| `pleiades-sim/src/sim/control.rs` | 加 `face` / `guard` 两条控制口命令（与 `goto` 平级） |
| `Pictor/src/websocket/protocol/protocol_def.gd` | `MISSION_TYPE_FACE := 4`、`MISSION_TYPE_GUARD := 5` + 字符串归一化 |
| `Pictor/src/websocket/protocol/orion_messages.gd` | 长度表 + encode/decode 两个分支 |
| `Pictor/src/websocket/protocol/message_builder.gd` | `build_auto_push_goto_face(x, y, fx, fy)`、`build_guard(x, y, fx, fy)` |
| `Pictor/src/cooperation/coop_controller.gd` | `:218` 改发 `[Goto(环点), Face(人)]` |
| `Pictor/src/control/auto_handler.gd` | `:180` 同步改（口径 #4）；另加 `X → Follow` 的下发处理（Step 4.7） |
| `Pictor/src/control/input_handler.gd` | 新增 `KEY_X` 分支：**先判焦点**，再对选中车发 Follow（Step 4.7） |
| `Pictor/src/event_bus/event_bus.gd` | 新增 `follow_requested(vehicle_id)` 信号（Step 4.7） |
| `Pictor/src/test/test_orion_protocol.gd` | Face / Guard 金标与长度表用例 |
| `Pictor/tool/pictor_ws_ctl.py` | 第三处长度表：常量 + `4: 9` / `5: 17` + `guard` 子命令（**不是 `Tool/`**） |
| `Architecture/robot_arch.md` | §8.12 变长表加 type 4/5；§8.15 车栈距离改 `2.0/1.0`；新增 §8.16「Face / Guard」 |
| `docs/design_doc/orion_protocol.md` | `msgid 5` 记录表加 type 4/5 |

---

## 五、Step

> 约定：每个 Step 按「改哪个文件 → 具体怎么改 → 怎么验 → 会踩什么坑」写。Step 之间是依赖关系（1 → 2 → 3 → 4 → 5 → 6 → 7），**不跳步**；每一步的验收命令必须先绿，再进下一步。

### Step 1 — base：`Face` + `Guard` 两条协议（type=4 / type=5）+ 四处 naive

**依赖**：无。**做完时 `cargo check` 四个 crate 必须全绿**，否则后面全卡住。

**1.1 `pleiades-base/src/Robot/core/mission_queue/element.rs`**

- 在 `Containment` 变体之后追加两个变体（带文档注释）：
  ```rust
  /// 朝向目标点（type=4）：**原地转到面向 (x, y)，不移动**
  Face { x: f32, y: f32 },
  /// 值守（type=5）：停在 (x, y) 朝 (fx, fy)；人进警戒区上前，出区归位
  Guard { x: f32, y: f32, fx: f32, fy: f32 },
  ```
- 注释里写明：`type` 编号只与协议绑定、与枚举顺序无关；`1` 是退役的 Circle，**不得复用**。

**1.2 `pleiades-base/src/Robot/core/protocol/messages.rs`**

- `MissionItem`（`:57`）加两个字段：`pub fx: f32, pub fy: f32`（注释：`仅 type=5 编码，其余恒 0.0`），并加 `impl Default for MissionItem`。
  → **16 处**测试构造点会编译失败，统一改成 `MissionItem { mission_type, x, y, ..Default::default() }`（机械修改，一次改完）。
- 常量：`MISSION_FACE: u8 = 4`、`MISSION_GUARD: u8 = 5`（紧邻 `MISSION_CONTAINMENT`，并保留 `MISSION_FOLLOW` 旁「1 已退役」那句注释）。
- `mission_wire_len`（`:318`）：
  ```rust
  MISSION_GOTO | MISSION_FOLLOW | MISSION_FACE => Some(9),
  MISSION_CONTAINMENT => Some(13),
  MISSION_GUARD => Some(17),
  ```
- `encode_task_set`（`:331`）：容量估算改 `1 + 17 * n`；写 payload 改成按 `len` 分支
  ```rust
  match len { 9 => {x,y} 13 => {x,y,half_side} 17 => {x,y,fx,fy} _ => unreachable!() }
  ```
  「未知 type 跳过该条 + warn」的既有语义**保持不变**（口径 #23）。
- `decode_task_set`（`:363` 起）：循环里按 `mission_wire_len` 得到 `len`，再按 `len` 读字段；17 B 读 `fx`/`fy`。
  保留三条严格校验：`offset == payload.len()`、`count` 与条数一致、未知 type → `None`（整帧拒）。
- 单测：
  - `test_mission_wire_len_table` 补 `4 → 9`、`5 → 17`；
  - 新增 `test_task_set_face_golden`（`01044280000042800000`、`01043f80000040000000`）；
  - 新增 `test_task_set_guard_golden`（`010542800000428000004280000043000000`）；
  - 新增 `test_task_set_goto_then_face`（`02004280000042800000044280000042800000`）；
  - 新增「Guard 记录少 1 B / 多 1 B → 整帧拒」。
- 坑：`encode_task_set` 的截断（`n >= u8::MAX`）与 `Vec::with_capacity` 的估算次数都别改语义，只改上限值。

**1.3 `pleiades-base/src/Robot/core/protocol/mod.rs`**

- `pub use` 补 `MISSION_FACE` / `MISSION_GUARD`（顺带补 `MISSION_CONTAINMENT`，现在它没导出）。

**1.4 四个 crate 的 `mission::build` 各加两个 naive 分支（先让编译过）**

- 文件：`pleiades-ugv/src/ugv/mission/mod.rs`、`pleiades-uav/src/uav/mission/mod.rs`、`pleiades-sim/src/sim/ugv_sim/mission/mod.rs`、`pleiades-sim/src/sim/uav_sim/mission/mod.rs`。
- 写法照现有 `Containment` 的 naive 分支：`tracing::warn!("… 未实现，跳过（平凡树）")` + `Action::new(|f| { f.out.next_cell = None; Status::Success })`。
- 坑：**不要返回 `Err`**（`build` 返回 Err = 清队列 + 停车）。

**1.5 验收**

```bash
cargo check -p pleiades-base -p pleiades-ugv -p pleiades-uav -p pleiades-sim --all-targets
cargo test  -p pleiades-base --lib protocol
```

### Step 2 — 叶子与树：Face 四端真叶子 + Guard 车端真树（UAV 保持 naive）

**依赖**：Step 1 绿。

**2.1 Face 叶子：先写 `pleiades-ugv/src/ugv/mission/face.rs`**

- 结构：
  ```rust
  pub struct FaceLeaf { target: (f32, f32), robot_state: Arc<RwLock<RobotState>>, ticks: u32 }
  // 无时限：口径 #6 已改为「一直转到进死区」
  pub fn build(robot_state: Arc<RwLock<RobotState>>, target: (f32, f32)) -> Box<dyn Node>
  ```
- `tick` 六步：
  1. 读 `rs.attitude.yaw` / `rs.x` / `rs.y`；
  2. `let delta = normalize_angle((fy - y).atan2(fx - x) - yaw)`；
  3. `|delta| <= DEADBAND_DEG`（8°）→ `return Status::Success`；
  4. **无时限**：不进死区就继续 `Running`（不计数、不兜底）；目标点与自身重合（`dx²+dy² < 1e-4`）→ `Success`（无方位可言）；
  5. `f.out.face_point = Some(self.target)`；`f.out.next_cell = None`；`f.out.target_point = None`（**三个字段必须都写**，见坑）；
  6. `Status::Running`。
- `halt`：`self.ticks = 0`（幂等）。
- 私有 `fn normalize_angle(a: f32) -> f32 { (a + PI).rem_euclid(2.0 * PI) - PI }`（与 executor 同式，四份一致）。
- 单测 5 条：`Δθ = 0` 一拍 `Success`；偏 5° → `Running` 且 `face_point` 正确、另两字段为 `None`；偏 90° → `Running`；第 61 拍 → `Success`；`halt` 后再 tick 从 0 计数。
- **复制到另外三份**（只改 `use` 与文件头）：`pleiades-uav/src/uav/mission/face.rs`、`pleiades-sim/src/sim/ugv_sim/mission/face.rs`、`pleiades-sim/src/sim/uav_sim/mission/face.rs`。
  - 四份的唯一 `use` 差异是 `DEADBAND_DEG` 的路径（`crate::ugv::executor::…` / `crate::uav::executor::…` / `crate::sim::ugv_sim::executor::…` / `crate::sim::uav_sim::executor::…`），值都是 `8.0`。
- 四处 `mission/mod.rs` 分派：`Mission::Face { x, y } => Ok(face::build(deps.robot_state.clone(), (*x, *y)))`。
- 坑：
  - `MotionOut::target()` 的优先级是 **face > point > cell**（`frame.rs:42-50`）→ 只写 `face_point` 而不清 `next_cell` / `target_point`，会被上一拍的残留顶掉；
  - 别在叶子里读 `nav`（Face 不碰导航，口径 #7）。

**2.2 Guard：先写 `pleiades-ugv/src/ugv/mission/guard.rs`**

- 常量（全 `pub`，单测直接用）：
  ```rust
  pub const GUARD_R_IN_M: f32 = 3.0;
  pub const GUARD_R_OUT_M: f32 = 3.5;
  pub const GUARD_HALF_ANGLE_DEG: f32 = 30.0;
  pub const GUARD_D_FAR_M: f32 = 2.2;
  pub const GUARD_D_NEAR_M: f32 = 1.2;
  ```
- 纯函数（单测主战场）：
  ```rust
  pub fn in_zone(px: f32, py: f32, station: (f32, f32), yaw: f32,
                 half_angle: f32, r_in: f32, r_out: f32, last_in: bool) -> bool
  ```
  ① `d = hypot(px−sx, py−sy)`；② `da = |normalize(atan2(py−sy, px−sx) − yaw)|`；
  ③ `inside_angle = da <= half_angle`；④ 迟滞：`if last_in { d <= r_out } else { d <= r_in }`；
  ⑤ `inside_angle && 迟滞判据`；NaN 一律 `false`（写 `!(a > b)` 风格）。
- `ZoneGate`（条件节点）：读位姿 → `source.sample(Instant::now(), me)` → 写 `Arc<StdMutex<Option<Sample>>>` 槽位（Follow 子树靠它）→ 用 `in_zone` 更新 `last_in` → `Success` / `Failure(FailCode::Unspecified)`；`halt` 清槽位 + `last_in = false`。
- `HomeLeaf { station: (f32,f32), armed: bool, nav: Arc<Mutex<P>>, robot_state }`：
  1. 距值守点 `< 0.3 m` → `Success`（避免每拍重算 D\*）；
  2. `if !armed || nav.status() == NavStatus::Idle { nav.set_goal(station); armed = true }`；
  3. `nav.step_result(&rs)`：
     - `Arrived` → `Success`；
     - `Blocked(..)` → `warn!` + `next_cell = None` + **`Running`**（口径 #15，绝不 `Failure`）；
     - 其余 → `next_cell = Step(cell)` / `None`，`Running`。
  4. `halt` → `armed = false`。
- `build(nav, robot_state, source, station, gaze)` 组装：
  ```
  ReactiveFallback[
    ReactiveSequence[ ZoneGate, follow::build(robot_state, source, FollowParams{2.2, 1.2}) ],
    RepeatForever[ Sequence[ HomeLeaf, face::build(robot_state, gaze) ] ]
  ]
  ```
  注意 **Guard 必须显式传 `FollowParams`**（口径 #12）：Step 3 会把全局 `D_NEAR` 改成 1.0，而 Guard 要 1.2。
- 单测 8 条：`in_zone` 区外 false / 区内 true / 迟滞（3.2 m 且 `last_in` 仍 true；3.6 m 变 false）/ 角度外 false / NaN false；树级：值守写 `face_point` 且 `target_point=None`；进区写 `target_point`（人的坐标）；归位到站后写 `face_point`；假 nav 恒 `Blocked` → 连续 10 拍仍 `Running`（不 Failure）；`halt` 幂等。
- **复制到 `pleiades-sim/src/sim/ugv_sim/mission/guard.rs`**（只改 `use` 与文件头），并在 `sim/ugv_sim/mission/mod.rs` 分派。
- `pleiades-uav` 与 `sim/uav_sim` 的 `Guard` **保持 Step 1 的 naive**（口径 #9）。
- 坑：
  - `HomeLeaf` **不能复用 `goto::build`**（自带 `Timeout(60 s)` → `Failure` → 守卫任务被弹走）；
  - `ZoneGate` 是「本拍唯一写快照的地方」，`Follow` 子树的 `TargetGate` 也会写自己的槽位 —— 两者**共用同一个 `Arc<TargetSource>`**（`containment.rs:115-116` 的先例），不要各建一份，否则 age 两套、条件与叶子会互相矛盾。

**2.3 验收**

```bash
cargo test -p pleiades-ugv --bins face
cargo test -p pleiades-ugv --bins guard
cargo test -p pleiades-uav --bins face
cargo test -p pleiades-sim --bins face
cargo test -p pleiades-sim --bins guard
```

### Step 3 — 车栈距离参数分家（Containment 收紧 / Follow 放宽 / Guard 不变）

**依赖**：Step 2 绿。

**问题**：`Follow` 与 `Containment` 的跟随分支现在都用 `FollowParams::default()`（同一组常量），改一个就动另一个。人类口径（2026-11-05）是**只让围控车靠得更近，跟人的车反而要拉开一点**，所以必须拆成三组、各自显式传参。

1. **`pleiades-ugv/src/ugv/mission/follow.rs`（`:57,59`）**：
   - `D_FAR_M: 2.2 → 2.4`、`D_NEAR_M: 1.2 → 1.4`（**Follow 放宽 0.2 m**，迟滞带仍 1.0 m）；
   - 新增 `pub const CONTAINMENT_D_FAR_M: f32 = 2.0;`、`pub const CONTAINMENT_D_NEAR_M: f32 = 1.0;`（**围控收紧 0.2 m**，带仍 1.0 m）；
   - 注释里写死的 `1.2` / `2.2`（`:111`、`:125`、`:450`、`:457`、`:473`、`:550` 一带）改成常量名或新值。
2. **`pleiades-sim/src/sim/ugv_sim/mission/follow.rs`**：同上（保持两份逐字一致）。
3. **`pleiades-ugv/src/ugv/mission/containment.rs`**：把 `build()` 里的 `follow::FollowParams::default()` 换成显式
   `FollowParams { d_far: follow::CONTAINMENT_D_FAR_M, d_near: follow::CONTAINMENT_D_NEAR_M, stale_ms: follow::STALE_MS }`（`InsideSquare` 继续用同一份 `stale_ms`）。
4. **`pleiades-sim/src/sim/ugv_sim/mission/containment.rs`**：同上（逐字一致）。
5. **两处 `Mission::Follow` 分支**（`ugv/mission/mod.rs`、`sim/ugv_sim/mission/mod.rs`）：显式传 `FollowParams { d_far: follow::D_FAR_M, d_near: follow::D_NEAR_M, .. }`，**不依赖 `default()`**（防止以后改常量时语义漂移）。
6. **Guard 不动**：仍是显式 `2.2 / 1.2`（口径 #12）。
7. **机端不动**：`pleiades-uav` 与 `sim/uav_sim` 的 `3.6 / 2.6` 一行不改。
8. 单测：`follow.rs` 的用例由常量推导（`MID_BAND`），改值后自动跟随；**新增两条**断言 —— `Containment` 的 `FollowParams` 用的是 `CONTAINMENT_*`（2.0/1.0），`Follow` 用的是 `D_*`（2.4/1.4）。
9. 验收：

```bash
cargo test -p pleiades-ugv --bins follow
cargo test -p pleiades-ugv --bins containment
cargo test -p pleiades-sim --bins follow
grep -n 'D_FAR_M\|D_NEAR_M\|CONTAINMENT_D_' pleiades-ugv/src/ugv/mission/follow.rs pleiades-sim/src/sim/ugv_sim/mission/follow.rs
# 车端 Follow 2.4/1.4 + Containment 2.0/1.0；机端两份仍 3.6/2.6
```

### Step 4 — Pictor：类型 / 长度表 / 编解码 / 下发

**依赖**：Step 1 的 Rust 金标已定（两端必须同串）。

1. **`Pictor/src/websocket/protocol/protocol_def.gd`**（`:68-70` 一带）
   - 加 `const MISSION_TYPE_FACE := 4`、`const MISSION_TYPE_GUARD := 5`；
   - `Mission_Type_From()`（`:79-83` 一带）加 `"face"` → 4、`"guard"` → 5；未知字符串仍 → 0（那是 LLM 防御路径）。
2. **`Pictor/src/websocket/protocol/orion_messages.gd`**
   - `_Mission_Wire_Len`（`:263` 一带）加 `4 → 9`、`5 → 17`；
   - `Encode_Task_Set`（`:263-290`）的 `printerr` 文案从 `{0,2,3}` 改成 `{0,2,3,4,5}`；写 payload 按 `ln` 分支（9 → x,y；13 → x,y,half_side；17 → x,y,fx,fy）；
   - `Decode_Task_Set` 同步两个分支。
3. **`Pictor/src/websocket/protocol/message_builder.gd`**
   - 新增 `build_auto_push_goto_face(x, y, fx, fy)` → `{"missions": [{"type":0,...},{"type":4,"x":fx,"y":fy}]}`；
   - 新增 `build_guard(x, y, fx, fy)` → `{"missions": [{"type":5,"x":..,"y":..,"fx":..,"fy":..}]}`。
4. **下发点改两条**：`src/cooperation/coop_controller.gd:218` 用 `build_auto_push_goto_face(p2.x, p2.y, center.x, center.y)`；`src/control/auto_handler.gd:180` 同样把"圆心/目标点"作为 `(fx,fy)`。
5. **Guard 下发入口**：`src/kernel/ws_command_server.gd` 的 `_Handle_Task_Set` 里有一处 **type 门控 `∉ {0,2,3} → ok:false`**，必须加 4/5，否则新命令会被 ws 通道直接拒；再补一个 `guard` 子命令（`{cmd:"guard", target, x, y, fx, fy}`）。
6. **单测**：`Pictor/src/test/test_orion_protocol.gd` 加两条金标（与 Rust 同一串）与一条往返；`test_coop_controller.gd` 若断言了下发帧的 missions 条数/类型，同步改成两条。
7. **新增键位入口：X → Follow**（人类 2026-11-05 追加；协议/车端本来就支持 `type=2`，本项只做 UI）
   - `src/control/input_handler.gd`：加 `KEY_X` 分支，**入口先判焦点**（`get_viewport().gui_get_focus_owner() != null → return`），否则在聊天框里打 "x" 也会触发（顺带修掉本文件既有的「输入框打字触发运动」缺陷）；
   - 触发条件：`selected_ids` 非空、`manual_target` 为空（手动优先）、`app_state.mode` **不是** `Mode.FOLLOW`（那个枚举是「相机跟车」，见 `camera_2d.gd:62-64`，**不得复用**）；
   - 动作：新增 `EventBus.follow_requested(vehicle_id)`（或在 `auto_handler.gd` 加处理函数）→ **先发 `msgid 8`（auto）**，再发 `build_task_set([{"type":2,"x":0.0,"y":0.0}])` 给**选中的第一台**；
   - 退出：再按一次 X → 发**空 missions**（`mission_count = 0` 是唯一的取消入口）；`Follow` 是持续任务，不取消不会自己结束；
   - 提示：目标车/机没开 detector 时 Follow 会原地 `Hold`（车端行为），UI 打一条日志说明，避免误判"按键没生效"。
8. 验收：

```bash
godot --headless --path Pictor -s src/test/test_orion_protocol.gd   # === ALL PASS: true ===
```

### Step 5 — sim 控制口（face / guard）

**依赖**：Step 2（叶子已在），Step 4（Pictor 可下发，但本步不依赖它）。

1. **`pleiades-sim/src/sim/control.rs`**
   - `ControlCmd` 加两个变体：`Face { x: f32, y: f32 }`、`Guard { x: f32, y: f32, fx: f32, fy: f32 }`；
   - `Parse_Control`（`:236-285`）加两个分支：坐标必须有限；`guard` 的 `(fx,fy)` 与 `(x,y)` 重合 → 直接 `Err`（`atan2(0,0)` 无定义）；
   - 注入路径：构造 `Command::Auto(AutoCmd::Set(vec![Mission::Face{..}]))` / `Guard{..}`，走现有 `robot_cmd_tx`（1 s 超时），与 `goto` 同一条路；
   - `Help_Data()`（`:432` 起）加两条；`help` 数组顺序保持「读 → 写」一致。
2. **同步两处"指令条数"断言**（本步唯一会碰的存量测试，见 §九 附录）：
   - `:1198` `assert_eq!(arr.len(), 11, …)` → **13**；
   - `:1204` 的名字列表补 `"face"`、`"guard"`；
   - `:865` 那句错误文案 `goto/circle/…/help` 补两个名字。
3. 单测：`test_parse_valid_face`、`test_parse_valid_guard`、`test_guard_rejects_coincident_gaze`、`test_help_data_has_all_commands`（13）。
4. 验收：

```bash
cargo test -p pleiades-sim --bins control
```

### Step 6 — Python 工具 + 文档

**依赖**：Step 4 的 GDScript 长度表已定（三处要同口径）。

1. **`Pictor/tool/pictor_ws_ctl.py`**（这是第三处长度表，别漏）
   - 常量区（`:72-75`）加 `MISSION_TYPE_FACE = 4`、`MISSION_TYPE_GUARD = 5`，并把 `SUPPORTED_MISSION_TYPES` 扩成 5 元组；
   - `_Mission_Wire_Len` 表（`:309-311`）加 `4: 9`、`5: 17`；
   - `_normalize_mission_type`（`:315` 起）加 `"face"` / `"guard"`；
   - 加 `guard` 子命令（参数 `--target --x --y --fx --fy`），并保留 `--type 9` 探针仍报错、不发帧。
2. **`RobotSim_TestEnv/pictor_ws/tool/pictor_ws_ctl.py`** 是同一文件的副本：同步改，或在该文件头注一句「以 `Pictor/tool/` 为准」。
3. **文档**：
   - `Architecture/robot_arch.md`：§8.12 变长表加 `type 4` / `type 5`；§8.15 的 `D_FAR/D_NEAR` 表改成车端 `2.0/1.0`；新增 §8.16「Face / Guard」（树形、常量、迟滞、`HomeLeaf` 不 Failure、Face 四端）；
   - `docs/design_doc/orion_protocol.md`：`msgid 5` 的记录表加 type 4/5 与 9/17 B。
4. 验收：三处长度表对拍

```bash
grep -n 'MISSION_FACE\|MISSION_GUARD\|4 => 9\|5 => 17\|4: 9\|5: 17' \
  pleiades-base/src/Robot/core/protocol/messages.rs \
  Pictor/src/websocket/protocol/orion_messages.gd \
  Pictor/tool/pictor_ws_ctl.py
```

### Step 7 — 模拟世界验证（Face / Guard / Containment×Face）

**依赖**：Step 1~6 全绿。

**目标**：用**最新代码**在仿真世界里跑一次**现象级**验证：`Face`、`Guard`，以及 `Face` 的**真实用途** —— 封控发现人 → 地面站派车 → 车到位后朝人（`[Goto, Face]` 两条任务）。`Follow` / `Containment` 自身的行为已在 task_46 Step 4 验过，本步不重复；但 **containment → assist → 派车 → Face** 这条链必须验，因为 Face 就是为它准备的。不设数值门槛（沿用 task_46 Step 4 / task_45 Step 3 口径 #10）。

#### 7.1 编译（CUDA，坑见 task_46 §4.1）

- `pleiades-sim` 对 base 是 `default-features = false`，`cargo build -p pleiades-sim --release` 会**静默编出 CPU 版**；要么全工作区 `cargo build --release`，要么按 task_46 给 sim 加 2 行 feature 转发。
- 编译后核对：`cargo tree -p pleiades-sim -e features -i pleiades-base` 必须显示 `cuda`；否则 CPU 版 detector 单帧 100~400 ms，tick 会被拖垮、现象不可信。
- nvcc 需 gcc-12（`build.sh` 的 PATH 手法）。

#### 7.2 环境

| 角色 | 启动 | 端口 |
|---|---|---|
| 世界 | `orion-simulation-world` | 9999 |
| 地面站 | `godot --headless --path Pictor`（**C 系列必需**；F/G 用控制口注入时不需要） | 9100 |
| sim 车 | `pleiades-sim`，`chassis = diff_drive_ugv`；G 系列要 `[detector].enabled = true`，C 系列可关（Face 不用 detector） | 9098（控制口） |
| sim 机 | `pleiades-sim`，`chassis = quadrotor_uav`，`device = "cuda"`；**C 系列必须**开 `[detector].enabled = true`（人是它发现的），F4 只验 Face、detector 可关 | 9097（控制口） |
| 行人 | `ped_console.py --peer-id human-01` | 9101 |
| 任务注入 | **优先** Step 5 的 sim 控制口（`face` / `guard`）；C 系列用 Pictor `pictor_ws_ctl.py` 的 `containment`，`guard` 用 Step 4/6 的子命令 | — |

> ⚠️ **Pictor 的 `.config/config.toml` 必须含** `[Network].subscribe_topics = ["pleiades/robot/assist_request"]`。仓库根那份**没有**这一行（`bootstrap.rs` 对缺省取空列表），缺了 C 系列会**静默不触发**（机发了 assist、地面站根本没订阅）；`RobotSim_TestEnv/pictor_ws/.config/config.toml:18` 有可抄的样例。
>
> Guard 必须有 detector（口径 #16：无 detector 只值守不追），所以 G 系列 sim 车的 `[detector].enabled` 一定要开。

#### 7.3 Case（Face 4 个 + Guard 4 个 + Containment×Face 2 个）

| # | 栈 | 注入 | 看什么 |
|---|---|---|---|
| **F1** | sim ugv | `[goto(A), face(B)]` 两条任务 | 到 A 后停住 → 第二段**原地转**（位置不变、`yaw` 变化）→ 进入 8° 死区后输出 `(0,0)` → 出现两次「任务完成」 |
| **F2** | sim ugv | 只 `goto(A)`（对照） | 到点停住、**机头保持进场方向**（不转）——证明 Face 与 Goto 可分离（口径 #4 的反面） |
| **F3** | sim ugv | `face(车身后侧的点)` | 原地转约 180°，全程 `Δpos < 0.05 m`（不位移） |
| **F4** | sim 机 | `[goto(A), face(B)]` 两条任务 | 机到点后**原地转向**（`yaw` 变、位置不变）→ 进 8° 死区后停；验 sim 机栈的 Face 叶子（真机 UAV 与它同码，无硬件不验） |
| **G1** | sim ugv | `guard(值守点, 注视点)` | 到值守点 → 停住并朝向注视点；**行人在 5 m 外走动时车不动**（看得见但不进区） |
| **G2** | sim ugv | 同上，人走进 3 m 扇形 | 车朝人前进 → 到 ~1.2 m 停住并朝人；日志出现「进区」 |
| **G3** | sim ugv | 同上，人退到 3.6 m 外 | 车回值守点 → 转回注视方向；日志出现「出区」 |
| **G4** | sim ugv | 同上，人在 3.0~3.5 m 之间来回走 | 车**不反复**出动/归位（只在跨 3.5 m 时才归）——迟滞成立（口径 #13） |
| **C1** | sim 机 + sim 车（+ 地面站） | 机发 `containment(中心, 半边长)`，人走进方框 | 机 `[Coop] 协助请求 …` → 地面站 `[Coop] 指派 车=…` → 车 `[Nav] 新目标` → 到位 → **车头转向人**（`v = 0`、`yaw` 指向人、误差 ≤ 8°）→ 两次「任务完成」（Goto + Face） |
| **C2** | 同上 | 人移动 > 1 m（`ASSIST_MOVE_M`） | 机重发 assist → 地面站重派 → 车重新到位并**重新朝人**（验证「整队替换」下 Face 仍然生效） |

可选 **G5**（时间够再做）：值守点设在障碍另一侧，`HomeLeaf` 被堵 → 只有 `warn`，**任务不终结**（仍在 `Running`），验证口径 #15。
可选 **C3**（时间够再做）：第二台 sim 车同时在线 → 两车**各收一帧**（一车一帧）、各自到位后都朝同一个人（`(fx, fy)` 相同）。

#### 7.4 判据（现象级，不设数值门槛）

- **PASS** = 现象出现：Face 转到位后自停；Guard 值守不动 / 出动到 ~1.2 m / 归位 / 迟滞不回跳；C 系列车到位后朝人、人移动后重派并重新朝人。
- **FAIL** = 车完全不动、任务不进队、日志报错/panic、连接掉线、**Guard 任务被弹走**（说明误用了带 `Timeout` 的 `goto`）、或车在警戒区外乱动。
- **C 系列专有 FAIL**：车到位但**不转向**（说明 Face 没下发，或地面站还是旧版只发 Goto）；机有 `[Coop] 协助请求` 而地面站无 `[Coop] 指派`（先查 Pictor 的 `[Network].subscribe_topics` 是否含 `pleiades/robot/assist_request`）。
- 不判到位误差、摆幅、耗时、tick 频率、`omega` 具体大小；数值只记录，不参与 PASS/FAIL。
- 不做基线对照（本步只看"现象是否成立"）。

#### 7.5 记录与收尾

- 每个 case 记一段日志摘录：`[Sim] UGV 位置=…`、`[Nav] 新目标 …`、`[Robot] 开始任务 …`、`[detector] → 世界=(x,y)`、`[Guard] 进区/出区 r=…`（建议在 `ZoneGate` 打这一行，便于事后核对判据）、C 系列另加 `[Coop] 协助请求 …` / `[Coop] 指派 …` + 一句话结论。
- 收尾：关世界 / Pictor / sim / `ped_console`，`ss -ltnp` 确认 9999、9098、9100、9101 无残留进程。
- 不接受任何「断言 N 项」式自检作为本步判据。

#### 7.6 执行顺序（一次跑完，避免反复起停）

1. 起世界（9999）→ 2. 起 sim 车（9098）→ 3. 跑 **F1 / F2 / F3**（不需要行人与地面站）→ 4. 起 sim 机（9097）→ 跑 **F4** →
5. 起 `ped_console`（9101）→ 跑 **G1 / G2 / G3 / G4**（车 + 行人）→ 6. 起 Pictor（9100）+ 让机接 `containment` → 跑 **C1 / C2**。
每个 case 之间用控制口 `stop`（或 `mode` 切手动再切回）把车停稳，避免上一个 case 的余速干扰下一个现象。


### Step 8 — 去掉 Face 的 60 拍时限（四份）

**依赖**：Step 7 的验证结果（F3/C1/C2 FAIL，根因就是它）。

**依据（可复算，不是拍的）**：车端 `cruise_yaw_rate = 0.5 rad/s = 28.6°/s` ⇒ 转 180° 需要 **6.3 s**；原 `FACE_GIVE_UP_TICKS = 60`（3 s）最多只转 **86°**。人类口径（2026-11-05）：**不设时限，转得久一点无所谓，只要能看到它能不能转过去**。

1. **四份 `face.rs`**（`ugv` / `uav` / `sim/ugv_sim` / `sim/uav_sim`，逐字一致）：
   - 删除 `FACE_GIVE_UP_TICKS` 常量与 `FaceLeaf::ticks` 字段；
   - `new()` 去掉 `ticks: 0`；`halt()` 改为空实现（无状态、幂等、仍不发 Stop）；
   - `tick()` 删掉「计数 + 到点 Success」分支；保留「`|Δθ| ≤ 8°` → `Success`」与「只转不走 + 清另两字段」；
   - 新增退化保护：`let (dx, dy) = (fx - x, fy - y); if dx*dx + dy*dy < 1e-4 { return Status::Success }`（目标点与自身重合时 `atan2(0,0)` 无意义，否则会无限原地转）；
   - 文件头注释里的「④ ticks ≥ 60 也 Success」同步改掉。
2. **单测**：删 `test_face_gives_up_after_60_ticks_success` 与 `test_face_halt_resets_ticks`；新增
   - `test_face_keeps_running_without_alignment`（偏 90°、位姿不变 → 连 300 拍都 `Running`，不 Success、不 Failure）；
   - `test_face_point_at_self_is_success`（目标 = 自身位置 → 一拍 `Success`）；
   - `test_face_halt_is_idempotent`（`halt` 两次不 panic，之后仍 `Running`）。
3. 验收：`cargo test -p pleiades-ugv --bins face`、`-p pleiades-uav --bins face`、`-p pleiades-sim --bins face`。

### Step 9 — 重跑仿真验证（F3 / C1 / C2）

**依赖**：Step 8 绿。

**目标**：只回答一个问题 —— **现在到底转不转得动 180°**。复用 Step 7 的驱动（`tmp/task48_tools/s7_evidence/s7_driver.py` 与 `tmp/task48/s7/env/`），至少重跑 **F3 / C1 / C2**；其余 7 条上一轮已 PASS，可复跑但不作判据。

- 判据（只判现象，不设时限）：F3 末态对准身后点（误差 ≤ 8°）；C1/C2 到位后车头朝人（误差 ≤ 8°）。
- 必须记录：F3 实际转过的角度与耗时（秒）——用来说明「转 180° 用了多久」。
- 证据落 `tmp/task48/s9/`；引擎结束会把它复制到 `tmp/task48_tools/s9_evidence/`。
- 收尾同 Step 7：kill 全部进程 + `ss -ltnp` 确认端口清空。

---

## 六、边界（本任务明确不做）

- 不做绝对航向（只有「朝点」）；不做「边走边朝」/侧移。
- 不改 `Goto` 的记录、树、`Timeout`；不改 `Follow` 的树形。
- 不改 UAV 封控树；UAV 的 **Guard** 仍是 naive（**Face 四端都真实现**，见口径 #3）。
- 不动 Pictor 的 `RING_RADIUS_M = 3.0`（派车环 ≠ 停距 ≠ 警戒区半径）。
- Guard 不做：多车值守编排、扫视（左右摆头找目标）、目标轨迹跟踪、警戒区形状自定义（固定扇形）。
- **不做警戒区可视化**（人类明确不要）：世界侧不加 `guard_zone` 消息、不改 `WireProtocol.gd`、Pictor 也不加叠加层；演示时"警戒区"只靠车的动作本身与日志解释。
- 不给 Face / Guard 加避障检查（原地转不位移；Guard 的归位走 D\* 已有避障）。
- 不修「编码侧未知 type 不对称」（口径 #23）。

---

## 七、风险

| 风险 | 说明 / 处置 |
|---|---|
| **旧端整帧拒** | 未升级的接收端遇到 type 4/5 → 整帧拒、队列不动（不会误动作，但下发失效）。同批升级 |
| **漏发第二条 Face** | TASK_SET 整队替换；只发 Goto 就「走而不朝」。口径 #4 + 验收专门验一次 |
| **`MotionOut` 优先级** | Face 叶子必须清 `next_cell` / `target_point`；`ZoneGate` 也要清上一拍残留 |
| **Guard 归位 `Failure`** | 若误用 `goto::build`，被堵 60 s 会把整个守卫任务终结。必须用无 Timeout 的 `HomeLeaf`（口径 #15） |
| **缺迟滞会抽** | 人在 3 m 边界半步来回 → 车反复出动/归位。口径 #13 |
| **警戒区随车移动会自激** | 必须绑值守点（世界系），口径 #11 |
| **detector 未启用** | Guard 与 Follow 一样依赖 `[detector].enabled = true`；未启用时 Guard 退化为「只值守不追」（warn 一次） |
| **8° 精度** | 对 3 m 距离 ≈ 0.42 m 横向偏差；扇形边界在演示中会看起来略偏 |
| **无警戒区可视化** | 演示时观众看不到 3 m 边界，「人进/出区」只能靠车的出动/归位与日志解释；本任务按人类要求不做（边界一节已写明） |
| **任务数变多** | Face / Guard 各占一个 `TaskId`；日志解析脚本若按条数统计会多一条 |
| **距离参数分离** | 三处 build 必须各自显式传参；漏传任一处就退回 `FollowParams::default()`（= Follow 的 2.4/1.4）→ Containment「不够近」、Guard「太近」 |

---

## 八、验收总表

| 检查项 | 命令 / 证据 | Step |
|---|---|---|
| 协议金标（Face / Guard） | `cargo test -p pleiades-base --lib protocol` | 1 |
| 四个 crate 编译 | `cargo check -p {pleiades-base,pleiades-ugv,pleiades-uav,pleiades-sim} --all-targets` | 1 |
| Face 叶子行为（四端） | `cargo test -p pleiades-ugv --bins face`、`cargo test -p pleiades-uav --bins face`、`cargo test -p pleiades-sim --bins face` | 2 |
| 三组距离参数 | 车端 `Follow 2.4/1.4`、`Containment 2.0/1.0`、`Guard 2.2/1.2`、机端 `3.6/2.6` 不变（`grep` 对拍 + follow/containment 单测） | 3 |
| Guard 树行为（含迟滞/不 Failure） | `cargo test -p pleiades-ugv --bins guard`、`cargo test -p pleiades-sim --bins guard` | 2 |
| Pictor 金标同串 | `godot --headless --path Pictor -s src/test/test_orion_protocol.gd` | 4 |
| sim 控制口 `face` / `guard`（含 help 11→13） | `cargo test -p pleiades-sim --bins control`；注入后出现对应建树与动作 | 5 |
| Pictor `X → Follow` | 按 X → 车切自动并收到 Follow（`[Robot] 开始任务` + 车朝人）；再按 X → 空 missions 取消 | 4 |
| sim 验证 Face（F1~F4） | 日志：第二段 `v=0`、`yaw` 变、位置不变、两段「任务完成」；只发 goto 时不转 | 7 |
| sim 验证 Guard（G1~G4） | 日志：值守不动 / 出动到 ~1.2 m / 归位 / 迟滞不回跳 | 7 |
| sim 验证 Containment×Face（C1/C2） | `[Coop] 协助请求 → 指派`、车到位后 `v=0` 且 `yaw` 指向人；人移动 > 1 m 后重派并重新朝人 | 7 |
| 文档同步 | `robot_arch.md` §8.12 / §8.15 / §8.16、`orion_protocol.md` | 6 |

---

## 九、待人类确认

1. **三组距离的数值**：本文件按 `Follow 2.4/1.4`（放宽 0.2）、`Containment 2.0/1.0`（收紧 0.2）、`Guard 2.2/1.2`（不变）写，三组迟滞带都保持 1.0 m。要改幅度（例如围控再近到 1.8/0.8）只动常量即可，但**必须保持「围控 < Guard < Follow」这个次序**。
2. **`auto_handler.gd:180`（自动分配）是否也带 Face**：本文件按「带」写（口径 #4）；若不需要，那里保持只发 Goto，但文档要写明「该入口不朝」。
3. **Guard 的三个几何参数**（3.0 / ±30° / 3.5）先做常量；要不要直接升成 `[guard]` 配置段（每车可调）？
4. **Pictor 的 X → Follow 交互**（Step 4.7）：发给 `selected_ids` 全部还是只第一台？退出用「再按一次 X 发空 missions」还是 `Esc`？按 X 时是否顺带发 `msgid 8` 切自动（不发的话车端 `Auto` 分支不生效）？

### 附：「指令条数」相关测试的现状（已核）

- **协议与队列都不限制我们的动态条数**：`TASK_SET` 的 `mission_count` 是 `u8`，硬上限 255；超了只截断 + `warn`（`messages.rs:340`），`test_task_set_max_count` 锁的是「255 条可编解码、256 条截断」这个**上限行为**，不是某个固定条数。
- 任务队列 `MissionQueue` **没有任何容量上限**（只有 `TaskId` 的 `u64` 回绕注释）。
- 会被我们改到的只有 **sim 控制口**的两处：`control.rs:1198` 的 `assert_eq!(arr.len(), 11)` 与 `:865` 的命令名文案 —— Step 5 已写明改成 13 并补名字。
- TUI 的 `available_commands` 是运行时动态填充，**没有条数断言**；Pictor 侧也没有。
