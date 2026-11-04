# task_46_robot_simulation_refine — Robot 仿真完善

> Presented by KeJi
> Created Date ： 2026-11-03
> Modified Date ： 2026-11-03
> 状态：Step 1 / Step 2 / Step 3 / Step 4 待人类确认（未开始）。

---

## 一、目标

真机上 `Follow` / `Containment` 已能正常运行（task_45 收尾），本任务修两个「小毛病」：

1. **问题一（Step 1）**：`Follow` 只在「前进」时朝人，**停下来就不转机头** → 人很容易走出视场 → 丢目标 → 永久 `Hold`。
2. **问题二（Step 2）**：仿真里的行人每个操作都要重新生成一个，无法对已存在的人「中途让他走一下」。

### 1.1 问题一的病根

`Follow` 叶子的输出分支（四份 `mission/follow.rs` 同构，以真车为例）：

```rust
// pleiades-ugv/src/ugv/mission/follow.rs:293-303
match snap {
    Some(s) if out == Outcome::Advance => {
        f.out.target_point = Some((s.x, s.y));   // ← 只有前进时才给目标点
        f.out.next_cell = None;
    }
    _ => {
        f.out.next_cell = None;
        f.out.target_point = None;               // ← 其余情形什么都不给
    }
}
```

而树形状是（`follow.rs:215-228`）：

```
ReactiveFallback
├─ ReactiveSequence [ TargetGate, Condition(!lost && !too_close), StepLeaf ]
└─ HoldLeaf            // 无参构造，两个字段都写 None（follow.rs:324-332）
```

于是「有目标但不该前进」有两条路，**两条都不发朝向**：

| 情形 | 判据 | 走哪条 | 实际输出 |
|---|---|---|---|
| 太近（`d <= D_NEAR`） | `Condition` 为假 | `HoldLeaf` | 两个 `None` |
| 带内停（`D_NEAR < d <= D_FAR` 且上拍 `Hold`） | `decide() == Hold` | `StepLeaf` 的 `_` 分支 | 两个 `None` |
| 丢失 / 过期（`age > STALE_MS`） | `TargetGate` 失败 | `HoldLeaf` | 两个 `None` |

`target = None` → 执行器 `decide(None, …)` 直接返回 `(0, 0)`（`executor.rs:103-106`）→ **既不走也不转**，机头冻结在当时的角度。

### 1.2 失效链（为什么「人很容易就走出去」）

1. 机栈 `D_NEAR = 2.6 m`（`pleiades-uav/src/uav/mission/follow.rs:64`），一进 2.6 m 内就永久 `Hold`，机头不动；
2. 机栈相机垂直视场 70°、下压 15°、巡航高 2.5 m ⇒ 画面下边缘只够到地面 `2.5 / tan(50°) ≈ 2.10 m`（task_45 口径 4 补充）——人再近就掉出画面；
3. 水平方向：2.6 m 处视场半宽约 3.2 m，人横移 3 m 左右出框；
4. 出框 ⇒ `dets = 0` ⇒ `age > STALE_MS(800 ms)` ⇒ `lost` ⇒ `HoldLeaf` 也发 `None` ⇒ **机头永远不会再转头去搜人**，只能等人自己走回那个固定视锥。

即：`robot_arch.md §8.15` 把它记作「人绕到车正后方才丢」，实际**停住后人往侧面走就会触发**。

### 1.3 为什么不能在 `Hold` 分支直接写 `target_point`

执行器拿到点后，只要 `|Δθ| <= DEADBAND_DEG(8°)` 就 `v = cruise_speed` 前进（`executor.rs:114-117`）——直接写点等于**朝人撞过去**。所以缺的不是目标，而是「朝向」这个意图本身。

### 1.4 做法：`MotionTarget` 加第三变体（沿用 task_45 的手法）

task_45 Step 1（commit `74382c3`）已经把执行器的入口从 `Option<(i32,i32)>` 改成 `Option<MotionTarget>`，并规定「**执行器只加类型，不改算法**」：进门 `match` 消解类型，其后的 `d` / `Δθ` / 五档 / `ω` 符号链一行不动。本次是同一动作的第二次——把「只朝哪」也变成目标的一种。

