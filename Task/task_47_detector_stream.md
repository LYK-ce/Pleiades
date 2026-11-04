# task_47_detector_stream — detector 检测流（WS 推「图 + 框」）

> Presented by KeJi
> Created Date ： 2026-11-04
> Modified Date ： 2026-11-04
> 状态：Step 1 ~ Step 5 待人类确认（未开始）。
> 上游结论：`tmp/survey/04_vm_lua.md` / `06_device_crates.md`（2026-11-04 全项目梳理）确认 `API/webui.rs` 这条链**有服务、有格式、有查看器，但零生产者**（task_29 之后 `caps.webui.send` 无人调用）。

---

## 一、目标

把 detector **每一帧推理结果**（原图 JPEG + 框坐标 + 元数据；没框也推）通过 WebSocket 推出去，用 `Tool/yolo_viewer.html` 在浏览器里直接看「画面 + 框」。

一句话形状：

```
detector 推理循环 ──(同步、非阻塞)──► base 的广播(16) ──► axum /ws ──► yolo_viewer.html
      │                                   ▲
      └─ 一处调用（三份拷贝各一行）────────┘（服务未启动 / 无订阅者 = 静默丢弃）
```

### 1.1 现状（代码事实）

| 事实 | 位置 |
|---|---|
| detector 每 100 ms 轮询相机槽位，`seq` 变了才推理，结果覆盖进唯一槽位 | `pleiades-uav/src/device/detector/mod.rs:404-578` |
| 帧是零拷贝的：`Frame { jpeg: Arc<[u8]>, width, height, seq, captured_at_ms }` | 同上 `:72`（`pub use camera_uav::slot::Frame`） |
| 推理用的局部 `jpeg` 与 `frame.jpeg` **可能不是一张图**（ugv `flip_vertical=true` 时先翻转再推理） | `pleiades-ugv/src/device/detector/mod.rs`（`Flip_Jpeg_Vertical` + `Loop_Ctx.flip_vertical`） |
| 快照 `Snapshot { dets, targets }` **不持有图**，图在推理完就丢 | 同上 `:150-155` |
| WS 服务、消息格式、广播、查看器**都已存在** | `pleiades-base/src/API/webui.rs`（95 行）、`Tool/yolo_viewer.html` |
| 格式：`[4B header_len **LE**][JSON header][JPEG 原始字节]`；查看器读 `header.dets[].{class_name,xmin,ymin,xmax,ymax}` | `API/webui.rs:60-71`、`yolo_viewer.html`（`getUint32(0,true)`） |
| 广播 `broadcast::channel(16)`，慢客户端 `Lagged → continue`（丢帧，不反压生产者） | `API/webui.rs:38,91` |
| 唯一的启动入口是 TUI `webui [port]` 命令 | `TUI/mod.rs:561-572` → `Orchestrator/command.rs:72` → `core/branch_user.rs:511-530` |
| 唯一的生产者是 Lua `caps.webui.send`，而 `programs/` 下**没有任何脚本用它** | `VM/capability_binding.rs:338-355`；`grep -rn webui programs/` = 0 |
| 三份 detector（ugv 988 / uav 942 / sim 940 行）除文件头、相机类型、ugv 的 `flip_vertical` 外逐字相同 | `tmp/survey/06_device_crates.md` §1 |

### 1.2 为什么服务器不写进 detector

- **依赖成本**：`axum` 与 `serde_json` 只有 base 有；`pleiades-ugv/Cargo.toml` 与 `pleiades-uav/Cargo.toml` **两个都没有**（sim 只有 serde_json）。把服务器塞进 detector 要给两个 crate 加依赖。
- **三份实现**：detector 是 3 份拷贝，塞进去就是 3 份服务器代码 + 3 份生命周期管理。
- **背压**：socket 一旦挨着推理循环，迟早有人在里面 `await`。`broadcast` + `Lagged` 丢帧是唯一不会反压推理的形状。
- **重复造轮子**：消息格式、多客户端广播、慢客户端丢帧、查看器（还支持多车多面板）都是现成的。

