# task_29_yolo — candle YOLO-v8 目标检测融入 ML_Engine

> Created Date ： 2026-09-06
> Modified Date ： 2026-09-08
> 状态：阶段 0/1/2 已实施（已提交 robot_yolo 分支）；真实权重验证通过（yolov8n/l），Lua 端到端验证与基准逐位一致
> 分支：`robot_yolo`（演示专用，webui 不进正式分支）
> 关联文档：`Architecture/robot_arch.md`、`Task/task_28_camera_device.md`（相机输出 JPEG）、实验项目 `/vepfs-mlp2/c20250205/240804016/Workspace/rust_yolo/`

---

## 一、目标

把 candle 生态的 **YOLO-v8 目标检测**融入现有 ML_Engine 体系，使机器人（UAV/UGV）具备「拍一张图 → 检测 → 得到 bbox」的能力。

- 现有 ML_Engine 是 GGUF/LLM 专用（`MlContext` + 量化 QTensor + 按层加载 + KV cache），YOLO 走 **safetensors 全量加载**（权重文件 FP16 存储：yolov8n 6.4MB / yolov8l 87MB；VarBuilder 加载时转 F32 计算），两者**平行隔离、互不侵入**。
- 已跑通实验：`rust_yolo` 项目（candle 0.10.2）先用随机权重验证链路，后用真实权重（yolov8n/l）检测 bike.jpg 成功（person/bicycle/motorcycle/dog/car 均正确检出，yolov8l 精度更高）。

### 第一版里程碑（验收标准）

跑通完整端到端闭环：**UAV 抓图 → tensor stream 发车 → 车跑 YOLO → webui 发浏览器 → HTML 画框**。一个环节都不能少。

### 实施阶段（拆分）

整个闭环拆成 5 个步骤（阶段 0 前置 + 4 个阶段），按依赖顺序实施：

**阶段 0：设备 Lua cap 注入口（前置，base）✅ 已实施**

解决评审 F1：设备端（uav）的 `CameraDevice` 无法把自己的 cap 注册进 base 的 Lua（跨 crate：uav 依赖 base，base 不能反向依赖 uav）。通用解法：

- base 定义 **`DeviceCapability` trait**（`register_lua_caps(&self, lua: &Lua) -> mlua::Result<()>`），与 network/storage 的 trait object 同风格
- `Capabilities` 加字段 `device_caps: Vec<Arc<dyn DeviceCapability>>`
- `spawn_lua_script` 注册完内置 caps 后，遍历调用 `device_caps` 的 `register_lua_caps`
- camera 留在 uav，实现 `DeviceCapability` trait 后注入（未来 lidar 等设备同理）

**阶段 1：Camera 适配（UAV 抓图）★ 当前第一部分**

整合自 `Task/task_28_camera_device.md`：

- 目标：UAV 的 USB 摄像头做成子设备 `CameraDevice`，第一版只做「拍照返回 JPEG 字节」
- 位置：`pleiades-uav/src/device/camera/`（新建 `device/` 目录；注：mavlink 在 `uav/mavlink/`、lg290p 在 `pleiades-ugv`，非同级）
- 底层：**nokhwa**（`input-v4l`），fourcc 固定 **MJPG**（读一帧 = 一张 JPEG）
- API：`new` / `open` / `capture`（async 返回 JPEG 字节）/ `close` 四方法
- 线程模型：`open` 起 `std::thread` 常驻线程（持摄像头 + loop 等命令）；`capture` 用 `oneshot` 回传
- config `[camera]`：`enabled`（默认 false）/ `path` / `width` / `height` / `timeout_ms`
- 接入：`UavDeviceHandler::start()` 装配 + `shutdown()` 关闭
- 涉及文件：`device/camera/mod.rs`（新增）、`device/mod.rs`、`uav/robot_handler.rs`、`config.rs`、`Cargo.toml`（加 nokhwa）

**阶段 2：检测核心（YOLO 模块）✅ 已实施**

- `ML_Engine/Yolo/`（model.rs + detector.rs + coco_names.rs）
- `ml.yolo_new` + `det:detect` cap
- 先用本地图片文件独立验证检测正确性

**阶段 3：图传输（tensor stream + U8）**

- `lua_tensor.rs` 加 U8 dtype + `ml.tensor_from_u8_bytes` cap
- UAV 发图脚本（capture → U8 tensor → send_tensor）
- 车收图脚本（accept → recv_tensor → detect）

**阶段 4：展示（webui + HTML）**