```rust
// base behavior_tree/frame.rs
pub enum MotionTarget {
    Cell((i32, i32)),   // 网格坐标（Goto / 巡逻），执行器取格心
    Point((f32, f32)),  // 世界坐标（Follow 前进），原值直接用
    Face((f32, f32)),   // 世界坐标（Follow 停住），只转不走   ← 新增
}
```

`MotionOut::target()`（`frame.rs:36-41`，全仓唯一的合并点，四个 `on_tick` 只认它）优先级改为 **`face_point` > `target_point` > `next_cell`**。

### 1.5 做完之后

1. **停住时也朝人**：`Follow` 只要还有未过期目标，机头就对准人；人走一圈，机原地转一圈。
2. **执行器仍只差文件头**：五档顺序、`8°`/`12°`/`0.5 m` 三阈值、`ω` 符号链零改动；`Face` 只是「`v` 强制 0」的修饰。
3. **四份 Follow 树仍同形**：除距离常数与文件头外只允许差 `use`。
4. **`Goto` / 巡逻 / `Containment` 巡逻分支不受影响**：仍走格 + D\*，`nav/` 一行不改。
5. **丢失时仍不乱转**：无目标 / 过期 ⇒ 三个字段都 `None`，行为与今天一致。

### 1.6 已定口径

| # | 项 |
|---|---|
| 1 | `MotionTarget` 加 `Face((f32,f32))`；`MotionOut` 加 `face_point`；`target()` 优先级 **face > point > cell** |
| 2 | `Face` 语义 = **只转不走**：`v` 恒 0，`ω` 仍走同一套 `Δθ` 与 `DEADBAND_DEG` |
| 3 | `Face` 下**跳过「身后直行」档**（`d < PASS_THROUGH_M && \|Δθ\| > 90°`）——那一档的目的就是穿过身后的格心，`Face` 永不前进；不跳过会出现「本该原地转却 0.45 m/s 直行」 |
| 4 | `Face` 下 `\|Δθ\| <= DEADBAND_DEG` ⇒ `(0, 0)`（不抖）；`> DEADBAND_DEG` ⇒ `(0, ±cruise_yaw_rate)`。原本的弧线档（8°~12°）因 `v = 0` 自然退化为原地转，五档表无需改写 |
| 5 | 写 `face_point` 的位置只有两处：`StepLeaf` 的「带内停」分支、`HoldLeaf` 的「太近」分支；丢失 / 过期两条仍写 `None` |
| 6 | `HoldLeaf` 因此必须拿到目标快照：`HoldLeaf::new(slot, params)`（现在是 `HoldLeaf::new()`，拿不到快照就没法写朝向） |
| 7 | 树形状**不变**（`Condition(!lost && !too_close)` 保留），只在叶子内部增加「不前进时给朝向」 |
| 8 | 范围：**四份统一**（真车 / 真机 / sim 车 / sim 机）。理由见 §1.7 |
| 9 | 不改协议、不改急停、不改 `0x12`、不改 Pictor、不新增阈值、不改 `nav/`、不改 `detector/` |
| 10 | 单测不作验收判据；用例与口径冲突时改用例，禁止为迁就用例改生产代码（沿用 task_45 口径 #10） |

### 1.7 为什么四份必须一起改

- **执行器四份是被编译器逼的**：`MotionTarget` 是共享枚举，`match` 非穷尽就编译不过，四份执行器都得加 `Face` 分支；
- **树四份是 task_45 口径 #3 逼的**：`ugv ≡ sim_ugv`、`uav ≡ sim_uav` 只允许差文件头（车/机之间再差距离常数）。只改机栈两份，车与机的 `follow.rs` 就会出现「一个发 `face_point`、一个不发」的实质差异，这条一致性约束当场失效。

### 1.8 已知限制（记录，本任务不修）