### 1.3 为什么由 detector 启动（而不是 base 全局启动）

- detector **本来就依赖 camera**（无相机 / `[camera].enabled=false` → detector 不 spawn），所以「服务只在带相机的设备上存在」**天然成立**，不需要第二套判断。
- 配置、启动、生命周期三者绑定在同一个设备上，与 `[camera]` / `[lidar]` / `[chassis]` 完全同构；`stream = false` → 连 bind 都不做（零线程）。
- 代价可接受：**实现**在 base 一份，**启动权**在设备端（device crate 调用 base 函数，本仓既有做法）。

---

## 二、已定口径（人类已确认，实施时不得改）

| # | 项 |
|---|---|
| 1 | WS 服务器**实现**留在 `pleiades-base/src/API/webui.rs`（一份），**不**写进 detector 文件 |
| 2 | 配置**全部**放 `[detector]` 段：`stream` / `stream_port` / `stream_bind`；缺省 `false` / `9010` / `"127.0.0.1"` |
| 3 | 只有 `DetectorDevice::spawn` **成功之后**才启动服务；失败只 `warn`，**不影响推理**（与 `[lidar]` 打不开同口径） |
| 4 | `stream = false` → **不 bind、零线程**（不是"起了不推"，是根本不启动） |
| 5 | 端口由各节点 config 各自写死；**不做**自动挑端口。`stream_port = 0` 时按 `bind` 由内核分配（`listener.local_addr()` 回填，日志打真实端口） |
| 6 | `stream_bind` 可配；非法值 `warn` + 回退 `127.0.0.1`（device 口径：不 panic） |
| 7 | 消息格式**沿用** `[4B header_len LE][JSON][JPEG]`，`Tool/yolo_viewer.html` **一行不改** |
| 8 | header 在现有 `{dets:[...]}` 基础上**增**字段：`seq` / `frame_ts_ms` / `infer_ms` / `width` / `height`；查看器自动忽略未知字段 |
| 9 | **每一帧推理结果都推**（有框 / 无框都推，`dets` 为空也照发）；不额外发心跳，但"有帧到达"本身就证明链路活着 |
| 10 | 推的 JPEG **必须**是送进 `Detect` 的那一份（ugv 翻转后的局部 `jpeg`），不是 `frame.jpeg` |
| 11 | **不在服务端画框**：发原图 + 坐标，客户端画（省一次 decode+draw+encode，约 10~30 ms/帧） |
| 12 | v1 **不缩放、不降采样**（带宽问题按下表记为已知限制，不在本任务解） |
| 13 | TUI `webui [port]` 命令**整体移除**（解析 + `UserCommand::Webui` + 分支处理），人类明确不需要第二个启动入口 |
| 14 | 服务未启动 / 无订阅者 → **静默丢弃**（沿用现有广播语义），detector 不因推送失败改变任何行为 |
| 15 | `caps.webui.send`（Lua）**保留不动**；服务被 detector 起来后它同样可用（本轮不评估去留） |
| 16 | 单测**不作**端到端验收判据；端到端由人类开浏览器看（沿用 task_45/46 口径） |
| 17 | `Tool/yolo_viewer.html` **移除全部 FPS 显示**（右下角"总 FPS"、每面板 FPS、相关计数器与 `setInterval`）；面板头右侧的文本位改作**连接状态**（`连接中… / 已连接 / 连接失败 / 断开`），不再显示帧率 |
| 18 | 推送策略与"省带宽"脱钩：v1 **不做** `stream_only_dets` 之类的开关，空场景照推（要省带宽另开任务） |

### 已知限制（记录，本任务不修）