- `webui` SSE 服务（仿 `api` 命令）+ `webui.send` cap
- HTML 画框 + FPS
- 端到端联调：抓图 → 发车 → YOLO → 发浏览器 → 画框

## 二、设计决策（已初步确定）

| # | 决策 |
|---|------|
| D1 | 核心是纯 Rust 的 `Yolo_Detector` 结构体（长驻，load 一次 detect 多次），通过**薄 Lua cap**（`ml.yolo_new`）暴露给 `exec program` 调用 —— 与 `MlContext` 完全同构 |
| D2 | 放 `pleiades-base/src/ML_Engine/Yolo/` 子目录（对齐 `GGUF_Models/` 做法），与 GGUF 管线完全隔离 |
| D3 | 权重用 `VarBuilder::from_mmaped_safetensors`（candle-nn 0.10.2 已有，无需新依赖），新增 `Resolve_Yolo_Model_Path`（找 `.safetensors`），**不碰** GGUF 的 `.pgguf`/`Resolve_Model_Path` 逻辑 |
| D4 | 输入 = **图像原始字节（JPEG/PNG）**，解码在 `detect` 内部用 `image` crate 完成；Lua 侧不碰像素 |
| D5 | NMS 复用 `candle_transformers::object_detection::non_maximum_suppression`（0.10.2 已有），不手写 |
| D6 | 画框/标注（`imageproc`/`ab_glyph`）**不进 base**，属展示层，留在 terminal/UAV 展示侧 |
| D7 | 检测触发走 **`exec program` 体系**：`exec <脚本>` → `spawn_lua_script`（独立线程）→ Lua 调 `det:detect`（同步阻塞跑 candle），与 LLM 推理 `session inference` 完全同构；**不做 Rust robot 核心直调** |
| D8 | `Yolo_Detector` 是 `Send + Sync`（`YoloV8::forward` 是 `&self`，无状态可变），可安全作为 mlua UserData 在 Lua 线程持有 |
| D9 | 设备解析复用 `Parse_Device_Str`；UAV CPU-only 走 `Device::Cpu`，有 GPU 的 base 节点走 `cuda:0` |
| D10 | 图传输走 **tensor stream**（`open/accept/send/recv_tensor` 现成 cap），UAV 轮询发图零膨胀；**不用 `send_data`**（其 `DataType::Data` 在 Network_Service 内部被吞，需大改架构） |
| D11 | `lua_tensor.rs` 加 **U8 dtype 支持**（`tensor_to_bytes`/`bytes_to_tensor` 加 `DType::U8 => 2`），新增 `ml.tensor_from_u8_bytes` cap（图字节 → U8 Tensor） |
| D12 | 展示走 **webui**（SSE 服务，仿 `api` 命令模式）+ HTML 画框，**各车各传**（每车一个 SSE 服务，浏览器开三连接） |
| D13 | 设备 cap 通过 **`DeviceCapability` trait** 注册（与 network/storage 的 trait object 同风格）：`Capabilities` 加 `device_caps: Vec<Arc<dyn DeviceCapability>>`，`spawn_lua_script` 遍历调用 `register_lua_caps`；camera 留 uav 实现该 trait |

## 三、目录结构（计划）

```
pleiades-base/src/ML_Engine/
├── Yolo/                     ← 新增
│   ├── mod.rs                # pub mod model; pub mod detector; re-export
│   ├── model.rs              # 从 rust_yolo/src/model.rs 移植（755 行网络结构，import 改 candle_core 前缀）
│   ├── detector.rs           # Yolo_Detector + 预处理/后处理 + Resolve_Yolo_Model_Path
│   └── coco_names.rs         # COCO_NAMES: [&str; 80]
├── context.rs / gguf_model.rs / ...   # 全部不动
```

## 四、大体流程（端到端）

```
[UAV — exec 拍照分发脚本]
  camera.capture() → JPEG 字节
  ml.tensor_from_u8_bytes(jpeg) → U8 Tensor
  open_tensor_stream(车N, task_id) → send_tensor(stream, 图tensor, 帧号)
     （轮询：帧 N 发车 N%3，零膨胀）

[UGV — exec 收图检测脚本]
  accept_tensor_stream(task_id) → 循环 recv_tensor → 拿回 JPEG 字节
  ml.yolo_new(device, path, "n") → det:detect(jpeg) → bbox
     ├ image::load_from_memory 解码
     ├ Yolo_Preprocess (letterbox resize)
     ├ YoloV8::forward (DFL 内建)
     ├ Yolo_Postprocess (置信度过滤 + NMS + 坐标缩放)
     └ device.synchronize()
  webui.send(原图 + bbox) → SSE 推给浏览器

[浏览器 — HTML]
  收「图 + 坐标」→ canvas 画框 + 算 FPS
```