- 跟踪角速度上限 `cruise_yaw_rate = 0.5 rad/s ≈ 28.6 °/s`。机栈 2.6 m 处，行人以 `1.26 m/s`（世界 `Pedestrian` 的 `DEFAULT_MAX_SPEED`）横移的切向角速度约 `0.48 rad/s` —— **几乎没有余量**，人一快走仍会掉出视场。届时该调的是 `cruise_yaw_rate` 或加「按目标角速度转向」，不是这条逻辑。
- 车栈 `D_NEAR = 1.2 m` 下车同样会「停住不转头」，本次一并修；但车是零转向半径差速底盘，原地转时相机扫视的角速度与机相同（`0.5 rad/s`），近距离行人更易出框。

---

## 二、Step

### Step 1 — `MotionTarget` 加 `Face`，Follow 停住时朝人

**小目标**：`Follow` 在「有目标但不前进」的每一拍把人的坐标作为**朝向目标**交给执行器；执行器只转不走；四份执行器与四份树保持一致。

**涉及文件**

| 层 | 文件 |
|---|---|
| base | `pleiades-base/src/Robot/core/behavior_tree/frame.rs`（+ `mod.rs` 若导出变化） |
| 执行器 ×4 | `pleiades-ugv/src/ugv/executor.rs`、`pleiades-sim/src/sim/ugv_sim/executor.rs`、`pleiades-sim/src/sim/uav_sim/executor.rs`、`pleiades-uav/src/uav/executor.rs` |
| Follow 树 ×4 | `pleiades-ugv/src/ugv/mission/follow.rs`、`pleiades-sim/src/sim/ugv_sim/mission/follow.rs`、`pleiades-sim/src/sim/uav_sim/mission/follow.rs`、`pleiades-uav/src/uav/mission/follow.rs` |
| 不动 | `mission/mod.rs`（`build()` 签名不变）、`containment.rs`（复用 `follow::build`，自动继承）、`nav/`、`detector/`、`robot_handler.rs`（调用点恒为 `decide(out.target(), …)`）、`config.rs`、Pictor |

**实施步骤**

1. **base `frame.rs`**
   - `MotionTarget` 加 `Face((f32, f32))`。
   - `MotionOut` 加 `pub face_point: Option<(f32, f32)>`（`Default` 自动为 `None`）。
   - `MotionOut::clear()` 清**三个**字段。
   - `MotionOut::target()`：

     ```rust
     pub fn target(&self) -> Option<MotionTarget> {
         if let Some((px, py)) = self.face_point { return Some(MotionTarget::Face((px, py))); }
         if let Some((px, py)) = self.target_point { return Some(MotionTarget::Point((px, py))); }
         self.next_cell.map(MotionTarget::Cell)
     }
     ```
   - 单测：三种单字段、三个字段同时有时的优先级（face 最高）、`clear()` 三清。

2. **执行器 ×4**（**只改进门消解，不改五档**）
   - 进门 `match` 解出 `(tx, ty, rotate_only)`：`Cell` / `Point` → `false`，`Face` → `true`。
   - `rotate_only` 为真时：
     - **先跳过身后的直行档**（口径 #3）；
     - `|Δθ| <= DEADBAND_DEG` → `VelocityCmd { v: 0.0, omega: 0.0 }`；
     - 否则 → `VelocityCmd { v: 0.0, omega: 按原符号链 }`（`Δθ > 0 ⇒ ω < 0`）。
   - `Cell` / `Point` 路径**逐字不动**（含 `PASS_THROUGH_M` 档）。
   - 单测补三条：① `Face` 且目标在身后很近（原「直行穿过」档被打中）仍 `v == 0`；② `Face` 且 `|Δθ| = 10°` → `v == 0`、`|ω| == cruise_yaw_rate`（弧线档退化）；③ `Face` 且 `|Δθ| <= 8°` → `(0, 0)`。
   - 保持四份两两 `diff` 只剩文件头。

