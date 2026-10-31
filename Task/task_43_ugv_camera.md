# task_43_ugv_camera — UGV 相机设备（camera_ugv）

> Presented by KeJi
> Created Date ： 2026-10-31
> Modified Date ： 2026-10-31
> 状态：**Step 1 ~ Step 6 已定稿**（2026-10-31，人类逐条确认）。执行按 `.github/instructions.md` 的执行引擎流程（每 step：规划 → 实现 → 评审）。

## 一、目标

1. **真车（`pleiades-ugv`）新增相机设备 `camera_ugv`**：后端走 Angstrong HP60C 厂商 SDK（运行时 `dlopen` `libAngstrongCameraSdk.so`，不编译期链接、不加第三方 crate 依赖）；取帧是 SDK 回调推送，写入共享槽位，供 detector 读最新帧。
   实现参考 2026-10-31 的实机验证材料 `rust_uav_camera_test/ugv_camera/`（`RESULTS.md`、`rust_probe/`）——**执行时须让子 agent 先读这份材料**。
2. **`[camera_ugv]` 启动默认 `false`**：相机独占 USB，与车上 ROS 栈互斥，默认关、按需开。
3. **sim ugv 也具备相机**：`pleiades-sim` 的车栈要有与真车同形的相机来源（世界推帧 → 槽位），供 detector 使用。
4. **sim ugv 的 follow 行为树**：实现车栈的真 Follow 树（当前车栈是平凡 `Success`），并完成**单车 sim ugv** 的跟随验证。
5. **sim ugv 的 containment 行为树**：实现车栈的真 Containment 树（当前车栈是平凡 `Success`），并完成 **4 车**验证——其中 **1 车带 camera**、其余 3 车不带。

---

## 二、已定口径（人类 2026-10-31 确认，执行时不再回头问）

| # | 项 | 定值 |
|---|---|---|
| 1 | camera 归属 | **不进 base**。每个设备端自带一份 camera 代码，即使逐字节相同也不共享。`pleiades-ugv/src/device/camera_ugv/`，其中 `slot.rs` 是第 4 份拷贝（`camera_uav` / `camera_pc` / `camera_sim` 已有 3 份），文件头必须写明「与谁同形、差异点」 |
| 2 | HP60C 后端 | 运行时 `dlopen` + `dlsym`（12 个 C++ mangled 符号），**零第三方依赖**、不编译期链接；`StartStream(cam, 0x83)` 后**只读 `mjpegImg`**（`pstData + 288` 那一路，已是完整 JPEG） |
| 3 | 深度 | **不做**。保持 `0x83`（唯一实测过的 flag），depth/rgb 两路只存在不拷贝；槽位只有 JPEG |
| 4 | 配置默认值 | `[camera_ugv] enabled = false` |
| 5 | Lua | **Lua 侧 camera 全部移除**：三张表（`camera_uav` / `camera_pc` / `camera_sim`）+ `programs/user/` 里 6 个相关脚本 + 相关文档同步 |
| 6 | sim ugv 相机 | **复用 `camera_sim`**（`SensorSet` 声明式装配，与 chassis 无关），不为车另写一套 |
| 7 | 车栈 follow | **与 sim 机栈同构**：照抄 `uav_sim/mission/follow.rs`，叶子**不经 D\*** |
| 8 | `TaskDeps` | `detector` / `assist` 与机栈同形、**均可为 `None`**（无相机 → 原地 Hold，不静默 `Success`） |
| 9 | containment 验证 | 4 车：**1 车**跑真 containment，**3 车待命**（不给指令、等协作请求）；**Pictor 协控参与**且**排除请求者**；环上分 **3 个点**（`θ = 2π·i/3`，`R = 3.0 m`） |
| 10 | assist 发布 | 复用 sim 侧已有的 `NodeHandlePublisher`，经 `AssistPublisher` 薄端口进车栈；**base 零改动**（topic / msgid 7 / `encode_assist_request` / `Gossipsub_Publish_Try` 均已在 base） |
| 11 | 验收边界 | 本机**没有车**：真车部分只做「代码正确」的静态验收（允许 `cargo check` / 单测，**禁止**上车、串口、开流实验）；仿真部分按 Step 4 / 6 实跑 |
| 12 | **Step 4 / 6 的验收口径** | **以「行为出现」为准，不卡数值指标**。判据分两层：**硬判据** = 行为确实发生（跟车/停车/跟丢停、请求/指派/任务不断）；**软观测** = 距离带宽、振荡、帧率、误差、耗时，**只记录、不判 FAIL**。评审**不得**自行追加人类未确认的量化门槛。数值不理想属已知限制（本机 CPU 推理 ~0.67 s/帧、车栈档位执行器粗糙），**最终 PASS 由人类在本机带 GPU 亲验**。理由：以往把指标定太严 → 一直不通过，但人类本地跑一次就过，白白耗轮次 |
| 13 | **Step 4 / 6 失败不阻断** | 这两个验证步**只跑 1 轮**：无论 PASS 还是 **FAIL**，都把结果与证据写进 workbook，然后**继续下一个 step**（不重跑、不中断、不回滚）。**例外**：**代码级失败**（编译不过 / 单测红 / 启动即 panic）**不适用**此条——那属于实现缺陷，按正常流程修（Step 3 / 5 的 3 轮预算），修不动才中断并汇报 |