- **解码**统一在 `detect` 内（`image::ImageReader` / `load_from_memory`）。
- **前向**：`model.forward(&image_t)?.squeeze(0)` → `[4+80, npreds]` 的 `pred`。
- **同步点**：照抄 `MlContext::forward` 的 `device.synchronize()` 习惯，取坐标前确保计算完成。

## 五、API 设计（初步）

```rust
// pleiades-base/src/ML_Engine/Yolo/detector.rs
pub struct Detection {
    pub class_index: usize,
    pub class_name: &'static str,     // COCO_NAMES[class_index]
    pub xmin: f32, pub ymin: f32,     // 已缩放回原图像素坐标
    pub xmax: f32, pub ymax: f32,
    pub confidence: f32,
}

pub fn Resolve_Yolo_Model_Path(workspace: &Path, model_name: &str) -> Result<PathBuf>;
pub fn Yolo_Load_Model(path: &Path, device: &Device, which: &str) -> Result<YoloV8>;

pub struct Yolo_Detector {
    model: YoloV8,
    device: Device,
    class_names: &'static [&'static str],
}
impl Yolo_Detector {
    pub fn new(device: &str, path: &Path, which: &str) -> Result<Self, String>;
    pub fn detect(&self, image_bytes: &[u8], conf_thr: f32, nms_thr: f32) -> Result<Vec<Detection>, String>;
}
```

Lua cap（薄胶水，`register_ml_caps` 增）：
```lua
local det = ml.yolo_new(device, path, which)      -- which: "n"/"s"/"m"/"l"/"x"
local dets = det:detect(image_bytes, conf, nms)   -- 返回 table 数组
-- dets = { {class_index=0, class_name="person", xmin=.., ...}, ... }
```

- `det:detect` 用 `add_method`（不是 `add_method_mut`，`forward` 是 `&self`）。
- table 拼装提取为独立函数 `detections_to_lua_table`（遵守「闭包只做薄胶水」规范）。

## 六、涉及文件（计划）

| 文件 | 改动 |
|---|---|
| `pleiades-base/src/ML_Engine/Yolo/mod.rs` | ✏️ 新增 |
| `pleiades-base/src/ML_Engine/Yolo/model.rs` | ✏️ 新增（移植） |
| `pleiades-base/src/ML_Engine/Yolo/detector.rs` | ✏️ 新增 |
| `pleiades-base/src/ML_Engine/Yolo/coco_names.rs` | ✏️ 新增 |
| `pleiades-base/src/ML_Engine/mod.rs` | ✏️ `pub mod yolo;` + re-export |
| `pleiades-base/src/lib.rs` | ✏️ 导出 `Yolo_Detector` / `Detection` 等 |
| `pleiades-base/Cargo.toml` | ✏️ 加 `image = "0.25"`（仅解码） |
| `pleiades-base/src/VM/capability_binding.rs` | ✏️ `register_ml_caps` 增 `ml.yolo_new`（阶段二） |
| `pleiades-base/src/ML_Engine/lua_tensor.rs` | ✏️ `tensor_to_bytes`/`bytes_to_tensor` 加 U8 dtype（dtype 2） |
| `pleiades-base/src/VM/capability_binding.rs` | ✏️ 增 `ml.tensor_from_u8_bytes`（图字节 → U8 Tensor） |
| `pleiades-uav/...`（相机接通） | ✏️ 依赖 task_28，后续 task 处理 |

## 七、验证（计划）

- `cargo check -p pleiades-base`（0 error）。
- 单元/冒烟：沿用 `Random` backend 思路做「无权重冒烟测试」。
- 有 `yolov8n.safetensors` 后：一张图走通 `detect` → bbox（对照 rust_yolo 实验）。
- UAV 实机联调（低分辨率 + 跳帧）。

## 八、待讨论问题（本次讨论焦点）

| # | 问题 |
|---|------|
| ~~Q1~~ ✅ | **分支职责（已定）**：YOLO 核心放 `ML_Engine/Yolo/`，无分支边界问题 |
| ~~Q2~~ ✅ | 分辨率：定 **640** |
| ~~Q3~~ ✅ | 第一版只做 detect（80 类），pose 代码保留但不暴露 |
| ~~Q4~~ ✅ | 第一版硬编码 80 类（`COCO_NAMES`），留接口后续可配置 |
| ~~Q5~~ ✅ | base 不画框，只出坐标；展示用 webui(SSE) + HTML 画框（D12） |
| ~~Q6~~ ✅ | 第一版跑通完整闭环：抓图 → tensor stream 发车 → 车 YOLO → webui 发浏览器 → HTML 画框 |