3. **Follow 树 ×4**
   - `HoldLeaf` 改持有快照：`HoldLeaf::new(slot: Arc<StdMutex<Option<Sample>>>, params: FollowParams)`。
     - `tick`：读快照；`snap.is_some() && !lost(snap, params)`（即「太近」）→ `f.out.face_point = Some((s.x, s.y))`、其余 `None`；否则（丢失 / 过期）→ 三个都 `None`。
     - 返回 `Status::Running`（不变）。
   - `StepLeaf::tick` 的 `_` 分支（带内停）：`snap` 有值且未过期 → `face_point = Some((s.x, s.y))`、`next_cell = None`、`target_point = None`。
   - `build()`：`Box::new(HoldLeaf::new(slot.clone(), params))`（`slot` 在 `StepLeaf::new(slot, params)` 之前先 clone 一份）。
   - `halt()` 语义不变（`HoldLeaf` 仍无状态；`StepLeaf` 仍只清 `last`）。
   - 单测补两条：① 太近（`d = D_NEAR`）时 `face_point == Some(人的坐标)` 且 `target_point/next_cell` 为 `None`；② 丢失（`age > STALE_MS`）时三个字段全 `None`。

4. **顺带修 task_45 遗留的两条红单测**（`5f183ae` 把机栈距离改成 3.6/2.6 时漏改用例）
   - `pleiades-uav/src/uav/mission/follow.rs:465-466` 与 `pleiades-sim/src/sim/uav_sim/mission/follow.rs:465-466` 硬编码 `sample(1.7, 0)` 断言「带内两向都验」；机栈 `1.7 <= D_NEAR(2.6)` 已属「太近」，第 466 行必红。
   - 改法：把 `1.7` 换成由常量推导的带内值（如 `(D_FAR_M + D_NEAR_M) / 2.0`），使四份用例与各自常数自洽；**不改生产代码去迁就用例**（口径 #10）。

5. **收尾核对**
   - `grep -rn 'face_point' pleiades-*/src` 只应命中：`frame.rs`（定义/`target()`/`clear()`/单测）+ 四份 `follow.rs`；**不得**出现在 `robot_handler.rs`、`executor.rs` 之外的模块（执行器只经 `MotionTarget::Face` 感知，不直接读 `face_point`）。
   - 四份执行器两两 `diff` 只剩文件头；四份 `follow.rs`：`ugv ≡ sim_ugv`、`uav ≡ sim_uav` 只差文件头，车↔机只差 `use` + 两个距离常量 + 文件头。
   - `git diff --stat` 自证：`nav/` / `detector/` / `protocol` / 急停 / Pictor **零改动**。

**验收（静态，不跑仿真）**

- `cargo check -p {pleiades-base,pleiades-sim,pleiades-ugv,pleiades-uav} --all-targets` 0 error、新增代码 0 warning；
- `cargo test -p pleiades-base --lib behavior_tree`、`cargo test -p <crate> --bins executor`、`cargo test -p <crate> --bins mission`（三个设备 crate 只有 `[[bin]]`，故用 `--bins`），四份执行器与四份 `follow.rs` 的单测全绿（含步骤 4 修掉的两条）；
- 端到端（跟人转圈不丢目标）属 **Step 3**，本轮不做。

---

### Step 2 — 行人常驻控制台（ws 套 ws）

**小目标**：仿真里的行人**一次启动、长期存活**；人和脚本都能经本机控制口随时让他走 / 停 / 转身。不再「每次操作重生成一个人」（重建 ⇒ 新 `peer_id` ⇒ `Follow` 丢目标）。

**交付物**：`orion-simulation-world/tool/pedestrian/ped_console.py`（新文件，与 `pedestrian_client.py` / `patrol.py` 同级）

**归属与范围（重要）**

- `orion-simulation-world/` **在根仓内、已被 git 跟踪**（`Pictor/` 才是独立子仓）⇒ 本 step 的改动以普通文件进入 task_46 的提交；
- 该文件是**公共工具**（三处现役行人工具之一），不是单 task 夹具，故不放 `tmp/`、不放 gitignored 的 `RobotSim_TestEnv/`；
- **世界源码零改动**（不改 `WebSocketManager.gd` / `Pedestrian.gd` / `WireProtocol.gd`）；不碰 Rust；不碰 Pictor。