---

## 三、Step 1 — 真车 ugv camera device（`camera_ugv`）

### 目的

让 `pleiades-ugv` 具备相机能力：SDK 回调 → 共享槽位 → `latest_frame()`，接口与 `camera_uav` 同形，供将来的车端 detector 直接吃。本次**不做** Lua 绑定、**不做**深度、**不上车调试**。

### 涉及文件

**新建**
- `pleiades-ugv/src/device/camera_ugv/mod.rs` —— `Ugv_Camera`：`new / Open / Close / latest_frame / Info / Drop`；**不实现 `DeviceCapability`**（Lua 已移除）
- `pleiades-ugv/src/device/camera_ugv/sdk.rs` —— FFI 层：dlopen/dlsym 符号表、帧结构偏移读、回调（`catch_unwind` + `try_lock` + 当场 memcpy + 原子计数）、生命周期清理
- `pleiades-ugv/src/device/camera_ugv/slot.rs` —— 与 `pleiades-uav/src/device/camera_uav/slot.rs` 同形（`Frame{ jpeg: Arc<[u8]>, width, height, seq, captured_at_ms }` / `FrameSlot` / `Monotonic_Ms`）

**修改**
- `pleiades-ugv/src/device/mod.rs` —— `pub mod camera_ugv;`
- `pleiades-ugv/src/config.rs` —— `CameraUgvConfig`（`enabled` 默认 false / `sdk_lib` / `config_json` / `width` / `height` / `stream_fps` / `timeout_ms`）+ `camera_ugv: Option<CameraUgvConfig>` + 在 `ensure_ugv_section` 里补 `fill_camera_ugv`（缺才补、幂等）
- `pleiades-ugv/src/bootstrap.rs` —— `enabled` 才构造；`Open` 失败只 `warn!` + 跳过；把 `Arc<Ugv_Camera>` 交给 `CarDeviceHandler`
- `pleiades-ugv/src/ugv/robot_handler.rs` —— 持 `Option<Arc<Ugv_Camera>>`（本 step 不消费，留给车端 detector）

**Lua 移除（横跨三 crate，只删不增）**
- `pleiades-uav/src/device/camera_uav/mod.rs`、`pleiades-uav/src/device/camera_pc/mod.rs` —— 删 `register_lua_caps` 与 `Capture` 的 Lua 出口（`Info()` / `Describe_Stream()` **保留**，bootstrap 还在用）
- `pleiades-sim/src/sim/device_cap.rs` —— 删 `camera_sim` 表（若该文件整体只剩它，则整个能力注册退化为空，需确认是否保留这个文件）
- `pleiades-uav/src/bootstrap.rs:69` —— 「camera_pc.capture 已注册到 Lua」这句日志作废
- 删脚本：`programs/user/{camera_photo,test_camera,yolo_uav,yolo_test_uav,yolo_ugv,yolo_test_ugv}.lua`
- 文档：`.github/instructions.md:16`、`Architecture/Pleiades_Architecture.md §3.7.3`、`docs/capabilities.md`