| 限制 | 说明 |
|---|---|
| 带宽 | 1280×720 JPEG q80 约 100–200 KB；10 Hz ≈ 1–2 MB/s/节点。**每帧都推 ⇒ 空场景（画面里没人）也照占满这个带宽**。4 车同时看 ≈ 32–64 Mbps，WiFi 上会开始丢帧（丢帧安全但会花屏） |
| 新客户端首帧 | 广播不重放；查看器连上后要等**下一帧**（≤ 一个推理周期，约 100 ms~1 s）才出画面 |
| 无鉴权 | `stream_bind = "0.0.0.0"` 时同网段任何人都能看画面；默认 `127.0.0.1` 就是要避免这一点 |
| 位姿近似 | header 不带位姿；detector 的日志行才有（`pose=(...)`） |
| 帧龄 | 查看器不显示 FPS 也不显示"这帧多老"；要判断卡死只能看画面是否在变 |

---

## 三、涉及文件

| 层 | 文件 | 动作 |
|---|---|---|
| base | `pleiades-base/src/API/webui.rs` | ✏️ 加 `StreamDet` + `publish_detection`；`spawn_webui_server` → `ensure_webui_server(SocketAddr)`（幂等 + 回填真实端口） |
| base | `pleiades-base/src/API/mod.rs` | ✏️ re-export 新函数 |
| base | `pleiades-base/src/Orchestrator/command.rs` | ✏️ 删 `UserCommand::Webui { port }` |
| base | `pleiades-base/src/Orchestrator/core/branch_user.rs` | ✏️ 删 `UserCommand::Webui` 分支（`:511-530`） |
| base | `pleiades-base/src/TUI/mod.rs` | ✏️ 删 `webui [port]` 解析块（`:561-572`） |
| ugv | `pleiades-ugv/src/config.rs` | ✏️ `DetectorConfig` 加 3 字段 + 访问器 + 常量；`fill_detector`（`:449`）补 3 键 |
| ugv | `pleiades-ugv/src/device/detector/mod.rs` | ✏️ spawn 后起服务；循环里推帧 |
| uav | `pleiades-uav/src/config.rs` | ✏️ 同上（`fill_detector` 在 `:463`） |
| uav | `pleiades-uav/src/device/detector/mod.rs` | ✏️ 同上 |
| sim | `pleiades-sim/src/config.rs` | ✏️ 只加 3 字段 + 访问器 + 常量（**无** `fill_detector`，sim 的 config 由夹具生成，不写回） |
| sim | `pleiades-sim/src/sim/detector/mod.rs` | ✏️ 同上 |
| 前端 | `Tool/yolo_viewer.html` | ✏️ 移除全部 FPS 显示与相关计数器/定时器；面板头右侧改显示连接状态 |
| 文档 | `Architecture/robot_arch.md` §8.7 | ✏️ 补「画面流」小节 + 常量速查 |
| **不动** | `API/server.rs`（`find_available_port`）、`VM/capability_binding.rs`（`caps.webui.send`）、`nav/`、`mission/`、`executor.rs`、ORION 协议、急停、Pictor、`orion-simulation-world/` | — |

---

## 四、Step

### Step 1 — base：类型化发布口 + 幂等服务启动

**小目标**：base 提供两个东西：`publish_detection(图, 元数据, 框列表)`（同步、非阻塞、无订阅者即丢）与 `ensure_webui_server(bind: SocketAddr) -> Result<u16, String>`（幂等，返回真实端口）。此时**还没有任何调用方**，TUI 命令也还在（下一 step 才删）。

**改动**