**设计口径：一个 python 进程 ↔ 一个行人（不引入归属解耦）**

行人 avatar 的生命周期本来就该等于它那条连接的生命周期（`_CleanupLink → _RemoveAvatar`，`WebSocketManager.gd:845-880/940`），这正是**「一台设备 = 一个人」的语义**：

- 要几个行人就起几个 `ped_console`（不同 `--peer-id`），彼此状态天然隔离，不需要共享的 avatar 注册表；
- 「进程退出 = 这个人离场」是**预期行为**，与真实场景一致（人不在了就该从场景消失）；
- 世界侧因此**不需要** `persistent` 标志、不需要「他者指挥」、也不需要为无主 avatar 再造一套 pose 订阅关系。

所以本任务**不做** avatar 归属解耦，也不把它记为遗留项。使用约定只有一条：**先起 `ped_console`，再注入 `Follow`；实验期间不要退那个进程。**

**为什么世界源码可以不动**

世界侧能力已经够：`motion_cmd {v, dir}` 的 `v`/`dir` 各自独立、缺省保持原值（`src/chassis/Pedestrian.gd:89-115`），300 ms 无指令**只自动 `v=0`、不删人**（`WebSocketManager.gd:107`、`:455-475`）；`PedestrianClient` 已经实现了「连上 → 30 Hz 持续发 → 10 Hz 收 pose」（`pedestrian_client.py:265-560`，API：`connect/drive/stop/tick/sleep_to_next/pose/pump/close`）。缺的只是**一个往它里面塞命令的入口**。

**形态**

```
线程 1  主循环（兼控制环）
   while running:
       c.pump(0)                                   # 收 pose
       if goal:
           err = atan2(gy - y, gx - x)
           |err - yaw| > 70° → c.drive(0.0, err)                     # 先原地转（v 不能为负）
           否则              → c.drive(min(speed, d * 2.0), err)
           d <= tol          → 清 goal; c.drive(0.0); 唤醒等待者
       c.tick(); c.sleep_to_next()                 # 30 Hz 持续发，永不触发 300 ms 空闲停车

线程 2  控制口监听   TCP 127.0.0.1:9101（UTF-8 JSON-lines，一问一答）
```

`goal` 放 `Mutex`，`goto` 的「阻塞到到位」由监听线程等 `Condition`——**闭环不占监听线程**，否则两个 `goto` 会互相踩。

**控制口契约**

```text
端口   : TCP 127.0.0.1:9101（只绑本机，无鉴权；与 sim 控制口 9098、Pictor ws 9100 同口径，9101 未被占用）
编码   : UTF-8，一行一条 JSON（`\n` 结尾），一问一答
请求   : {"id": <int>, "cmd": "<name>", ...按 cmd 的参数}
成功   : {"id": <int>, "ok": true,  "data": {...}}
失败   : {"id": <int>, "ok": false, "error": "<code>", "why": "<人类可读>"}
错误码 : bad_json | unknown_cmd | bad_arg | not_ready | timeout | frozen | preempted | shutdown
```

| cmd | 参数 | 语义 | 返回 `data` |
|---|---|---|---|
| `goto` | `x, y, [speed=1.0], [tol=0.15], [timeout=60]` | 闭环走到世界坐标点；**阻塞**到到位 / 超时 / 冻结 / 被抢占 | `pos, d, seconds` |
| `face` | `x, y` | 原地转向朝该点（`v=0`）；立即返回 | `dir` |
| `walk` | `v, dir` | 开环直走；立即返回 | — |
| `stop` | — | `v=0` 并清 goal；立即返回 | — |
| `pos` | — | 当前位姿 | `x, y, yaw, v, pose_age` |
| `status` | — | 连接 / 目标 / 节拍 | `peer_id, connected, goal, pose_age, send_hz` |
| `quit` | — | `drive(0)` → 关连接 → 进程退出（人随之消失） | — |

示例：

```bash
$ echo '{"id":1,"cmd":"goto","x":70,"y":66}' | socat - TCP:127.0.0.1:9101
{"id":1,"ok":true,"data":{"pos":[70.0,66.0],"d":0.02,"seconds":4.3}}
```