### 实施步骤

1. 子 agent **先读参考材料**：`rust_uav_camera_test/ugv_camera/RESULTS.md`、`.engine/BRIEF.md`、`rust_probe/src/main.rs`（1 610 行，参考实现，不入库）。
2. 抄 `slot.rs`（第 4 份）：文件头写清「与 `camera_uav/slot.rs` 同形，差异：无；同步责任方：本任务」，并在模块注释里指向 `camera_uav` 那份。
3. `sdk.rs`：
   - 12 个 mangled 符号名逐条写注释（`AS_SDK_Init` / `Deinit` / `GetSwVersion` / `GetCameraList` / `OpenCamera` / `SetStreamParam` / `RegisterStreamCallback` / `StartStream` / `StopStream` / `CloseCamera` / `DestoryCamHandle` / 可选诊断三个）；
   - 帧结构按 **偏移常量化**（`OFF_FRAME_TYPE/WIDTH/HEIGHT/DATA/SIZE/BUFFER_SIZE/ID/TS` + `OFF_DATA_MJPEG = 288`），配 `const _: () = assert!(...)` 与布局自检函数；
   - 回调：`catch_unwind(AssertUnwindSafe(...))` → 读 size → 闸门（`>0` 且 `<= 8 MiB`）→ `try_lock` 写槽位 → 原子计数；**零日志**；
   - 清理：`StopStream(cam, 0)` **只一次**（`started.take()` 结构性保证）→ `CloseCamera` → `DestoryCamHandle` → `Deinit` → `dlclose`；**不调用 `AS_SDK_FreeCameraList`**（实机 `double free` → SIGABRT，参考 `RESULTS.md` §6），代码里注明原因。
4. `mod.rs`：`Open` = dlopen → Init → GetCameraList 取首句柄 → OpenCamera(加密 json) → SetStreamParam(DEPTH/RGB 640×480@25) → RegisterCallback → StartStream(0x83)；回调线程写槽位；`latest_frame()` 同步非阻塞克隆 `Arc<[u8]>`；`Close` 幂等。
5. config / bootstrap / handler 接线（默认 false，失败只 warn，不影响主循环）。
6. Lua 移除 + 文档同步（清单见上）。
7. 评审：逐条对照本节「验收」+ `.github/instructions.md` 的命名/归属规范。

### 验收（**不上车**）

1. `cargo check -p pleiades-ugv --all-targets` 0 error，新增代码 0 warning（可跑编译，不跑实验）；
2. 回调链扫描：`sdk.rs` 回调体内 0 处 `println!/eprintln!`、只 `try_lock`、无 `unwrap/expect`；
3. 偏移与符号名逐条能对上参考实现（评审逐条比对 `rust_probe/src/main.rs`）；
4. `[camera_ugv]` 缺失时程序不 panic，`enabled=false` 时零线程；
5. Lua 移除后 `cargo check -p pleiades-uav -p pleiades-sim --all-targets` 0 error，且无遗留的 `camera_*` Lua 表引用。

---

## 四、Step 2 — sim ugv 相机接入

### 目的

让 sim 车也有相机帧（世界推 → 槽位），detector 能在车栈上起起来；不改 chassis 判型、不新写相机类。

### 涉及文件