## 九、实施阶段详细方案

### 阶段 0：设备 Lua cap 注入口（前置，base）✅ 已实施

**文件架构**
```
pleiades-base/src/Orchestrator/
├── mod.rs                          # ✏️ 定义 DeviceCapability trait + Capabilities 加 device_caps 字段
└── core/
    └── branch_user.rs              # ✏️ spawn_lua_script 遍历调用 register_lua_caps
```

**涉及文件**
| 文件 | 改动 | 说明 |
|---|---|---|
| `pleiades-base/src/Orchestrator/mod.rs` | ✏️ | 定义 `DeviceCapability` trait + `Capabilities` 加 `device_caps: Vec<Arc<dyn DeviceCapability>>` |
| `pleiades-base/src/Orchestrator/core/branch_user.rs` | ✏️ | `spawn_lua_script` 里 register_* 后遍历调用 `register_lua_caps` |

**方法签名**
```rust
// Orchestrator/mod.rs：定义设备能力 trait（与 network/storage 的 trait object 同风格）
pub trait DeviceCapability: Send + Sync {
    fn register_lua_caps(&self, lua: &Lua) -> mlua::Result<()>;
}

pub struct Capabilities {
    // ...现有字段...
    pub device_caps: Vec<Arc<dyn DeviceCapability>>,
}

// Orchestrator/core/branch_user.rs（spawn_lua_script 内，register_*_caps 之后）
for device in &caps.device_caps {
    device.register_lua_caps(&lua)?;
}
```

**输入输出功能**
- `DeviceCapability` trait：设备能力接口，设备端实现 `register_lua_caps`，把自己设备的 cap（如 `camera.capture`）挂进 Lua
- `device_caps`：设备端（uav/ugv bootstrap）注入的设备能力列表（与 network/storage 的 trait object 同风格）
- 每次 `spawn_lua_script`（每个 exec 命令）遍历调用 `register_lua_caps`，保证每个新 Lua 实例都有设备 cap

**实施步骤**
1. `Orchestrator/mod.rs`：定义 `DeviceCapability` trait + `Capabilities` 加 `device_caps` 字段（默认空）
2. `Orchestrator/core/branch_user.rs`：`spawn_lua_script` 里，注册完内置 caps 后、执行脚本前，遍历调用 `register_lua_caps`
3. 验证：base 编译通过；阶段 1 的 `CameraDevice` 实现 `DeviceCapability` 后，exec 脚本能调 `camera.capture()`

---

### 阶段 1：Camera 适配（UAV 抓图）✅ 已实施（2026-09-08，Windows 实机验证通过）

**文件架构**
```
pleiades-base/src/Orchestrator/mod.rs                  # ✏️ 定义 DeviceCapability trait + Capabilities 加 device_caps（阶段 0）
pleiades-base/src/Orchestrator/core/branch_user.rs     # ✏️ spawn_lua_script 遍历调用 register_lua_caps（阶段 0）
pleiades-uav/src/
├── device/
│   ├── mod.rs                    # 🆕 新建（pub mod camera;）
│   └── camera/
│       └── mod.rs                # 🆕 CameraDevice + CameraCmd + 常驻线程 + impl DeviceCapability
├── bootstrap.rs                  # ✏️ 创建 CameraDevice + 作为 Arc<dyn DeviceCapability> 塞进 device_caps
├── config.rs                     # ✏️ 加 CameraConfig + [camera] 段 + fill_camera
└── uav/
    └── robot_handler.rs          # ✏️ UavInner 加字段 + start() 装配 + shutdown() 关闭
pleiades-uav/Cargo.toml           # ✏️ 加 nokhwa 依赖
```

**涉及文件**
| 文件 | 改动 | 说明 |
|---|---|---|
| `pleiades-base/src/Orchestrator/mod.rs` | ✏️ | 定义 `DeviceCapability` trait + Capabilities 加 `device_caps`（阶段 0） |
| `pleiades-base/src/Orchestrator/core/branch_user.rs` | ✏️ | spawn_lua_script 遍历调用 `register_lua_caps`（阶段 0） |
| `pleiades-uav/src/device/camera/mod.rs` | 🆕 | CameraDevice + CameraCmd + 常驻线程 + impl DeviceCapability |
| `pleiades-uav/src/device/mod.rs` | 🆕 | 新建，`pub mod camera;` |
| `pleiades-uav/src/bootstrap.rs` | ✏️ | 创建 CameraDevice + 塞进 device_caps |
| `pleiades-uav/src/config.rs` | ✏️ | CameraConfig + fill_camera |
| `pleiades-uav/src/uav/robot_handler.rs` | ✏️ | UavInner 加字段 + 装配/关闭 |
| `pleiades-uav/Cargo.toml` | ✏️ | `nokhwa = { default-features=false, features=["input-v4l"] }` |