1. `API/webui.rs`
   - 新增：
     ```rust
     #[derive(Debug, Clone, Copy)]
     pub struct StreamDet {
         pub class_name: &'static str,
         pub conf: f32,
         pub xmin: f32, pub ymin: f32, pub xmax: f32, pub ymax: f32,
     }

     /// 服务未启动 / 无订阅者 = 静默丢弃（沿用现有语义）
     pub fn publish_detection(
         jpeg: &[u8], width: u32, height: u32,
         seq: u64, frame_ts_ms: u64, infer_ms: u64,
         dets: &[StreamDet],
     );
     ```
   - 内部：`build_header_json(...) -> String`（纯函数，便于单测）+ 复用现有 `[4B LE][JSON][JPEG]` 打包（把 `webui_publish` 拆成 `pack_frame(header, jpeg) -> Vec<u8>`，两条入口共用）。
   - `spawn_webui_server(port: Option<u16>)` → `ensure_webui_server(bind: SocketAddr) -> Result<u16, String>`：
     - `static SERVER: OnceLock<SocketAddr>`（或 `OnceLock<u16>`）；已启动 → **直接返回已绑定端口**（关键：自动启动 + 将来任何第二个调用方都不会撞 `address in use`）。
     - `bind.port() == 0` 时用 `listener.local_addr()?.port()` 回填。
     - 日志打**真实**地址与端口。
2. `API/mod.rs`：`pub use webui::{publish_detection, ensure_webui_server, webui_publish, StreamDet};`
   > ⚠️ **每步必须单独编译通过**：本 step 结束时 TUI 命令还在，`branch_user.rs:514` 仍调 `spawn_webui_server(port)`。
   > 所以本 step **允许**把 `spawn_webui_server(Option<u16>)` 留成**薄包装**（内部转 `ensure_webui_server(SocketAddr)`），
   > 到 Step 2 删 TUI 分支时**一并删掉**该包装。
3. 单测（`webui.rs` 尾部）
   - `pack_frame` 的字节布局：`header_len` 是 **LE**、JSON 在 `[4..4+len)`、JPEG 紧随其后、总长 = `4+len+jpeg.len()`。
   - `build_header_json`：`dets` 里的键名与查看器一致（`class_name/conf/xmin/ymin/xmax/ymax`）；`seq/frame_ts_ms/infer_ms/width/height` 都在。
   - `ensure_webui_server(127.0.0.1:0)` 连调两次 → **同一端口**（幂等）。
   - `publish_detection` 在服务未启动时返回且不 panic。

**验收（静态）**

- `cargo check -p pleiades-base --lib` 0 error、新增代码 0 warning；
- `cargo test -p pleiades-base --lib webui` 全绿；
- `git diff --stat` 证明本 step **只碰** `API/webui.rs` 与 `API/mod.rs`。

---

### Step 2 — base：移除 TUI `webui` 命令

**小目标**：删掉第二个启动入口，服务只能由 detector 拉起。

**改动**

1. `TUI/mod.rs`：删 `:561-572` 整块（`// ---- webui [port] ----`）。
2. `Orchestrator/command.rs`：删 `Webui { port: Option<u16> }` 变体及其文档注释（`:70-74`）。
3. `Orchestrator/core/branch_user.rs`：删 `UserCommand::Webui` 分支（`:511-530`，含 `spawn` + EventBus 通知）。

**验收（静态）**

- `cargo check -p pleiades-base --all-targets` 0 error；
- `grep -rn "UserCommand::Webui\|spawn_webui_server\|webui \[port\]" pleiades-base/src` → **0 命中**；
- `grep -rn "webui" pleiades-base/src` 只剩 `API/webui.rs` 自身 + `API/mod.rs` 的 re-export + `VM/capability_binding.rs` 的 `caps.webui.send`（口径 #15 保留）；
- `TUI/mod.rs` 单测（若有命令解析用例）不受影响 → 跑 `cargo test -p pleiades-base --lib`。

---

### Step 3 — 三份 detector：配置 + 启动 + 推帧

**小目标**：`[detector].stream = true` 时，detector 起来后自动开 WS；之后每出一帧**有框**的结果就推一份（图 + 框 + 元数据）。

**改动（三份同形，只允许差相机类型 / `flip_vertical` / 文件头）**

