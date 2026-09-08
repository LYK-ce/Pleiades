# Workbook — Task 29: YOLO-v8 目标检测（阶段 2 检测核心）

> 对应任务：`Task/task_29_yolo.md`
> 分支：`robot_yolo`
> 创建日期：2026-09-08

## 状态

阶段 0/1/2 已实施 + 验证通过（已提交推送）。阶段 3（图传输 U8 tensor）、阶段 4（webui + HTML 画框）未实施。

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

## 结束时间

2026-09-08