**方法签名**
```rust
pub struct CameraDevice { config, cmd_tx: Option<mpsc::Sender<CameraCmd>>, join, cancel }
enum CameraCmd {
    Capture { reply: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>> },
    Shutdown,
}
impl CameraDevice {
    pub fn new(config: CameraConfig) -> Self;            // 只存配置，不开设备
    pub fn open(&mut self) -> Result<(), String>;        // spawn 常驻线程 + 打开摄像头（幂等）
    pub async fn capture(&self) -> Result<Vec<u8>, String>; // 发 Capture + await oneshot，返回 JPEG 字节
    pub fn close(&mut self);                             // 发 Shutdown + join（幂等）
}
impl Drop for CameraDevice { fn drop(&mut self) { self.close(); } }

// 设备 cap 注册（uav 侧，实现 base 的 DeviceCapability trait）
impl DeviceCapability for CameraDevice {
    fn register_lua_caps(&self, lua: &Lua) -> mlua::Result<()> { ... }
}
// Lua 侧：local jpeg = camera.capture()   -- async cap，返回 JPEG 字节
```

**输入输出功能**
- `new(config)`：输入 CameraConfig，输出空壳实例（未 open 时 capture 返回 Err）
- `open()`：输入无，输出 Ok；起 std::thread 常驻线程，nokhwa 打开 MJPG 摄像头
- `capture()`：输入无，输出 `Vec<u8>`（一帧 JPEG 字节）；async，oneshot + timeout 包超时
- `close()`：输入无，输出无；关线程释放设备

**实施步骤**
1. base 侧（阶段 0）：定义 `DeviceCapability` trait + Capabilities 加 `device_caps`，spawn_lua_script 遍历调用
2. `Cargo.toml` 加 nokhwa（`default-features=false, features=["input-v4l"]`）⚠️ 本地 registry 无缓存，需联网下载
3. 写 `device/camera/mod.rs`：CameraDevice + CameraCmd + 常驻线程 + `impl DeviceCapability for CameraDevice`
4. `device/mod.rs` 加 `pub mod camera;`
5. `config.rs` 加 CameraConfig（`enabled/path/width/height/timeout_ms`）+ fill_camera
6. `robot_handler.rs`：UavInner 加 `camera: CameraDevice`，`start()` 里 `[camera].enabled=true` 时 `open()`，`shutdown()` 里 `close()`
7. `bootstrap.rs`：创建 CameraDevice + 作为 `Arc<dyn DeviceCapability>` 塞进 `device_caps`
8. `cargo check -p pleiades-uav`；实机 `open → capture → 写 /tmp/test.jpg → 确认出图`

---

### 阶段 2：检测核心（YOLO 模块）

**文件架构**
```
pleiades-base/src/ML_Engine/
├── Yolo/
│   ├── mod.rs                    # 🆕 pub mod model; pub mod detector; re-export
│   ├── model.rs                  # 🆕 从 rust_yolo/src/model.rs 移植（755 行，import 改 candle_core）
│   ├── detector.rs               # 🆕 Yolo_Detector + 预处理/后处理 + Resolve_Yolo_Model_Path
│   └── coco_names.rs             # 🆕 COCO_NAMES: [&str; 80]
├── mod.rs                        # ✏️ pub mod yolo; + re-export
└── context.rs / gguf_model.rs 等  # 不动
pleiades-base/src/VM/capability_binding.rs  # ✏️ register_ml_caps 加 ml.yolo_new
pleiades-base/Cargo.toml          # ✏️ 加 image = "0.25"
```

**涉及文件**
| 文件 | 改动 | 说明 |
|---|---|---|
| `ML_Engine/Yolo/mod.rs` | 🆕 | 模块声明 + re-export |
| `ML_Engine/Yolo/model.rs` | 🆕 | 移植 candle yolo-v8 网络（YoloV8/DetectionHead/DFL 等） |
| `ML_Engine/Yolo/detector.rs` | 🆕 | Yolo_Detector + Detection + 预处理/后处理 |
| `ML_Engine/Yolo/coco_names.rs` | 🆕 | COCO 80 类名 |
| `ML_Engine/mod.rs` | ✏️ | `pub mod yolo;` + `pub use` |
| `VM/capability_binding.rs` | ✏️ | register_ml_caps 增 `ml.yolo_new` |
| `Cargo.toml` | ✏️ | 加 `image = "0.25"` |

