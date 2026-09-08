# Workbook — Task 29: YOLO-v8 目标检测（阶段 2 检测核心 + 阶段 3 图传输）

> 对应任务：`Task/task_29_yolo.md`
> 分支：`robot_yolo`
> 创建日期：2026-09-08

## 状态

阶段 0/1/2 已实施 + 验证通过（已提交推送）。阶段 3 Rust 侧 + 单帧测试脚本已实施（待两节点联调）。阶段 4（webui + HTML 画框）未实施。

## 关键决策（与 task_29 方案 D1-D13 的偏差，均已落地）

- **`ml.yolo_new(model_file, device)` 两参数**（非文档 device/path/which 三参数）：权重文件名作第一参数，型号从文件名推断（n/s/m/l/x），权重放 `Pleiades_Workspace/`。换模型只改脚本一行，不动 Rust。
- **detect 返回检测尺度（resize 后）坐标，不缩放回原图**（文档原设计缩放回原图）：与 rust_yolo 基准直接逐位对比，省换算。
- **读写盘复用 storage**：`StorageReadHandle` 补 `read()`（对称阶段 1 的 `StorageWriteHandle::write()`），不新增 read_file_bytes cap。
- **YOLO 走 `register_ml_caps` 的 `ml` 表**（`ml.yolo_new`），不进 `Capabilities` 结构体、不走 `DeviceCapability`（与 `MlContext` 同构）。

## 改动文件

- **base 新增**：`ML_Engine/Yolo/{mod.rs, model.rs(758行移植), detector.rs, coco_names.rs}`
- **base 改**：`ML_Engine/mod.rs`（`#[path="Yolo/mod.rs"] pub mod yolo`）、`lib.rs`（导出 Yolo_Detector/Detection）、`VM/capability_binding.rs`（ml.yolo_new）、`VM/storage_handle.rs`（StorageReadHandle::read）、`Cargo.toml`（image="0.25"）
- **新增脚本**：`programs/user/yolo_test.lua`（读图 → detect → 打印）
- **新增 example**：`pleiades-base/examples/yolo_detect.rs`（支持命令行传模型名）

## 验证

- `cargo check -p pleiades-base --no-default-features` → 0 error
- example + Lua `exec yolo_test` 端到端：yolov8n **22 框**（15 person/5 bicycle/2 motorcycle）、yolov8l **28 框**（+car+dog，person 19），与 rust_yolo 基准**逐位一致**（置信度/坐标）

## 关键坑（后续 agent 注意）

- `model.rs` 移植只改 2 处 `candle::`→`candle_core::`（`candle_nn::` 不变）；candle 版本两边都是 0.10.2
- 大写目录 `Yolo/` 需 `#[path = "Yolo/mod.rs"]`（同 `GGUF_Models` 惯例）；裸 `pub mod yolo;` 会找小写 `yolo.rs` 报 E0583
- mlua `String::as_bytes()` 返回 `BorrowedBytes`，需 `.as_ref()` 转 `&[u8]`（`detect` 入参）
- Lua 沙箱禁 `io`（engine.rs），读文件走 `storage_acquire_read` + `handle:read()`
- rust_yolo 跑 l 需 `--which l`（默认 n 会 shape mismatch）；我们的 `Infer_Which` 从文件名自动推断，无此问题
- mDNS `failed reading datagram`（os error 10040）= libp2p-mdns 接收缓冲区硬编码 4096 字节，超大米 DNS 包被丢弃，与 YOLO 无关，忽略

## 阶段 3（图传输 U8 tensor）— Rust 侧 + 单帧测试脚本

> 2026-09-08 实施完成，待两节点联调验证

### 状态
Rust 侧（U8 dtype + 两个 cap）+ 单帧测试脚本（yolo_test_uav/yolo_test_ugv）已实施并推送。联调（UAV 抓图 → 传 UGV → detect）待本地两节点验证。

### 关键决策（与李永康讨论定）
- **先单 UAV → 单 UGV 单帧跑通**，正式 3 车轮询脚本留后续。
- **命名**：`tensor_from_u8_bytes` / `tensor_to_u8_bytes`，与 `tensor_from_bytes` 的区别已写入 task 文档 + `lua_tensor.rs` 注释（四函数对照）：
  - `tensor_to/from_bytes` = 序列化对（带 `[dtype][ndim][dims]` header，跨网络恢复张量）
  - `tensor_from/to_u8_bytes` = wrap/unwrap 对（无 header，裸字节进出 tensor stream）