1. `config.rs`（三份）
   - `DetectorConfig` 加：
     ```rust
     /// 检测流：true = detector 起来后自动开 WS（false = 连 bind 都不做）
     pub stream: Option<bool>,
     /// 检测流端口（各节点自己配；0 = 内核分配）
     pub stream_port: Option<u16>,
     /// 检测流绑定地址（默认只本机；想让别的机器看写 "0.0.0.0"）
     pub stream_bind: Option<String>,
     ```
   - 常量：`DEFAULT_DETECTOR_STREAM = false`、`DEFAULT_DETECTOR_STREAM_PORT: u16 = 9010`、`DEFAULT_DETECTOR_STREAM_BIND = "127.0.0.1"`。
   - 访问器：`Stream() -> bool`、`Stream_Port() -> u16`、`Stream_Bind() -> &str`（空串 → 默认）。
   - `fill_detector`（ugv `:449` / uav `:463`）补三个键（缺失才补、幂等、写回）；**sim 无此函数，不加**。
2. `detector/mod.rs`（三份）
   - `DetectorDevice::spawn` 成功路径末尾（`Spawn_Tuned` 之前或之后均可）：
     ```rust
     if cfg.Stream() {
         let addr = parse_stream_addr(cfg.Stream_Bind(), cfg.Stream_Port()); // 非法 = warn + 127.0.0.1
         match pleiades_base::api::ensure_webui_server(addr).await {
             Ok(p)  => info!("[detector] 画面流已开 ws://{}:{}/ws", ip, p),
             Err(e) => warn!("[detector] 画面流启动失败（推理照跑）：{e}"),
         }
     }
     ```
   - `Loop_Ctx` 加 `stream: bool`；推理完成、快照写入后：
     ```rust
     if stream {
         publish_detection(&jpeg, frame.width, frame.height,
                           frame.seq, frame.captured_at_ms, infer_ms,
                           &to_stream_dets(&dets));   // dets 为空也照推（口径 #9）
     }
     ```
     - `jpeg` = **送进 `Detect` 的那一份**（ugv 是翻转后的局部变量，口径 #10）。
     - `to_stream_dets(&[Detection]) -> Vec<StreamDet>`：纯函数，字段映射 + `class_name` 是 `&'static str`（`coco_names` 里就是 `'static`）。
     - **不** `await`、**不**处理返回值（广播 send 同步）。
   - `shutdown()` 语义**不变**：只 `cancel()` 推理 task；WS 服务器是进程级，随进程退出（不 join、不优雅关闭，登记为已知行为）。
3. 单测（三份同形，纯函数级）
   - `to_stream_dets` 字段映射（含 `class_name` 指针不复制、顺序不变）。
   - 假引擎喂一帧 2 个框 → 断言**推了 1 次**、`dets.len() == 2`；再喂一帧 **0 个框** → 断言**仍推了 1 次**、`dets.len() == 0`（口径 #9 的判据）。
   - `stream = false` 时不绑端口、不推帧：断言 `ensure_webui_server` **未被调用**（`SERVER.get().is_none()`）且 `PUBLISH_COUNT == 0`（`#[cfg(test)] pub(crate) static PUBLISH_COUNT: AtomicU64`，**不**起真实 WS）。
4. 三份一致性
   - `diff pleiades-uav/src/device/detector/mod.rs pleiades-sim/src/sim/detector/mod.rs` 只允许差文件头 + 相机类型别名 + `FRAME_TS_SOURCE`；
   - ugv 那份额外只允许差 `flip_vertical` 相关（既有事实）。

**验收（静态）**

- `cargo check -p {pleiades-base,pleiades-ugv,pleiades-uav,pleiades-sim} --all-targets` 0 error、新增代码 0 warning；
- `cargo test -p <crate> --bins detector`（三个设备 crate 只有 `[[bin]]`）全绿；
- `grep -rn "publish_detection" pleiades-*/src` 只应命中：`API/webui.rs`（定义）+ 三份 `detector/mod.rs`（调用）+ `API/mod.rs`（re-export）；
- 配置文件自证：在没有 `[detector].stream` 的旧 config 上跑一次 `Ensure_Ugv_Config` / `Ensure_Uav_Config`，三个键被补齐且**其余行不变**（`git diff` 或 `diff` 旧新文件）。