**方法签名**
```rust
// detector.rs
pub struct Detection { pub class_index: usize, pub class_name: &'static str,
    pub xmin: f32, pub ymin: f32, pub xmax: f32, pub ymax: f32, pub confidence: f32 }
pub fn Resolve_Yolo_Model_Path(workspace: &Path, model_name: &str) -> Result<PathBuf>;  // 找 .safetensors
pub fn Yolo_Load_Model(path: &Path, device: &Device, which: &str) -> Result<YoloV8>;   // VarBuilder::from_mmaped_safetensors
pub struct Yolo_Detector { model: YoloV8, device: Device, class_names: &'static [&'static str] }
impl Yolo_Detector {
    pub fn new(device: &str, path: &Path, which: &str) -> Result<Self, String>;
    pub fn detect(&self, image_bytes: &[u8], conf_thr: f32, nms_thr: f32) -> Result<Vec<Detection>, String>;
}
fn Yolo_Preprocess(image_bytes: &[u8], device: &Device) -> Result<(Tensor, usize, usize), String>;  // 解码+resize_exact（保比例+32对齐）
fn Yolo_Postprocess(pred: &Tensor, orig_w, orig_h, conf, nms) -> Result<Vec<Detection>, String>;   // 过滤+NMS+缩放
fn detections_to_lua_table(lua: &Lua, dets: &[Detection]) -> mlua::Result<mlua::Table>;            // 独立转换函数
// 必须显式实现 UserData（det:detect 用 add_method）
impl mlua::UserData for Yolo_Detector {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("detect", ...);  // 薄胶水，业务在 detect()
    }
}
```

**Lua cap（薄胶水）**
```lua
local det = ml.yolo_new(device, path, which)     -- create_function：Yolo_Detector::new
local dets = det:detect(image_bytes, conf, nms)  -- add_method（&self，非 mut）
-- dets = { {class_index=0, class_name="person", xmin=.., xmax=.., confidence=0.91}, ... }
-- ⚠️ image_bytes 是二进制（JPEG），Lua 入参须用 mlua::String（不能用 String——String 要求合法 UTF-8，二进制会报错）
```

**输入输出功能**
- `Yolo_Detector::new(device, path, which)`：输入设备字符串+权重路径+型号("n"/"s"..)，输出加载好的 Detector
- `detect(image_bytes, conf, nms)`：输入 JPEG/PNG 字节 + 置信度/NMS 阈值，输出 Vec<Detection>（bbox 坐标）
- 内部：解码 → resize_exact（保比例+32 对齐，非 pad letterbox）→ forward（DFL 内建）→ 置信度过滤 → NMS（复用 candle_transformers）→ 坐标缩放回原图

**实施步骤**
1. 移植 model.rs（`candle::` → `candle_core::` 前缀，其余原样）
2. 写 detector.rs（Detection + Resolve_Yolo_Model_Path + Yolo_Detector + 预处理/后处理）
3. 写 coco_names.rs（80 类）
4. ML_Engine/mod.rs 加 `pub mod yolo;` + re-export
5. register_ml_caps 加 `ml.yolo_new`（薄胶水闭包）
6. Cargo.toml 加 image 依赖
7. `cargo check -p pleiades-base`；用本地图 + 真实权重验证 detect 出正确 bbox

---

### 阶段 3：图传输（tensor stream + U8）

**文件架构**
```
pleiades-base/src/ML_Engine/lua_tensor.rs   # ✏️ tensor_to_bytes/bytes_to_tensor 加 U8（dtype 2）
pleiades-base/src/VM/capability_binding.rs  # ✏️ 加 ml.tensor_from_u8_bytes
programs/user/
├── yolo_uav_send.lua          # 🆕 UAV：capture → U8 tensor → send_tensor（轮询）
└── yolo_car_detect.lua        # 🆕 车：accept → recv_tensor → detect → webui.send
```

**涉及文件**
| 文件 | 改动 | 说明 |
|---|---|---|
| `ML_Engine/lua_tensor.rs` | ✏️ | tensor_to_bytes/bytes_to_tensor 加 `DType::U8 => 2`；bytes_to_tensor 字节数改按 dtype 算 |
| `VM/capability_binding.rs` | ✏️ | 加 `ml.tensor_from_u8_bytes` + `ml.tensor_to_u8_bytes` |
| `programs/user/yolo_uav_send.lua` | 🆕 | UAV 发图脚本 |
| `programs/user/yolo_car_detect.lua` | 🆕 | 车收图检测脚本 |