**行为细节**

1. 控制环 **20 Hz**；`goto` 的 `speed` 上限夹到世界 `max_speed`（缺省 `1.26`，`Pedestrian.gd:47`）。
2. 发送节拍 **30 Hz**（`tick()` + `sleep_to_next()`），节拍用**单调时钟**，落后超过 2 个周期就**重置节拍**而不是补发突发帧（`pedestrian_client.py` 头注释第三条硬边界）。
3. **冻结检测**：下过 `v > 0` 但位姿 **3 s 无位移** → `goto` 返回 `error:"frozen"`（世界掉到 3~4 FPS 时 300 ms 超时会打平，实测过；`step3_driver.py` 的 `walk_to` 已用同一判据）。
4. **同一时刻只有一个 goal**：后到的 `goto` 覆盖前一个，被覆盖者返回 `error:"preempted"`。
5. `quit` / `SIGINT` → 先 `drive(0)` 再关连接（干净退出会产生一条 `[ws] pedestrian removed`）。
6. 无世界时 `connect()` 失败 → 控制口仍在监听，所有指令返回 `error:"not_ready"`，不 traceback、不退出。

**验收（静态 + 冒烟）**

- `python3 -m py_compile orion-simulation-world/tool/pedestrian/ped_console.py` 通过、`--help` 正常；
- 无世界时：控制口可连、`pos` 返回 `not_ready`、进程不退；
- 起世界后冒烟四条：`goto` 到位、`face` 让 `yaw` 变化而位置不变、`stop` 后 `v=0`、`quit` 后世界日志出现一次 `pedestrian removed`；
- 不改 Rust / Pictor / 世界源码（`git status` 自证只有 1 个新文件）。

**边界与已知限制（必须写进交付说明）**

- **过程即生命**（设计如此）：一个 `ped_console` 对应一个行人，进程退出 = 这个人离场（世界日志 `pedestrian removed`）。故实验约定是「先起 console、再注入 `Follow`，期间不退进程」；这不是缺陷，不需要解耦。
- `v >= 0`，**不能倒着走**；`goto` 目标在身后时靠 `dir` 先转（控制环已如此）。
- 行人 `dir` 是**世界系绝对朝向**，不是角速度；转向速率由世界侧 `max_turn_rate`（180°/s）限速。
- 世界重启 / socket 断 → console 会退出 → 人随之离场；重起 console 即为「换了一个人」（新 `peer_id`，`Follow` 需重新注入）。这属于环境重建，不是本工具要解决的问题。

---

## 三、Step 3 — 去掉世界预置的 2 个行人

**背景**：世界初始化时会自己造 2 个常驻行人（`src/world/World.gd:131-134` 的 `PEDESTRIANS` 表 + `:155` `_ready` 里调 `BuildPedestrians()`）：

| 名字 | 世界坐标 | 初始朝向 |
|---|---|---|
| `pedestrian_01` | `(44.0, 96.0)` | `yaw 0°`（朝东） |
| `pedestrian_02` | `(52.0, 102.0)` | `yaw 180°`（朝西） |

它们是**真正的 `Pedestrian` 节点**（`CharacterBody3D`、有碰撞、可被雷达扫到），但**不在 WS 的 `_pedestrians` 表里**（由 `World` 建、不是 `_Register_Pedestrian` 建）⇒ `motion_cmd` / `ped_console` 都驱动不了它们，永远站着不动（世界注释自述「没有外部连接时人就站着不动，这是预期行为」）。

**为什么要去掉**

1. **会抢 Follow 目标**：它们是 `person`，会进相机 → 进 YOLO → 变成 `Target`；而 `TargetSource::sample` 是 `min_by(range_m)` 取**最近的一个**（`pleiades-ugv/src/ugv/mission/follow.rs:179-182`，四份同构）⇒ 只要它们比目标行人更近，`Follow` 就跟错人。
2. **是物理障碍**：`collision_layer = 2`，世界注释写明 `Vehicle.mask = 7` 含 layer 2 ⇒ 落在行车路线上会挡车。
3. 相对出生点 `(64,64)` 距离 37.7 m / 39.8 m，**超出 detector `max_range_m = 12`**，所以出生点附近的实验不受影响；但只要把车/机开进 `(44..56, 96..106)` 那块广场就会撞上。
4. Step 1 的「停住时跟踪」验证要求**画面内只有目标行人**，否则机头转对了也可能跟到路人身上，误判成「没跟对」。