**代码（预计很小）**
- `pleiades-sim/src/sim/sim_handler.rs` —— 确认「相机挑选」与「detector 启动」都与 chassis 无关（现状见 `:386-396` 挑相机、`:570-609` 起 detector、`:611` 才是按 chassis 分栈）；按需补日志/注释
- `pleiades-sim/src/sim/sensors/mod.rs` —— `SensorSet::Build` 的 `"camera"` 分支（`:128` 附近），确认无 chassis 判据（预计不改）
- `pleiades-sim/src/sim/detector/mod.rs` —— 已 `impl FrameSource for CameraSensor`（`:90`），**预计不改**
- 世界侧 —— `sensors/CameraSensor.tscn` 已有；按现有代码**不需要改**，挂点靠配置声明 + `mount`

**配置（不入库，属验证环境）**
- 节点 `.config/config.toml` 加：`[[sim_world.sensor]]`（`slot = "camera_front"` / `sensor = "camera"`）+ `[sim_world.sensor.mount]`（`x` / `y` / `z` / `yaw` / `pitch` 五个键，**按车高 0.25 m 取值**，不是机的高度）+ `[detector]`（`enabled = true` / `model = "yolov8s.safetensors"` / `device = "cpu"`）
- 权重放 `Pleiades_Workspace/yolov8s.safetensors`

### 实施步骤

1. 读 `SensorSet::Build` 的 camera 分支与 `sim_handler` 5.2（挑相机）/ 5.5（起 detector），确认两者都不看 chassis。
2. 起一个 sim 车节点 + 世界，配置里声明相机，看日志 `[camera] slot=… 首帧 WxH N B seq=…` 与 `[detector] detector 设备已 spawn`。
3. 若发现世界侧确实只给机挂相机（即需要改 `Vehicle.tscn` / `World.gd`），才动世界侧——**先按「不需要改」验证**。
4. 记录：车节点确实拿到帧（`latest_frame()` 有值）、detector 有推理输出。

### 验收

1. sim 车节点日志出现相机首帧 + detector spawn；
2. 无 `[camera] … 帧丢弃` 洪泛、无 `panic`；
3. `cargo check -p pleiades-sim --all-targets` 0 error（若有代码改动）；
4. 世界侧代码零改动（若不得不改，需在 workbook 里记明原因）。

---

## 五、Step 3 — sim ugv follow 行为树

### 目的

把车栈 `Follow` 从「平凡树一拍 `Success`」换成真树，与机栈**同构**（照抄叶子，不经 D\*）。

### 涉及文件

**新建**
- `pleiades-sim/src/sim/ugv_sim/mission/follow.rs` —— 照抄 `uav_sim/mission/follow.rs`（`ReactiveFallback[ ReactiveSequence[TargetGate, Condition(未太近), StepLeaf], HoldLeaf ]` + `step_cell` / `guard_cell` / `D_FAR_M=4.0` / `D_NEAR_M=2.5` / `STALE_MS=800` / `PERSON_INFLATE_M=0.5`）

**修改**
- `pleiades-sim/src/sim/ugv_sim/mission/mod.rs` —— `TaskDeps` 加 `detector` / `assist`（与机栈同形，默认 `None`）；`Follow` 分派到真树；删掉「车没有相机/detector，Hold 会永远占着队首」（`:33`）与 D-S6 相关注释
- `pleiades-sim/src/sim/ugv_sim/mod.rs` —— `Stack` 加 `detector` / `assist` 字段；`Stack::new` 加这两个参数；`build` 组装 deps
- `pleiades-sim/src/sim/sim_handler.rs` —— 调用点把 `detector` 也传给车栈；`NodeHandlePublisher` 提到 `match` 外构造一次（两栈共用）；删 `:628` 的「车栈不传」注释
- `pleiades-sim/src/sim/control.rs` —— `follow` 的帮助文案从「sim 无人机；x/y 预留」改为「车/机通用」

### 实施步骤