---

### Step 4 — 前端：`Tool/yolo_viewer.html` 去掉 FPS

**小目标**：查看器不再计算、不再显示任何帧率；面板头右侧的文本位改显示**连接状态**（`连接中… / 已连接 / 连接失败 / 断开`）。解析与画框逻辑一行不动。

**改动（按当前行号，删完请复核行号已变）**

| 行 | 现在 | 改成 |
|---|---|---|
| `:14` | CSS `.panel .fps { color: #4ade80; ... }` | 删（改为 `.panel .status { color: #9ca3af; }` 或直接不加样式） |
| `:24` | `<span id="totalFps">- FPS（总）</span>` | 删整行（"总 FPS"不再存在） |
| `:30-31` | `let totalFrames = 0;` / `let totalStart = 0;` | 删两个变量 |
| `:44` | `<span class="fps" id="fps${idx}">-</span>` | `<span class="status" id="st${idx}">连接中…</span>` |
| `:53-54` | `const fpsEl = ...` / `let last = 0;` | `const stEl = document.getElementById("st" + idx);`（删 `last`） |
| `:61-62` | `onerror/onclose` 往 `fpsEl` 写 | 往 `stEl` 写 `连接失败` / `断开` |
| `:65-66` | `totalStart` / `totalFrames++` | 删两行 |
| `:94` | `fpsEl.textContent = (1000/(now-last)).toFixed(1) + " FPS"` 与 `last = now` | 删（`img.onload` 里只留 `drawImage` + 画框） |
| `:107-116` | `setInterval(...)` 总 FPS 块 | 删整块 |

另加一行：`ws.onopen` 里 `stEl.textContent = "已连接";`（原来只 `console.log`）。

**不改**：`[4B header_len LE][JSON][JPEG]` 的拆包、`header.dets[]` 的画框、多面板/断开逻辑、坐标（框仍是原图像素坐标）。

**验收**

- `grep -in "fps\|totalFrames\|totalStart" Tool/yolo_viewer.html` → **0 命中**（连注释和 CSS 类名都不能留）；
- 浏览器打开：能连、有画面、有框；面板头在连接/断开时显示对应状态；**页面上没有任何帧率数字**；
- 可选自证（本机有 node 时）：把 `<script>` 段抽出来 `node --check`，确认没有语法错误。

---

### Step 5 — 端到端人工验收 + 文档

**小目标**：人类开浏览器确认「画面持续刷新 + 框正确 + 无 FPS」；把口径写进架构文档。

> 引擎口径：执行引擎只做**文档 + 静态终检**（本节第 3 条与 §七 的静态项）；下面的 1、2 两步浏览器验收由**人类**在引擎结束后手动完成，引擎不启动仿真/浏览器。

**做**

1. 起一个带相机 + detector 的节点（sim 最省事）：
   - `[detector] enabled = true, stream = true, stream_port = 9010, stream_bind = "127.0.0.1"`；
   - 日志应出现 `[detector] 画面流已开 ws://127.0.0.1:9010/ws`；
   - `ss -ltn | grep 9010` 在**开流时**有监听，`stream = false` 时**没有**（口径 #4 的判据）。
2. 浏览器打开 `Tool/yolo_viewer.html` → 填 `127.0.0.1` + `9010` → 连接；
   - 画面应出现并**持续刷新**（口径 #9：空帧也推，画面里没人时也在动）；
   - 框位置正确（**ugv 必验**：`flip_vertical = true` 时框不能画反 —— 口径 #10 的判据）；
   - 让框消失（人走出画面）→ **画面继续刷新，只是没有框**（不是冻结）；
   - 页面上**不出现任何 FPS 数字**，面板头只显示连接状态（口径 #17）；
   - 拔掉 `stream` 所在进程 → 面板头变 `断开`。