**方法签名**
```rust
// lua_tensor.rs：tensor_to_bytes 的 dtype match 加分支
candle_core::DType::U8 => 2,     // 序列化 u8（零膨胀）
// ⚠️ bytes_to_tensor 现硬编码 expected_bytes = elem_count*4，须改成按 dtype 算（U8=1 / F32=U32=4）
// capability_binding.rs（入参用 mlua::String，不能 String——JPEG 是二进制非合法 UTF-8）
ml.tensor_from_u8_bytes(bytes: mlua::String, device: String) -> LuaTensor   // 图字节 → 1D U8 Tensor
ml.tensor_to_u8_bytes(tensor: mlua::AnyUserData) -> mlua::String            // U8 Tensor → 纯字节（to_vec1::<u8>，非带 header 的 to_bytes）
```

**Lua 用法**
```lua
-- UAV 发图（轮询：帧 N 发车 N%3）
local stream = caps.network.open_tensor_stream(car_peer, task_id)
local img_t = ml.tensor_from_u8_bytes(img_bytes, "cpu")
caps.network.send_tensor(stream, img_t, frame_no)

-- 车收图检测
local stream = caps.network.accept_tensor_stream(task_id, timeout)
local img_t, frame_no = caps.network.recv_tensor(stream, "cpu")
local jpeg = ml.tensor_to_u8_bytes(img_t)   -- 剥 header，取回纯 JPEG 字节
dets = det:detect(jpeg, 0.25, 0.45)         -- 喂给 detect
webui.send(jpeg, dets)                       -- 发图 + 坐标给浏览器
```

**输入输出功能**
- `tensor_from_u8_bytes(bytes, device)`：输入任意 u8 字节（JPEG）+ 设备，输出 1D U8 LuaTensor（零膨胀）；入参 mlua::String
- `tensor_to_u8_bytes(tensor)`：输入 U8 LuaTensor，输出纯字节（剥掉序列化 header），喂 det:detect
- `send_tensor(stream, tensor, offset)`：输入流 + U8 tensor + 帧号，输出无（fire-and-forget，现成 cap）
- `recv_tensor(stream, device)`：输入流 + 设备，输出 (LuaTensor, offset)，现成 cap

**实施步骤**
1. `lua_tensor.rs` 加 U8：`tensor_to_bytes` 加 `DType::U8 => 2`；`bytes_to_tensor` 加 U8 反序列化分支 + 字节数改按 dtype 算
2. `capability_binding.rs` 加 `ml.tensor_from_u8_bytes`（mlua::String → Tensor::from_vec u8 → LuaTensor）+ `ml.tensor_to_u8_bytes`（to_vec1::<u8> → 纯字节）
3. 写 `yolo_uav_send.lua`：capture → tensor_from_u8_bytes → open_tensor_stream → send_tensor 轮询
4. 写 `yolo_car_detect.lua`：accept → 循环 recv_tensor → tensor_to_u8_bytes → detect → webui.send
5. 两节点联调：传一张图，确认车收到完整字节并能 detect

---

### 阶段 4：展示（webui + HTML）

**文件架构**
```
pleiades-base/src/API/
├── webui.rs                   # 🆕 spawn_webui_server（SSE，仿 spawn_api_server）
└── mod.rs                     # ✏️ pub mod webui;
pleiades-base/src/VM/capability_binding.rs  # ✏️ 加 caps.webui.send
pleiades-base/src/Orchestrator/core/branch_user.rs  # ✏️ 加 webui 命令（仿 api 命令）
Tools/ 或独立目录
└── yolo_viewer/index.html     # 🆕 HTML 展示页（三车 SSE + canvas 画框 + FPS）
```

**涉及文件**
| 文件 | 改动 | 说明 |
|---|---|---|
| `API/webui.rs` | 🆕 | axum SSE 服务 + broadcast channel |
| `API/mod.rs` | ✏️ | `pub mod webui;` |
| `VM/capability_binding.rs` | ✏️ | 加 `webui.send` |
| `Orchestrator/core/branch_user.rs` | ✏️ | 加 `webui` 命令 |
| `yolo_viewer/index.html` | 🆕 | 浏览器展示页 |