- **YOLO 输入尺寸核实**：非固定 640×640，而是「长边 640、保比例、32 对齐」（1280×720 → 640×352），无 letterbox 补边。camera capture 的 JPEG 直接喂 detect，无需中间处理。
- **测试脚本参数写死在脚本顶部「配置块」**（李永康要求：直接改脚本文件，不用 exec 传参）。
- **peer 发现**：脚本顶部 `PEER_NAME`（默认 "ugv"）按名字找 peer_id；留空则 `get_all_peers()` 自动找第一个非本机节点。
- **task_id（=inference_id）**：tensor stream 的 rendezvous 配对 id，两端一致即可（默认 1001）。

### 改动文件
- **base 改**：`ML_Engine/lua_tensor.rs`（U8 dtype + `tensor_from_u8_bytes_fn`/`tensor_to_u8_bytes_fn`）、`VM/capability_binding.rs`（`ml.tensor_from_u8_bytes`/`ml.tensor_to_u8_bytes`）
- **新增脚本**：`programs/user/yolo_test_uav.lua`、`programs/user/yolo_test_ugv.lua`
- **文档**：`Task/task_29_yolo.md`（阶段 3 状态 + 命名区别对照表）

### 验证
- `cargo check -p pleiades-base --no-default-features` → 0 error（24 个 warning 均为改动前已存在）
- 联调待本地两节点（UAV + UGV）跑通

### 关键坑
- `recv_tensor` 收到 EOF 时**抛异常**（`capability_binding.rs` 返回 `Err("recv_tensor: received EOF")`），Lua 用 `pcall(function() return recv_tensor(...) end)` 捕获判断流结束（模式见 `programs/archived/pipe_2.lua`）。
- candle U8 `from_vec::<u8>` / `to_vec1::<u8>` 首次在项目使用，`cargo check` 验证编译通过。
- `tensor_to_u8_bytes_fn` 显式校验 dtype==U8，非 U8 直接报错。
- exec 参数（`params`）在 Lua 里全是字符串（`HashMap<String,String>`），数字需 `tonumber`；但本阶段测试脚本不用 exec 传参，参数写死在脚本顶部。

## 阶段 4（WebSocket 展示）— 已实施，待联调

> 2026-09-08 方案定稿（与李永康讨论）

### 最终方案
- **WebSocket**（非 SSE）：二进制帧传图，免 base64。
- **三车三 WS**：浏览器一个 HTML 连三个 WebSocket，三栏画框 + 各算 FPS。
- **FPS 浏览器算**（`onmessage` 打 `performance.now()` 差分），车端不算。
- **detect 坐标缩放回原图**（方案 B，推翻阶段 2「不缩放回原图」决策）。
- **`webui` TUI 命令**起服务（默认端口 9010，可 `webui <port>`），服务 spawn 到 Core 主 runtime（多线程，不受 Lua detect 阻塞）；不用 Lua 启动。
- **`caps.webui.send(jpeg, dets)`**：dets table → JSON（独立函数 `dets_table_to_json`）→ 打包 → 广播；**webui 未起服务时静默丢弃**。
- 消息格式：一条二进制消息 = 一帧 `[4B header_len LE][header JSON {dets:[...]}][JPEG 字节]`。
- HTML 输入框手动填三车 IP；端口统一 9010。

### 关键决策理由
- 轮询不行：FPS 需要帧精确到达，轮询的拉取间隔污染帧时间戳。
- SSE 可但要 base64（文本协议）；WebSocket 二进制帧直传图，且全双工（将来可反向发命令）。
- FPS 必须浏览器算：三车汇聚点，车端各算各的没意义。

### 待实施文件
- base 改：`Cargo.toml`（axum ws）、`ML_Engine/Yolo/detector.rs`（方案 B）、`API/webui.rs` + `mod.rs`、`Orchestrator/command.rs` + `branch_user.rs`、`TUI/mod.rs`、`VM/capability_binding.rs`
- 新增：`Tool/yolo_viewer.html`
- 车端脚本：`yolo_test_ugv.lua` 加 `caps.webui.send`

## 结束时间

2026-09-08