1. 复制叶子文件，只改 `use` 与类型路径（**不改行为**）。
2. `TaskDeps` 扩字段 → `Follow` 分支改真树；`detector = None` 时退化为**原地 Hold**（`Action` 恒 `Running` + `next_cell = None` + `warn`），**不得**一拍 `Success`。
3. `Stack::new` 签名与机栈对齐（差异只剩 `map_tx` 与 `ObstacleMode`）。
4. 调用点接线，确认车栈与机栈拿的是**同一个** publisher 实例（`Arc` 克隆）。
5. 单测：照搬机栈那几条（无 detector → Hold；`step_cell` 方向；`guard_cell` 人格否决；`halt` 清迟滞）。
6. 评审：diff 与机栈叶子逐行比对，确认「只差 `use`」。

### 验收

1. `cargo check -p pleiades-sim --all-targets` 0 error、新增 0 warning；
2. `cargo test -p pleiades-sim --bins follow` 全绿（车栈新增单测）；
3. 车栈 `Follow` 不再返回平凡 `Success`；`detector=None` → 树保持 `Running` 且 `next_cell=None`；
4. `ugv_sim/mission/goto.rs` 与 `nav/` 零改动。

---

## 六、Step 4 — sim ugv follow 验证（单车）

### 目的

单车 sim ugv（带 camera + detector）跟一个人，验证「树 + 执行器 + detector」三件套在车里真的能用。

### 涉及文件

- 运行环境（不入库）：`RobotSim_TestEnv/<node>/`（1 台 sim ugv + 世界 + 行人）、节点 `.config/config.toml`
- 验证脚本：`tmp/task43_tools/`（按「`Tool/` 只放公共工具」的规矩，本任务专用脚本放 `tmp/`）
- 产物：`tmp/task43/`（日志 / 截图）+ `Workbook/wb_43_ugv_camera.md`

### 实施步骤

1. 起世界 + 1 台 sim ugv（`[sim_world.sensor]` 声明相机、`[detector] enabled=true`）+ 1 个行人（task_35 机制，速度 0.5 m/s）。
2. 通过本地控制口（WebSocket `127.0.0.1:<control.port>`）注入 `{"cmd":"follow"}`，并用 `{"cmd":"status"}` 采状态。
3. 观察四件事：
   - 距离稳在带内（`> 4.0` 前进、`≤ 2.5` 停、带内迟滞不抖）；
   - 人停下 → 车停；人走 → 车跟；
   - 目标丢失（人出画 / `age > 800 ms`）→ 停车，**不追最后一个点**；
   - 车不原地打转、不撞墙（车栈 `ObstacleMode::Respect` + 急停仍在）。

### 验收（以「行为出现」为准，不卡数值）

> **轮次与失败处理（口径 #13）**：本步**只跑 1 轮**；跑完无论 PASS / FAIL 都记录 + 继续 Step 5。
> 例外：编译不过 / 单测红 / 启动即 panic 属**代码级失败**，不适用「继续」，回 Step 3 修。

**硬判据（必须出现，缺一才 FAIL）**

1. `Follow` 树真的在跑：人动 → 车动；人停 → 车停（不是一直原地不动，也不是一直往前冲）；
2. 跟丢（人出画 / `age > 800 ms`）→ 车停住，且**不追最后一个点**；
3. 全程无 `panic`、无崩溃，日志可解析；
4. 证据（日志 / 截图）落进 `Workbook/wb_43_ugv_camera.md`。

**软观测（只记录，不判 FAIL）**

- 距离是否落在 2.5~4.0 m 带内、有无过冲/小幅振荡；
- 跟随时的 `age_ms` 与 detector 输出频率（本机 CPU 约 0.67 s/帧，频率天然低）；
- 是否偶发绕小圈（车栈「档位 + 追格心」的固有现象）。

> **口径（人类 2026-10-31 强调）**：本步的目的是「看到 follow 的行为确实出现」，不是「数值达标」。数值不理想属于已知限制，记录即可；**PASS 由人类在本机带 GPU 亲验**。评审不得自行追加量化门槛。

---

## 七、Step 5 — sim ugv containment 行为树

### 目的

车栈 `Containment` 成真树（正方形巡逻 + 界内跟随 + 协助请求），并让请求能发出去（assist 端口）。