3. 文档：`Architecture/robot_arch.md` §8.7 加一小节「画面流（task_47）」，写清配置键、默认值、消息格式、**每帧都推**的带宽后果、以及"服务未启动即静默丢弃"；常量速查表补 `stream_port` 默认 9010 / `stream_bind` 默认 127.0.0.1。

**验收**

- 人工确认（浏览器）：「画面持续刷新 + 框正确 + 框消失后画面仍在动 + 无 FPS + 连接状态正确」；
- `git diff --stat` 自证：`nav/` / `mission/` / `executor.rs` / ORION 协议 / 急停 / Pictor / `orion-simulation-world` **零改动**（`yolo_viewer.html` 只在 Step 4 改过）；
- 文档改动只落在 `Architecture/robot_arch.md`。

---

## 五、边界（本任务明确不做）

- 不做鉴权 / token / TLS（`bind` 默认本机就是唯一防线）。
- 不改 `Tool/yolo_viewer.html` 的**拆包 / 画框 / 多面板**逻辑（只按口径 #17 删 FPS、加连接状态文本）；不新增其它前端。
- 不做服务端画框、不做缩放/降采样/质量重编（口径 #11/#12）。
- 不落盘（不存图、不存视频、不存检测日志）。
- 不补发"最新一帧"给新客户端（广播不重放，登记为已知限制）。
- 不改 `caps.webui.send` 的契约，也不评估它的去留（口径 #15）。
- 不动 `nav/` / `mission/` / `executor.rs` / ORION 帧格式 / 急停 / Pictor / `orion-simulation-world`。
- 不做多节点聚合入口（一个节点一个端口，查看器自己开多面板）。

---

## 六、风险

| 风险 | 应对 |
|---|---|
| 端口被占（同机跑 sim + ugv + uav） | 配置各写不同端口；起失败只 `warn`，推理不受影响（口径 #3/#5） |
| ugv 翻转导致框画反 | Step 5 明确要求 ugv 必验；单测覆盖 `to_stream_dets` 与"推的是局部 jpeg" |
| 推送拖慢推理 | `publish_detection` 是同步 `send`（失败即丢），无 `await`；`Lagged` 丢帧在消费侧 |
| 空场景也占满带宽 | 口径 #9/#18 明确接受；省带宽的开关不在本任务 |
| 三份 detector 漂移 | Step 3 收尾要求两两 `diff` 只剩文件头/相机类型/`flip_vertical` |
| 与 `caps.webui.send` 抢同一个广播 | 同一格式、同一频道；Lua 路径当前无脚本使用，若将来启用需约定 header 字段并集 |
| 旧 config 没有新键 | 全 `Option` + 默认值 ⇒ 功能上等价 `stream = false`；`fill_detector` 只补缺失键，不动已有行 |

---

## 七、验收总表

| 项 | Step | 判据 |
|---|---|---|
| 打包格式与查看器兼容 | 1 | `pack_frame` 单测（LE、偏移、总长） |
| 服务幂等 | 1 | 连调两次同端口 |
| 无生产者时静默 | 1 | 未启动调用不 panic |
| TUI 命令彻底移除 | 2 | `grep` 0 命中 |
| 配置三键 + 幂等补全 | 3 | 旧 config 跑一遍只多三行 |
| 每帧都推（含 0 框） | 3 | 单测：喂 0 框帧仍推 1 次 |
| 推的是推理用的那张图 | 3/5 | ugv 浏览器实测框不反 |
| `stream=false` 零监听 | 5 | `ss -ltn` 无该端口 |
| 框消失后画面仍在刷新 | 5 | 浏览器实测（不是冻结） |
| 查看器无任何 FPS | 4 | `grep -in "fps\|totalFrames\|totalStart"` 0 命中 |
| 查看器仍能画框 | 4/5 | 浏览器实测 |
| 文档同步 | 5 | `robot_arch.md` §8.7 + 常量速查 |