**方法签名**
```rust
pub async fn spawn_webui_server() -> Result<u16, String>;   // Router(/stream SSE) + bind + tokio::spawn，返回端口
pub fn webui_publish(data: String);                          // 往 broadcast channel 发
// cap：caps.webui.send(img_bytes, dets)  → Rust 打包（图 base64 + bbox JSON）→ webui_publish(json)
// ⚠️ find_available_port 现为私有函数（API/server.rs），webui.rs 需复制一份或改 pub(crate)
```

**输入输出功能**
- `webui` 命令：输入无，输出端口号；起 SSE 服务（`/stream` 端点，`find_available_port`）
- `webui.send(img_bytes, dets)`：输入图字节 + bbox table，**Rust 侧打包**（图 base64 + bbox JSON，Lua 无 base64/json 能力）后推给浏览器
- HTML：`new EventSource(url)` 连车，`onmessage` 解析 → canvas 画框 + FPS 计数

**实施步骤**
1. 写 `API/webui.rs`：`Router::new().route("/stream", get(sse_handler))` + `broadcast::channel` + `find_available_port` + `tokio::spawn(axum::serve)`
2. `API/mod.rs` 加 `pub mod webui;`
3. `capability_binding.rs` 加 `webui.send`（薄胶水 → webui_publish）
4. `branch_user.rs` 加 `webui` 命令（仿 api 命令，spawn_webui_server + 显示端口）
5. 写 `yolo_viewer/index.html`（三车三 SSE 连接，三栏画框 + FPS）
6. 端到端联调：UAV 抓图 → 发车 → 车 YOLO → webui → 浏览器画框，跑通完整闭环

---

## 十、讨论记录

- 2026-09-06 与李永康讨论：
  - Q1 分支职责 → 定：YOLO 核心放 `ML_Engine/Yolo/`，无边界问题
  - 触发方式 → 定：走 `exec program`（Lua 脚本）体系，非 Rust robot 核心直调（D7）
  - Q2 分辨率 → 定：640
  - Q3 检测范围 → 定：第一版只 detect（80 类），pose 代码保留不暴露
  - 图传输 → 定：tensor stream（不用 send_data，其 Data 类型被 Network_Service 内部吞掉），lua_tensor 加 U8 dtype（D10/D11）
  - 展示 → 定：webui(SSE) + HTML 画框，各车各传（D12）
  - Q5 画框 → 定：base 不画框，展示层画（Q5 已定）
  - Q4 类别表 → 定：第一版硬编码 80 类
  - Q6 结果消费 → 定：第一版跑通完整闭环（抓图→发车→YOLO→发浏览器→画框）
- 2026-09-07 代码评审 + 修正：
  - 派子 agent 对照代码评审，发现 F1-F7 七类问题；主体决策 D7/D10/D12 均与代码严格吻合
  - F1 camera cap 跨 crate → 定：加「设备 cap 注入口」（阶段 0，Capabilities + spawn_lua_script），camera 留 uav
  - F2-F7（UserData / mlua::String / resize_exact / bytes_to_tensor 硬编码 / tensor_to_u8_bytes / webui.send 打包 / find_available_port / nokhwa 离线）→ 已逐条修正进文档
- 2026-09-08 实施 + 验证：
  - 阶段 0 实施完成（DeviceCapability trait 版，cargo check 通过），提交推送到 `robot_yolo` 分支
  - 真实权重验证：yolov8n（6.1MB）+ yolov8l（84MB）在 rust_yolo 检测 bike.jpg 成功，yolov8l 更准（多检出 dog/car）
  - 分支策略 → 定：robot_yolo 演示专用不合并；webui（阶段 4）不进正式分支；核心能力（阶段 0-3）归属待定
  - 阶段 1 实施完成（CameraDevice + DeviceCapability，Windows 实机 exec test_camera 验证通过）
  - 阶段 2 实施完成，与设计的三处偏差（均已在代码落地）：
    1. `ml.yolo_new(model_file, device)` 两参数（文档原设计 device/path/which 三参数）——权重文件名作参数，型号从文件名推断（n/s/m/l/x），权重放 Pleiades_Workspace/
    2. detect 返回**检测尺度（resize 后）坐标**，不缩放回原图（文档原设计缩放回原图）——省掉换算，与 rust_yolo 基准直接逐位对比
    3. 读写盘复用 storage 机制：`StorageReadHandle` 补 `read()`（对称阶段 1 的 `write()`），不新增 read_file_bytes cap
  - 验证：cargo check 通过；example + Lua `exec yolo_test` 端到端，yolov8n 22 框 / yolov8l 28 框，与 rust_yolo 基准逐位一致