### 涉及文件

**新建**
- `pleiades-sim/src/sim/ugv_sim/mission/containment.rs` —— 照抄 `uav_sim/mission/containment.rs`（`ReactiveFallback[ ReactiveSequence[InsideSquare(+publisher), follow], RepeatForever(4×goto) ]` + `corners` / `inside_square` / `intruder`），**并复制 `AssistPublisher` trait 定义**（按口径 1：每个设备自带）

**修改**
- `pleiades-sim/src/sim/ugv_sim/mission/mod.rs` —— `Containment` 分派到真树；无 detector → 原地 Hold（同 follow）
- `pleiades-sim/src/sim/sim_handler.rs` —— `NodeHandlePublisher` 补一份 `impl sim::ugv_sim::mission::containment::AssistPublisher`（方法体与机栈那份逐字相同，共用 `assist_frame` 纯函数）
- **Pictor（独立仓 `Pictor/`，分支 `godot-rust`）** —— `src/cooperation/coop_controller.gd`：① `_on_assist_request` 里**排除请求者**（`vid` 不进 targets）；② 环上按 **3 点**分（`θ = 2π·i/3`，`R = 3.0`）；单测 `src/test/test_coop_controller.gd` 同步

**零改动**
- `pleiades-base`（topic / msgid 7 / 编码 / `Gossipsub_Publish_Try` 都已就绪）

### 实施步骤

1. 复制树与 trait；`ugv_sim` 的 `TaskDeps.assist` 打通到 `InsideSquare`。
2. `sim_handler` 里补 publisher 的第二个 `impl`（同一个 `NodeHandlePublisher`，两个 trait）。
3. Pictor 改「排除请求者 + 3 点」，并补/改单测（含「请求者不在 targets」断言）。
4. 单测：树形（外层必须 `ReactiveFallback`）、角点顺序、`inside_square` 边界含界内、assist 重发阈值 `1.0 m`、`halt` 清 `last_sent`。

### 验收

1. `cargo check -p pleiades-sim --all-targets` 0 error；`cargo test -p pleiades-sim --bins containment` 全绿；
2. `godot --headless --path Pictor -s src/test/test_coop_controller.gd` → `ALL PASS: true`；
3. `base` 零改动（`git diff pleiades-base/` 为空）；
4. assist 帧字节契约仍为金标 `3f80000040000000`（8 B payload）。

---

## 八、Step 6 — sim ugv containment 验证（4 车）

### 目的

4 台 sim ugv：**1 台带 camera** 跑 containment，**3 台待命**；人进入正方形 → 该车发协助请求 → Pictor 协控给另外 3 台各分一个环上点 → 三车到位 → 人离开后请求车回巡逻。

### 涉及文件

- 运行环境（不入库）：4 个 sim 节点（`c1` 带 `[sim_world.sensor]` 相机 + `[detector] enabled=true`；`c2~c4` 不带）+ 世界 + Pictor
- Pictor 节点配置：`[Network].subscribe_topics` **必须含** `pleiades/robot/assist_request`（默认订阅清单里没有）
- 验证脚本：`tmp/task43_tools/`；产物：`tmp/task43/`

### 实施步骤

1. 起世界 + 4 台 sim ugv + Pictor（内核 `.so` 需含 assist 转发分支）。
2. 确认 Pictor 日志有「已订阅 N 个 topic」且包含 assist。
3. 给 `c1` 注入 containment（`{"cmd":"queue","missions":[{"type":3,"x":…,"y":…,"half_side":…}]}` 或经 Pictor 的 containment 命令）。
4. 行人走进正方形 → `c1` 判入侵 → 发 assist（`TOPIC_ROBOT_ASSIST_REQUEST`，54 B 帧）→ Pictor `[Coop] 请求` → 指派 3 车 → `[Coop] 指派 车=… 点#…`。
5. 观察 `c1` **不被** `TASK_SET` 打断（它不在 Pictor 的 targets 里）；人离开正方形 → `c1` 回巡逻。