**改法**

| 文件 | 改动 | 状态 |
|---|---|---|
| `src/tests/` 整目录 | **删除**（19 个 `check_*.gd` + 19 个 `.uid` + `doubles/TestChassis|TestSensor` 4 个，共 42 文件）。这些是「断言 N 项 / 检查几条信号」式的一次性自检，不作为验收判据，也不为它们保留任何兼容 | ✅ 已删（2026-11-03，git 可恢复） |
| `src/world/World.gd` | 删 `PEDESTRIANS` 表、`BuildPedestrians()` 函数、`_ready()` 里的调用、`_pedestrians_root` 字段与 `GetPedestriansRoot()`（自检是它唯一调用方）；**保留** `PEDESTRIANS_NODE` 常量并在注释里写明「容器由 `WebSocketManager._Pedestrians_Root()` 在首个行人到达时惰性创建」 | 待做 |
| 注释清理 | `World.gd:13/194/222/259`、`Facade.gd:189`、`Pedestrian.gd:182` 里引用 `check_bounds/check_models/check_world_look` 的注释删除（否则指向已删文件） | 待做 |
| `COMPONENTS.md` | 删掉 19 个自检的表格行与相关引用（§3 脚本表、§4.11.2、§9.1 E29、§11），避免文档指向已删文件 | 待做 |

**不做**：不加 `@export build_pedestrians` 之类的开关，不为任何自检保留分支或兼容，也不把「世界里有没有人」写成可配置项。

**验收**

- 起世界看日志：不再有 `[world] pedestrians built: …`（该函数已删）；
- 动态行人照常可用：`chassis_hello{kind:"pedestrian"}` 到达时 `WS manager _Pedestrians_Root()` 惰性建出 `Pedestrians` 容器并在其中装配 avatar；
- 仓库里不再有任何 `src/tests/` 残留、也没有指向它们的注释引用；
- **不以任何「断言 N 项」式自检作为本步判据**（已整体删除）。

**边界**：只动 `World.gd`（删表/删函数/删容器访问器）+ 删 `src/tests/` + 清注释与文档；不改 `Pedestrian.gd` / `Pedestrian.tscn` / `WebSocketManager.gd` / `WireProtocol.gd`；不碰 Rust / Pictor。

---

## 四、Step 4 — 验证：sim ugv / sim uav 的 Follow 与 Containment

**目标**：用**最新代码**跑一次现象级验证（不做基线对照），确认 sim 车 / sim 机两栈的 `Follow` 与 `Containment` 现象仍然出现，并单独验证 `ped_console`（ws 套 ws）能把行人驱动起来。不设数值门槛（沿用 task_45 Step 3 口径 #10）。

### 4.1 编译（CUDA）⚠️ 有个静默降级的坑

现状：`pleiades-sim/Cargo.toml` 对 base 是 `default-features = false`，而 base 自己的 `[features] default = ["cuda"]`。实测：

```bash
cargo tree -p pleiades-sim         -e features -i pleiades-base   # → base 无 cuda（CPU 版）
cargo tree                          -e features -i pleiades-base   # → base 带 cuda（全工作区）
```

即 **`cargo build -p pleiades-sim --release` 会静默编出 CPU 版**；只有全工作区 `cargo build --release` 才靠 feature 统一把 cuda 带进来。为了不依赖这种隐式行为，给 sim 加显式转发（2 行，不动 ugv/uav —— `ddde6bb` 已因那台车没卡把 ugv 退回 CPU）：

```toml
# pleiades-sim/Cargo.toml
[features]
default = ["cuda"]
cuda = ["pleiades-base/cuda"]
```