### 验收（以「行为出现」为准，不卡数值）

> **轮次与失败处理（口径 #13）**：本步**只跑 1 轮**；跑完记录 + 结束（本步是最后一个 step）。
> 例外同 Step 4：代码级失败不适用「继续」，回 Step 5 修。

**硬判据（必须出现，缺一才 FAIL）**

1. assist 链路贯通：`c1` 发出协助请求（Pictor 日志有 `[Coop] 请求`），`unknown msgid` 为 0；
2. Pictor 给 3 台待命车各下发一条 Goto（`[Coop] 指派` 三条）且 3 车都动起来；
3. `c1` 的 containment 任务**不断**（队列未被 `TASK_SET` 替换、`task_id` 不变；人离开后回巡逻）；
4. 全程无 `panic`、无碰撞卡死；
5. 全流程日志与证据写入 `Workbook/wb_43_ugv_camera.md`。

**软观测（只记录，不判 FAIL）**

- 3 车的到点误差与到位耗时；
- 环上 3 点的视觉对称性（缺口是否正好在传感器车那侧）；
- `c1` 的重发次数（`ASSIST_MOVE_M = 1.0` 触发）、检测频率、跟随时延。

> 同 Step 4 口径：数值只记录、不判 FAIL；**PASS 由人类在本机带 GPU 亲验**。

---

## 九、不做

- 不做深度、不做目标跟踪（不做 3D / 俯仰补偿 / 平滑）；
- 不做真车的 detector / follow / containment（真车与机端 `Follow` / `Containment` 仍是平凡树）；
- 不改 D\*（四份 pathfinder 不动）、不改执行器常量（`DEADBAND_DEG` / `ARC_MAX_DEG` / `PASS_THROUGH_M`）；
- 不给 sim 世界新增资产（相机模型 `camera_module.glb` 已有）；
- 不把参考材料 `rust_uav_camera_test/ugv_camera/` 入库；
- `tmp/task43/` 是**中间产物**（step.md / plan.md / impl.md / review.json），引擎结束时清理；**验证产物**落 `RobotSim_TestEnv/results/task43_s{4,6}_<时间戳>/`（含 `verdict.md` 与日志），长期保留。

---

## 十、风险

| # | 风险 | 判据 / 处理 |
|---|---|---|
| R1 | 本机无车，SDK 符号名 / 结构体偏移只能静态对齐参考实现 | 评审逐条比对 `rust_probe/src/main.rs`；上车首跑若有 ABI 问题，报错信息必须可读（函数名 + 返回码） |
| R2 | `0x83` 是唯一实测 flag；若某车固件降级 `0x03`，`mjpegImg.size` 恒 0 | 回调计数 `no_mjpeg`，超时诊断里打出来；不静默降级 |
| R3 | detector 在 CPU 上约 0.67 s/帧（task_41 实测），跟人时视觉频率低 | follow 判据按「人 0.5 m/s 慢走」设计；跟丢即停是**预期行为**，不是 bug |
| R4 | 车栈执行器是「档位 + 追格心」，跟随时可能过冲/绕小圈 | 按口径 #12：「偶发绕小圈 / 过冲」归**软观测**，只记录；只有「人走车不走 / 原地打转不前进」才触犯硬判据 1。真出现严重打转再查 `step_cell` 的 0.5 m 步长 |
| R5 | Pictor 排除请求者后，环上 3 点可能留缺口在与传感器车同侧 | 属视觉不对称，不影响功能；需要时后续用「基于请求者方位的 offset」优化 |
| R6 | `slot.rs` 第 4 份拷贝 → 漂移 | 文件头标注同形关系；改动时人工 `diff` 三份 |
| R7 | **评审把指标定太严 → 假 FAIL 死循环** | Step 4 / 6 只认「行为出现」的硬判据（口径 #12）；数值一律归入软观测、只记录；评审若追加量化门槛，视为**评审越界**，按口径 #12 驳回 |