编译命令（nvcc 需 gcc-12，`build.sh` 里那段 PATH 手工带上）：

```bash
mkdir -p /tmp/gcc12
ln -sf /usr/bin/gcc-12 /tmp/gcc12/gcc && ln -sf /usr/bin/g++-12 /tmp/gcc12/g++
PATH=/tmp/gcc12:$PATH cargo build -p pleiades-sim --release
```

- 本机 4× **A100-SXM4-80GB**；
- sim 侧两实例的 config 都要 `[detector].enabled = true` + `[detector].device = "cuda"`；
- **编译后核对**：`cargo tree -p pleiades-sim -e features -i pleiades-base` 必须显示 `cuda`；否则别开始验证（CPU 版 detector 单帧 100~400 ms，tick 会被拖垮，现象不可信）。

### 4.2 环境

| 角色 | 启动 | 端口 |
|---|---|---|
| 世界 | `orion-simulation-world`（Step 3 之后已无预置行人） | 9999 |
| 地面站 | `godot --headless --path Pictor` | 9100 |
| sim 车 | `pleiades-sim`，`chassis = diff_drive_ugv`，带 `camera_front` + detector(cuda) | 9098（控制口） |
| sim 机 | `pleiades-sim`，`chassis = quadrotor_uav`，同上 | 9098（换端口） |
| 行人 | `ped_console.py`（Step 2），`--peer-id human-01` | 9101（控制口） |
| 任务注入 | `Pictor/tool/pictor_ws_ctl.py` 的 `follow` / `containment` 子命令 | — |

### 4.3 Case（5 个，一次跑完）

| # | 栈 | 任务 | 看什么 |
|---|---|---|---|
| **C0** | **行人** | **`ped_console` 走一圈** | 依次 `goto` 四个点绕一圈（或 `face` + `walk` 组合）：人行进连续、朝向随目标改变、`stop` 立刻停、`pos` 读数与日志一致；**全程不掉线**（世界日志不出现 `pedestrian removed`，`peer_id` 不变） |
| C1 | sim ugv | Follow | 车朝**人**靠近（不是朝格心）；到 ~1.2 m 停住；**停住后 `ped_console` 让人横移 2~3 m，车头是否继续转向跟人** |
| C2 | sim ugv | Containment | 四角顺时针巡逻（**方框半边长取 4~5 m**，16 m 见方会触发 `goto` 60 s 超时）；人进方框 → 切跟随；出框 → 回巡逻；跟随分支停住时同样看机头跟踪 |
| C3 | sim uav | Follow | 机朝人飞；到 **2.2~2.6 m** 停住（机栈 `D_NEAR = 2.6`）；停住后人横移，机头是否跟踪、`dets` 是否掉零 |
| C4 | sim uav | Containment | 四角巡逻 + 人进/出框切分支（方框半边长同上） |

C0 是其余四个 case 的前提：**行人驱动不起来，后面四个都无从判**。所以 C0 先跑，跑通再跑 C1~C4。

### 4.4 判据（现象级，不设数值门槛）

- 只判「**现象是否出现**」：行人有没有真的走起来（C0）、Follow 有没有跟、停住后有没有继续转头、Containment 有没有巡逻与切分支。
- 不判到位误差、摆幅、耗时、tick 频率、弧线半径；数值只记录，不参与 PASS/FAIL。
- 出现「完全不动」「任务不进队」「日志报错/panic」「连接掉线」才算 FAIL。
- 不做基线对照：只看**当前最新代码**下这 5 个现象是否成立。

### 4.5 记录与收尾

- 每个 case 记一段日志摘录：`[Sim] UGV/UAV 位置=…`、`[Nav] 新目标 …`、`[Robot] 开始任务 …`、`[detector] → 世界=(x,y)`、`[ws] pedestrian …` + 一句话结论。
- 收尾：关世界 / Pictor / 两个 sim / `ped_console`，确认端口 9999、9098、9100、9101 无残留进程（`ss -ltnp`）。
- 不接受任何「断言 N 项」式自检作为本步判据（`src/tests/` 已整体删除，见 Step 3）。
