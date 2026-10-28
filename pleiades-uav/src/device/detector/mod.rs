//Presented by KeJi
//Created Date ： 2026-10-28
//Modified Date ： 2026-10-28

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! detector 设备（task_37 §三 Step 2）：只做「**帧到 → 框出**」，别的一概不做。
//!
//! 形态与 `mavlink` / `lidar` / `camera_*` 同构：自己的目录、自己的 tokio task、自己的状态与
//! 配置段，随 `DeviceHandler::start()` / `shutdown()` 走；`enabled = false` → **不 spawn、零线程**。
//!
//! ```text
//! 相机帧槽位（只读）                       本设备（唯一写者）
//! camera_uav::latest_frame() ──► FrameSource::Latest_Frame()
//!                                     │ 每 POLL_INTERVAL_MS 看一眼（同步、非阻塞）
//!                                     ▼
//!                        spawn_blocking(Yolo_Detector::detect)
//!                                     │
//!                                     ▼
//!                    Mutex<Option<Detections>> ──► latest_dets() / latest()
//! ```
//!
//! 三条硬边界（§三.6 判据 5）：
//! 1. **不进 `on_tick`**：本文件没有任何决策 / 寻路 / 树 / `ExecuteState` 代码；
//! 2. **不写 `RobotState`**：只 `try_read()` 位姿打日志（读锁忙就写「锁忙」，绝不阻塞控制链）；
//! 3. **不改相机**：帧源只是把相机**已有**的 `latest_frame()` 搬过来（Task 34「相机互不引用」不动）。
//!
//! 算法归属（§七-1 已定）：**不迁移** base 的 `ML_Engine/Yolo`，直接
//! `Yolo_Detector::new(model_file, device)` / `detect(jpeg, conf, nms)`；权重**加载一次常驻**。
//! 模型缺失 / 加载失败 → `spawn` 返回 `Err`，调用方 `warn` 后置 `None`（不拖垮进程）。
//!
//! ⚠️ 为什么 `pub mod detector` 声明在 `crate::uav::robot_handler` 而不是 `device/mod.rs`：
//! 本 step 的允许清单里没有 `pleiades-uav/src/device/mod.rs`（`tmp/task37/step.md:20-30`），
//! 故用 `#[path]` 从 handler 里声明（与 `sim_handler.rs:51-58` 声明 `device_cap` 同款解法）。
//! 规范路径因此是 `crate::uav::robot_handler::detector`。
//!
//! ## 位置估计（task_37 §四 Step 3）
//!
//! 每次推理完、打日志前，对每个框调一次 `projection::Projector::Project`：框底边中点（≈脚）
//! → 针孔反投影 → 机体系 → 世界 → 与 `z = ground_z` 求交 → 世界 `(x, y)`；
//! 不满足保护就**拒答**（只打原因，不写坐标）。估计结果拼在**同一行** `info!` 的检测项末尾，
//! 并随 `Detections` 一起覆盖进结果槽位（`latest_targets()`）。
//!
//! **位姿是「最新」不是「帧时刻」**（§四.3 允许的近似）：帧只有 `captured_at_ms`，`RobotState`
//! 不带时间戳、无法反查「那一帧的位姿」；sim 位姿 10 Hz（100 ms 一拍）、推理 ~100 ms，
//! 两者最多差一拍。按 `tmp/task37/prev_issues.md` 的约定，**不为此改 chassis / pose_ingest**
//! （sim 侧已有 `PoseHistory`，留给 Step 4 再做帧时刻对齐）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use pleiades_base::config::Ensure_Config;
use pleiades_base::ml_engine::yolo::Yolo_Detector;
use pleiades_base::robot::core::state::RobotState;

use crate::config::DetectorConfig;

// 位置估计（task_37 Step 3）：像素 → 世界 (x,y) 的**纯函数**子模块，无 I/O、无锁、无线程。
// 本文件是 `#[path]` 从 `uav/robot_handler.rs:44-45` 声明的，子模块按 mod.rs 所在目录解析
// → `src/device/detector/projection.rs`（rustc 最小样例已实测通过）。
// sim 侧同款（`sim/detector/projection.rs`，走 `sim/mod.rs:22` 的正规 `pub mod detector;`）。
pub mod projection;

/// detector 的帧 = `camera_uav` 的槽位帧（**零拷贝别名**：字段与语义完全一致，不再复制结构）
pub use crate::device::camera_uav::slot::Frame;

/// 轮询间隔（毫秒）：每 100 ms 看一眼帧槽位（10 Hz，与 `[camera_*].stream_fps` 默认值对齐）
///
/// 实际推理节奏 = `poll_ms + 推理耗时`（candle CPU 推理远慢于 100 ms）→ 不会空转刷 CPU。
pub const POLL_INTERVAL_MS: u64 = 100;

/// 无帧 `warn` 的限流间隔（毫秒）：相机没开时不要每 100 ms 刷一条
pub const NO_FRAME_WARN_EVERY_MS: u64 = 5_000;

/// 本 crate 帧时刻的语义（**只影响日志文案**）：真机 = 本机单调毫秒（`now_boot_ms()`，与位姿同源）
pub const FRAME_TS_SOURCE: &str = "本机单调";

/// 帧源：**同步、非阻塞**读最新帧（与三处相机的 `latest_frame()` 同语义）
pub trait FrameSource: Send + Sync {
    /// `None` = 相机未开 / 还没收到帧 / 已关流（调用方只 `warn`，不 panic）
    fn Latest_Frame(&self) -> Option<Frame>;
}

/// `camera_uav` 就是帧源：直接搬它的 `latest_frame()`（**不改相机文件**）
impl FrameSource for crate::device::camera_uav::Uav_Camera {
    fn Latest_Frame(&self) -> Option<Frame> {
        self.latest_frame()
    }
}

/// 一条检测（base 的 `Detection` 无 `Clone`，不能放进槽位 → 存自己的快照类型）
///
/// `#[allow(dead_code)]`：`class_index`（person 过滤用）本 step 不消费，但它是快照的公开字段，
/// 不为了消警告而删（删了 Step 3 又得加回来）。本仓同款容忍见 `camera_uav/mod.rs:334`。
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub class_index: usize,
    pub class_name: &'static str,
    pub conf: f32,
    pub xmin: f32,
    pub ymin: f32,
    pub xmax: f32,
    pub ymax: f32,
}

/// 一帧的推理结果（`frame_ts_ms` 供 Step 3 与位姿对齐；本 step 只写日志，不消费）
///
/// `#[allow(dead_code)]`：三个元数据字段本 step 只写不读（非测试构建下会报「fields never read」），
/// 它们正是 Step 3 的输入，不能删。
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Detections {
    pub frame_ts_ms: u64,
    pub frame_seq: u64,
    pub infer_ms: u64,
    pub dets: Vec<Detection>,
}

/// 一个世界坐标目标（task_37 §四：只做位置标记，不跟踪、不进决策链）
///
/// `#[allow(dead_code)]`：字段由 Step 4 的跟随消费；本 step 只写日志 + 槽位，只有单测读。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    /// 世界坐标（米）
    pub x: f32,
    pub y: f32,
    /// 水平距离（米，以机体原点为基准）
    pub range_m: f32,
    /// 世界系方位角（度）
    pub bearing_deg: f32,
    /// 与 `Detections` 同源（Step 4 算 `age` 用）
    pub frame_ts_ms: u64,
    pub frame_seq: u64,
}

/// 一帧的完整结果：检测框 + 由它算出的世界坐标目标（**同一次覆盖**，读者看不到半成品）
#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    dets: Detections,
    targets: Vec<Target>,
}

/// 计数快照（§三.6「本步不判 `stats()`」，只有单测读）
#[allow(dead_code)] // 同上：字段只写不读（观测用），本 step 只有单测读
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetectorStats {
    pub frames_seen: u64,
    pub frames_inferred: u64,
    pub infer_errors: u64,
    pub empty_polls: u64,
    pub dets: u64,
    pub last_frame_ts_ms: u64,
    pub last_infer_ms: u64,
}

/// 推理引擎（薄适配层；trait 的存在只为单测能塞假引擎）
pub trait DetectorEngine: Send + Sync {
    /// 跑一次推理：`jpeg` 原始字节（解码在引擎内部），返回原图像素坐标框
    fn Detect(&self, jpeg: &[u8], conf_thres: f32, nms_thres: f32) -> Result<Vec<Detection>, String>;
    /// 设备名（日志用，如 `cpu` / `fake`）
    fn Device_Name(&self) -> &str;
}

/// base `Yolo_Detector` 的薄适配（本 step 唯一的生产实现）
struct Yolo_Engine {
    inner: Yolo_Detector,
    device: String,
}

impl DetectorEngine for Yolo_Engine {
    fn Detect(&self, jpeg: &[u8], conf_thres: f32, nms_thres: f32) -> Result<Vec<Detection>, String> {
        let raw = self.inner.detect(jpeg, conf_thres, nms_thres)?;
        Ok(raw
            .into_iter()
            .map(|d| Detection {
                class_index: d.class_index,
                class_name: d.class_name,
                conf: d.confidence,
                xmin: d.xmin,
                ymin: d.ymin,
                xmax: d.xmax,
                ymax: d.ymax,
            })
            .collect())
    }

    fn Device_Name(&self) -> &str {
        &self.device
    }
}

/// 权重路径 = `<workspace_dir>/<model_file>`（与 `Yolo_Detector::new` 的规则**一致**）。
///
/// 单独成函数有两个目的：① 把 candle 的 mmap 报错换成一句可读的「模型文件不存在」；
/// ② **纯函数**（只吃 `workspace_dir`，不碰 `Ensure_Config`）→ 可单测且零副作用。
pub fn Resolve_Model_Path(workspace_dir: &Path, model_file: &str) -> Result<PathBuf, String> {
    let path = workspace_dir.join(model_file);
    if !path.is_file() {
        return Err(format!(
            "模型文件不存在：{}（[detector].model 只写文件名，权重放 <workspace_dir>/ 下，\
             且文件名须为 yolov8<size>.safetensors）",
            path.display()
        ));
    }
    Ok(path)
}

/// 加载引擎（**唯二碰模型文件的地方**；阻塞，只在 `spawn_blocking` 里调）
fn Load_Engine(model_file: &str, device: &str) -> Result<Arc<dyn DetectorEngine>, String> {
    let (config, _) = Ensure_Config().map_err(|e| format!("读取配置失败: {e}"))?;
    let path = Resolve_Model_Path(&config.workspace_dir(), model_file)?;
    info!(
        "[detector] 加载权重 {}（device={device}，加载一次常驻，不逐帧加载）",
        path.display()
    );
    let inner = Yolo_Detector::new(model_file, device)?;
    Ok(Arc::new(Yolo_Engine { inner, device: device.to_string() }))
}

/// 计数（task 写、`stats()` 读；全 `Relaxed`，只做观测）
#[derive(Default)]
struct Counters {
    frames_seen: AtomicU64,
    frames_inferred: AtomicU64,
    infer_errors: AtomicU64,
    empty_polls: AtomicU64,
    dets: AtomicU64,
    last_frame_ts_ms: AtomicU64,
    last_infer_ms: AtomicU64,
}

/// 推理 task 的入参（避免一长串位置参数）
struct Loop_Ctx {
    source: Arc<dyn FrameSource>,
    robot_state: Arc<RwLock<RobotState>>,
    engine: Arc<dyn DetectorEngine>,
    latest: Arc<Mutex<Option<Snapshot>>>,
    counters: Arc<Counters>,
    cancel: CancellationToken,
    conf: f32,
    nms: f32,
    poll_ms: u64,
    /// 位置估计器（`[detector.camera]` 解析结果；mount 基向量在 spawn 时算好）
    projector: projection::Projector,
}

/// detector 设备（`Send + Sync`，可整份塞进 `Option<Arc<...>>`）
pub struct DetectorDevice {
    /// 置信度阈值（`[detector].conf_thres`）
    conf: f32,
    /// NMS IoU 阈值（`[detector].nms_thres`）
    nms: f32,
    /// 引擎设备名（日志用）
    engine_name: String,
    /// 最新结果槽位（task 写、`latest()` / `latest_dets()` / `latest_targets()` 读；
    /// 检测框与世界坐标目标**同一次覆盖**，读者不会看到半成品）
    latest: Arc<Mutex<Option<Snapshot>>>,
    /// 计数
    counters: Arc<Counters>,
    /// 停止信号（`shutdown()` 只做 `cancel()`，与 `MavlinkDevice::shutdown` 同法）
    cancel: CancellationToken,
}

impl DetectorDevice {
    /// **对外唯一构造口**（§三.2 签名逐字）。`async` 只为把权重加载丢进 `spawn_blocking`。
    ///
    /// 失败语义（调用方一律 `warn` + 置 `None`，**不得**把 `Err` 透给 `start()`）：
    /// - `enabled = false` → `Err`（防御性：真正的门在 handler，见 `robot_handler.rs`）
    /// - 权重缺失 / 型号非法 / 设备名非法 → `Err`
    pub async fn spawn(
        cfg: DetectorConfig,
        source: Arc<dyn FrameSource>,
        robot_state: Arc<RwLock<RobotState>>,
    ) -> Result<Arc<Self>, String> {
        if !cfg.Enabled() {
            return Err("detector.enabled=false：不 spawn（零线程）".to_string());
        }
        let cfg_for_load = cfg.clone();
        let engine = tokio::task::spawn_blocking(move || {
            Load_Engine(cfg_for_load.Model(), cfg_for_load.Device())
        })
        .await
        .map_err(|e| format!("推理引擎加载 task 异常结束: {e}"))??;
        Self::Spawn_Tuned(cfg, source, robot_state, engine, POLL_INTERVAL_MS)
    }

    /// 内部构造口：引擎已就绪、轮询间隔可指定（生产走 [`POLL_INTERVAL_MS`]，单测走 5 ms）
    pub(crate) fn Spawn_Tuned(
        cfg: DetectorConfig,
        source: Arc<dyn FrameSource>,
        robot_state: Arc<RwLock<RobotState>>,
        engine: Arc<dyn DetectorEngine>,
        poll_ms: u64,
    ) -> Result<Arc<Self>, String> {
        if !cfg.Enabled() {
            return Err("detector.enabled=false：不 spawn（零线程）".to_string());
        }
        // 位置估计器（§四.4）：mount 基向量在这里一次算好，逐帧只做乘加
        let projector =
            projection::Projector::New(projection::ProjectionParams::From_Config(cfg.camera.as_ref()));
        let cancel = CancellationToken::new();
        let latest = Arc::new(Mutex::new(None));
        let counters = Arc::new(Counters::default());
        let device = Arc::new(Self {
            conf: cfg.Conf_Thres(),
            nms: cfg.Nms_Thres(),
            engine_name: engine.Device_Name().to_string(),
            latest: latest.clone(),
            counters: counters.clone(),
            cancel: cancel.clone(),
        });
        info!(
            "[detector] 已启动：device={} model={} conf={:.2} nms={:.2} 轮询={}ms",
            device.engine_name,
            cfg.Model(),
            device.conf,
            device.nms,
            poll_ms
        );
        info!(
            "[detector] 投影参数：fov={:.1}°({}) mount=({:.2},{:.2},{:.3}) m yaw={:.1}° pitch={:.1}° ground_z={:.2} max_range={:.1} m",
            projector.Params().fov_deg,
            projector.Params().fov_axis.As_Str(),
            projector.Params().mount_x,
            projector.Params().mount_y,
            projector.Params().mount_z,
            projector.Params().mount_yaw_deg,
            projector.Params().mount_pitch_deg,
            projector.Params().ground_z,
            projector.Params().max_range_m
        );
        tokio::spawn(Run_Loop(Loop_Ctx {
            source,
            robot_state,
            engine,
            latest,
            counters,
            cancel,
            conf: device.conf,
            nms: device.nms,
            poll_ms,
            projector,
        }));
        Ok(device)
    }

    /// 最新一帧结果（只读快照；无 → `None`）
    #[allow(dead_code)] // Step 4 的消费者（跟随要 `frame_ts` + 目标）；本 step 只有单测读
    pub fn latest(&self) -> Option<Detections> {
        Lock(&self.latest).as_ref().map(|s| s.dets.clone())
    }

    /// 最新一帧的检测框（只读快照；无结果 → 空 `Vec`，**不 panic**）
    #[allow(dead_code)] // Step 3/4 的消费者；本 step 只有单测读（判据只认日志）
    pub fn latest_dets(&self) -> Vec<Detection> {
        self.latest().map(|d| d.dets).unwrap_or_default()
    }

    /// 最新一帧**通过保护**的世界坐标目标（只读快照；无 → 空 `Vec`，**不 panic**）
    ///
    /// 拒答的框**不在这里**（§四.5：不输出坏点）。`range_m` 以机体原点为基准，
    /// `frame_ts_ms / frame_seq` 与 `Detections` 同源，供 Step 4 算 `age`。
    #[allow(dead_code)] // Step 4 的消费者（FollowLeaf 的目标来源）；本 step 只有单测读
    pub fn latest_targets(&self) -> Vec<Target> {
        Lock(&self.latest).as_ref().map(|s| s.targets.clone()).unwrap_or_default()
    }

    /// 计数快照（观测用，不参与任何决策）
    #[allow(dead_code)] // §三.6「明确不判 stats()」；保留给后续 task 与单测
    pub fn stats(&self) -> DetectorStats {
        let c = &self.counters;
        DetectorStats {
            frames_seen: c.frames_seen.load(Ordering::Relaxed),
            frames_inferred: c.frames_inferred.load(Ordering::Relaxed),
            infer_errors: c.infer_errors.load(Ordering::Relaxed),
            empty_polls: c.empty_polls.load(Ordering::Relaxed),
            dets: c.dets.load(Ordering::Relaxed),
            last_frame_ts_ms: c.last_frame_ts_ms.load(Ordering::Relaxed),
            last_infer_ms: c.last_infer_ms.load(Ordering::Relaxed),
        }
    }

    /// 停推理 task（**同步**：只 `cancel()`；可重复调用；不 join → 不阻塞 `shutdown()`）
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

/// 推理循环：看一眼槽位 → 有新帧就 `spawn_blocking` 推理 → 覆盖结果 + 打一行日志
async fn Run_Loop(ctx: Loop_Ctx) {
    let Loop_Ctx { source, robot_state, engine, latest, counters, cancel, conf, nms, poll_ms, projector } =
        ctx;
    // 上一帧的槽位帧号：同一帧不重复推理（相机 10 Hz、推理可能更慢，靠这条去重）
    let mut last_seq: Option<u64> = None;
    // 无帧 warn 的限流（`Instant` 单调，系统时钟回拨不影响）
    let mut next_no_frame_warn = Instant::now(); // 首轮立即 warn 一次
    // 位姿读不到 / 帧宽高非法时 `warn` 的限流（与无帧告警同节奏，5 s 一条）
    let mut next_proj_warn = Instant::now();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_millis(poll_ms)) => {}
        }

        // ① 取帧（同步、非阻塞）
        let Some(frame) = source.Latest_Frame() else {
            counters.empty_polls.fetch_add(1, Ordering::Relaxed);
            if Instant::now() >= next_no_frame_warn {
                next_no_frame_warn = Instant::now() + Duration::from_millis(NO_FRAME_WARN_EVERY_MS);
                warn!("[detector] 帧源无帧（相机未开 / 还没收到帧）：本轮跳过，不推理、不空转");
            }
            continue;
        };
        if last_seq == Some(frame.seq) {
            continue; // 同一帧：不重复推理（相机比推理快时这条是常态）
        }
        last_seq = Some(frame.seq);
        counters.frames_seen.fetch_add(1, Ordering::Relaxed);

        // ② 推理：candle 的 CPU 计算是纯同步阻塞 → 必须逃出 tokio 的协作调度
        let jpeg = frame.jpeg.clone();
        let engine_for_task = engine.clone();
        let started = Instant::now();
        let outcome =
            tokio::task::spawn_blocking(move || engine_for_task.Detect(jpeg.as_ref(), conf, nms)).await;
        let infer_ms = started.elapsed().as_millis() as u64;
        let dets = match outcome {
            Ok(Ok(dets)) => dets,
            Ok(Err(e)) => {
                counters.infer_errors.fetch_add(1, Ordering::Relaxed);
                warn!(
                    "[detector] 推理失败（frame_ts={}ms seq={}）：{e}",
                    frame.captured_at_ms, frame.seq
                );
                continue;
            }
            Err(e) => {
                counters.infer_errors.fetch_add(1, Ordering::Relaxed);
                warn!(
                    "[detector] 推理 task 异常结束（frame_ts={}ms seq={}）：{e}",
                    frame.captured_at_ms, frame.seq
                );
                continue;
            }
        };

        // ③ 位姿（**只读、非阻塞**：读锁忙就写「锁忙」，绝不阻塞控制链）
        //
        //    ⚠️ 取的是「推理结束那一刻的最新位姿」，不是「帧曝光时刻的位姿」（§四.3 的近似）：
        //    帧只有 `captured_at_ms`、`RobotState` 不带时间戳，无法反查那一帧的位姿；
        //    sim 位姿 10 Hz（100 ms 一拍）、推理约 100 ms → 最多差一拍。
        //    按 tmp/task37/prev_issues.md 的约定，**不为此改 chassis / pose_ingest**。
        let pose_now = match robot_state.try_read() {
            Ok(rs) => Some(projection::Pose { x: rs.x, y: rs.y, z: rs.z, yaw: rs.attitude.yaw }),
            Err(_) => None,
        };
        let pose = match &pose_now {
            Some(p) => format!(
                "({:.2},{:.2},{:.2},yaw={:.1}°)",
                p.x,
                p.y,
                p.z,
                p.yaw.to_degrees()
            ),
            None => "(读锁忙)".to_string(),
        };

        // ④ 位置估计（§四.2）：框底边中点 → 针孔反投影 → 机体系 → 世界 → 与 z=ground_z 求交。
        //    **只多算一个纯函数**：不做跟踪、不进 on_tick、不写 RobotState（§四.7）。
        //    拒答只记原因（`Reject::Reason()`），**绝不往槽位/日志写坏坐标**（§四.5）。
        let mut targets: Vec<Target> = Vec::new();
        let mut guesses: Vec<Result<projection::Ground, projection::Reject>> = Vec::new();
        let intr = projector.Intrinsics_For(frame.width, frame.height);
        match (&pose_now, &intr) {
            (Some(p), Some(intr)) => {
                for d in &dets {
                    let (u, v) = projection::Bottom_Center(d.xmin, d.xmax, d.ymax);
                    let one = projector.Project(intr, p, u, v);
                    if let Ok(g) = &one {
                        targets.push(Target {
                            x: g.x,
                            y: g.y,
                            range_m: g.range_m,
                            bearing_deg: g.bearing_deg,
                            frame_ts_ms: frame.captured_at_ms,
                            frame_seq: frame.seq,
                        });
                    }
                    guesses.push(one);
                }
            }
            (None, _) => {
                if Instant::now() >= next_proj_warn {
                    next_proj_warn = Instant::now() + Duration::from_millis(NO_FRAME_WARN_EVERY_MS);
                    warn!("[detector] 位置估计跳过（seq={}）：位姿读锁忙（本帧只打框）", frame.seq);
                }
            }
            (_, None) => {
                if Instant::now() >= next_proj_warn {
                    next_proj_warn = Instant::now() + Duration::from_millis(NO_FRAME_WARN_EVERY_MS);
                    warn!(
                        "[detector] 位置估计跳过（seq={}）：帧宽高非法（{}x{}）",
                        frame.seq, frame.width, frame.height
                    );
                }
            }
        }

        // ⑤ 日志（§三.6 判据 1+2 的格式**一字不改**；§四 Step 3 在每个框后面追加估计结果）
        let ts = frame.captured_at_ms;
        let mut line = format!(
            "[detector] frame_ts={ts}ms({FRAME_TS_SOURCE}) seq={} {}x{} 推理={infer_ms}ms dets={} pose={pose}",
            frame.seq,
            frame.width,
            frame.height,
            dets.len()
        );
        for (i, d) in dets.iter().enumerate() {
            line.push_str(&format!(
                " | {} conf={:.2} bbox=({},{})-({},{})",
                d.class_name,
                d.conf,
                d.xmin as i32,
                d.ymin as i32,
                d.xmax as i32,
                d.ymax as i32
            ));
            let (u, v) = projection::Bottom_Center(d.xmin, d.xmax, d.ymax);
            match guesses.get(i) {
                Some(Ok(g)) => line.push_str(&format!(
                    " → 世界=({:.2},{:.2}) 距离={:.2}m 方位={:+.1}° 相对机头={:+.1}° 判定=采纳",
                    g.x, g.y, g.range_m, g.bearing_deg, g.rel_bearing_deg
                )),
                Some(Err(r)) => line.push_str(&format!(
                    " 像素=({},{}) 判定=拒答（{}） 世界=-",
                    u as i32,
                    v as i32,
                    r.Reason()
                )),
                None => {}
            }
        }
        info!("{line}");

        // ⑥ 覆盖结果（**只有这一个写者**）：检测框与世界坐标目标同一次覆盖
        counters.frames_inferred.fetch_add(1, Ordering::Relaxed);
        counters.dets.fetch_add(dets.len() as u64, Ordering::Relaxed);
        counters.last_frame_ts_ms.store(ts, Ordering::Relaxed);
        counters.last_infer_ms.store(infer_ms, Ordering::Relaxed);
        *Lock(&latest) = Some(Snapshot {
            dets: Detections { frame_ts_ms: ts, frame_seq: frame.seq, infer_ms, dets },
            targets,
        });
    }

    info!("[detector] 推理 task 已停（cancel）");
}

/// 取锁，毒化（某 task panic 过）时恢复内部数据而不是二次 panic（与 `camera_uav/slot.rs:79` 同法）
fn Lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 假件（不碰模型、不碰配置、零 I/O）

    /// 假帧源：按 cursor 逐次喂 `frames`，喂完后**反复返回最后一帧**（用来验「同一帧不重复推理」）
    struct Fake_Source {
        frames: Vec<Frame>,
        cursor: Mutex<usize>,
        polls: AtomicU64,
    }

    impl Fake_Source {
        fn New(frames: Vec<Frame>) -> Arc<Self> {
            Arc::new(Self { frames, cursor: Mutex::new(0), polls: AtomicU64::new(0) })
        }
    }

    impl FrameSource for Fake_Source {
        fn Latest_Frame(&self) -> Option<Frame> {
            self.polls.fetch_add(1, Ordering::Relaxed);
            if self.frames.is_empty() {
                return None;
            }
            let mut cursor = Lock(&self.cursor);
            let index = (*cursor).min(self.frames.len() - 1);
            let frame = self.frames[index].clone();
            *cursor += 1;
            Some(frame)
        }
    }

    /// 假引擎：按 JPEG 首字节答不同的框（`b'A'` → 1 条 person，`b'B'` → person + dog）
    #[derive(Default)]
    struct Fake_Engine {
        calls: AtomicU64,
    }

    impl DetectorEngine for Fake_Engine {
        fn Detect(&self, jpeg: &[u8], _conf: f32, _nms: f32) -> Result<Vec<Detection>, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(match jpeg.first().copied().unwrap_or(0) {
                b'A' => vec![Det_Of("person", 0.90, 10.0, 20.0, 30.0, 40.0)],
                b'B' => vec![
                    Det_Of("person", 0.80, 11.0, 21.0, 31.0, 41.0),
                    Det_Of("dog", 0.50, 1.0, 2.0, 3.0, 4.0),
                ],
                _ => Vec::new(),
            })
        }

        fn Device_Name(&self) -> &str {
            "fake"
        }
    }

    /// 假引擎②：固定给一个**画面下半部**的 person 框（用来验「通过保护 → 出目标」）
    #[derive(Default)]
    struct Fake_Person_Engine {
        calls: AtomicU64,
    }

    impl DetectorEngine for Fake_Person_Engine {
        fn Detect(&self, _jpeg: &[u8], _conf: f32, _nms: f32) -> Result<Vec<Detection>, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(vec![Det_Of("person", 0.90, 300.0, 300.0, 340.0, 390.0)])
        }

        fn Device_Name(&self) -> &str {
            "fake_person"
        }
    }

    fn Det_Of(class_name: &'static str, conf: f32, xmin: f32, ymin: f32, xmax: f32, ymax: f32) -> Detection {
        Detection {
            class_index: if class_name == "person" { 0 } else { 16 },
            class_name,
            conf,
            xmin,
            ymin,
            xmax,
            ymax,
        }
    }

    fn Frame_Of(marker: u8, seq: u64, captured_at_ms: u64) -> Frame {
        Frame {
            jpeg: Arc::from(vec![marker; 8]),
            width: 640,
            height: 480,
            seq,
            captured_at_ms,
        }
    }

    fn Robot_State_For_Test() -> Arc<RwLock<RobotState>> {
        Arc::new(RwLock::new(RobotState::default()))
    }

    fn Enabled_Cfg() -> DetectorConfig {
        DetectorConfig { enabled: Some(true), ..DetectorConfig::default() }
    }

    /// 等计数到 `want`（最多 ~2 s；不用 unwrap/expect，只 assert）
    async fn Wait_For(counter: &AtomicU64, want: u64, why: &str) {
        for _ in 0..1_000 {
            if counter.load(Ordering::Relaxed) >= want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(counter.load(Ordering::Relaxed) >= want, "等待超时：{why}");
    }

    // ---------------- 判据（step.md:18 的四条 + 三条守卫）

    /// 判据①：无帧**不 panic**、不推理、只记空轮（warn 限流，不会刷屏）
    #[tokio::test]
    async fn test_no_frame_is_not_panic() {
        let source = Fake_Source::New(Vec::new());
        let engine = Arc::new(Fake_Engine::default());
        let device = match DetectorDevice::Spawn_Tuned(
            Enabled_Cfg(),
            source.clone(),
            Robot_State_For_Test(),
            engine.clone(),
            5,
        ) {
            Ok(d) => d,
            Err(e) => {
                assert!(false, "Spawn_Tuned 应成功：{e}");
                return;
            }
        };
        tokio::time::sleep(Duration::from_millis(60)).await;

        assert!(device.latest_dets().is_empty(), "无帧时不得有任何框");
        assert!(device.latest().is_none(), "无帧时结果槽位保持 None");
        assert_eq!(device.stats().frames_seen, 0, "无帧不得计为「见到新帧」");
        assert!(device.stats().empty_polls > 0, "空轮必须被计数（证明确实在跑）");
        assert_eq!(engine.calls.load(Ordering::Relaxed), 0, "无帧绝不调推理");
        device.shutdown();
    }

    /// 判据②：`dets` 覆盖（新帧结果**整体覆盖**旧帧，`frame_ts` 跟着新帧走）
    #[tokio::test]
    async fn test_dets_overwrite_each_frame() {
        let source = Fake_Source::New(vec![Frame_Of(b'A', 1, 1_000), Frame_Of(b'B', 2, 2_000)]);
        let engine = Arc::new(Fake_Engine::default());
        let device = match DetectorDevice::Spawn_Tuned(
            Enabled_Cfg(),
            source.clone(),
            Robot_State_For_Test(),
            engine.clone(),
            5,
        ) {
            Ok(d) => d,
            Err(e) => {
                assert!(false, "Spawn_Tuned 应成功：{e}");
                return;
            }
        };
        Wait_For(&engine.calls, 2, "两帧都要被推理").await;

        let latest = match device.latest() {
            Some(l) => l,
            None => {
                assert!(false, "推理过就必须有结果");
                return;
            }
        };
        assert_eq!(latest.frame_seq, 2, "必须是**第二帧**的结果（覆盖式）");
        assert_eq!(latest.frame_ts_ms, 2_000, "frame_ts 必须取自第二帧");
        assert_eq!(latest.dets.len(), 2, "第二帧 2 条：旧帧的 1 条必须被整体覆盖");
        assert_eq!(latest.dets[0].class_name, "person");
        assert_eq!(latest.dets[0].conf, 0.80);
        assert_eq!(latest.dets[1].class_name, "dog");
        assert_eq!(device.latest_dets().len(), 2);
        assert_eq!(device.stats().frames_inferred, 2);
        device.shutdown();
    }

    /// 守卫：同一帧（槽位帧号不变）**不重复推理**
    #[tokio::test]
    async fn test_same_frame_not_reinferred() {
        let source = Fake_Source::New(vec![Frame_Of(b'A', 7, 7_000)]);
        let engine = Arc::new(Fake_Engine::default());
        let device = match DetectorDevice::Spawn_Tuned(
            Enabled_Cfg(),
            source.clone(),
            Robot_State_For_Test(),
            engine.clone(),
            5,
        ) {
            Ok(d) => d,
            Err(e) => {
                assert!(false, "Spawn_Tuned 应成功：{e}");
                return;
            }
        };
        Wait_For(&engine.calls, 1, "第一帧要推理").await;
        tokio::time::sleep(Duration::from_millis(60)).await; // 期间槽位反复返回同一 seq
        assert_eq!(
            engine.calls.load(Ordering::Relaxed),
            1,
            "同一 seq 不得重复推理（seq 是去重键）"
        );
        assert!(source.polls.load(Ordering::Relaxed) > 5, "但轮询必须照常进行");
        device.shutdown();
    }

    /// 判据③：`enabled = false` → **不 spawn**（Err + 零线程 + 一帧都没读）
    #[tokio::test]
    async fn test_disabled_does_not_spawn() {
        let cfg = DetectorConfig::default(); // 缺 `enabled` → 默认 false
        assert!(!cfg.Enabled(), "[detector].enabled 缺省必须是 false");
        let source = Fake_Source::New(vec![Frame_Of(b'A', 1, 1_000)]);
        let engine: Arc<dyn DetectorEngine> = Arc::new(Fake_Engine::default());
        let result =
            DetectorDevice::Spawn_Tuned(cfg, source.clone(), Robot_State_For_Test(), engine, 5);
        assert!(result.is_err(), "enabled=false 时 spawn 必须返回 Err（不 spawn）");
        if let Err(e) = result {
            assert!(e.contains("enabled=false"), "错误文案必须点明开关：{e}");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            source.polls.load(Ordering::Relaxed),
            0,
            "enabled=false 不得有 task 去读帧（零线程的观测证据）"
        );
    }

    /// 判据④（前半）：**模型缺失**只是 `Err`（调用方 `warn` + 置 None），不 panic、不 abort
    #[test]
    fn test_missing_model_is_err_not_panic() {
        let dir = std::env::temp_dir().join(format!(
            "task37_detector_no_model_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let err = Resolve_Model_Path(&dir, "yolov8n.safetensors");
        assert!(err.is_err(), "空目录里没有权重 → 必须是 Err");
        if let Err(message) = err {
            assert!(message.contains("模型文件不存在"), "错误要可读：{message}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 判据④（后半）：权重放对了 → 路径解析成功（<workspace_dir>/<model_file>，文件名原样）
    #[test]
    fn test_resolve_model_path_ok() {
        let dir = std::env::temp_dir().join(format!(
            "task37_detector_model_ok_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("yolov8n.safetensors");
        assert!(std::fs::write(&file, b"stub").is_ok(), "写占位文件失败（测试环境异常）");
        match Resolve_Model_Path(&dir, "yolov8n.safetensors") {
            Ok(path) => assert_eq!(path, file, "路径必须是 <workspace_dir>/<model_file>"),
            Err(e) => assert!(false, "文件存在时必须 Ok：{e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 守卫：`shutdown()` 可重复调用，且停掉之后**不再读帧**
    #[tokio::test]
    async fn test_shutdown_stops_polling() {
        let source = Fake_Source::New(vec![Frame_Of(b'A', 1, 1_000)]);
        let engine = Arc::new(Fake_Engine::default());
        let device = match DetectorDevice::Spawn_Tuned(
            Enabled_Cfg(),
            source.clone(),
            Robot_State_For_Test(),
            engine,
            5,
        ) {
            Ok(d) => d,
            Err(e) => {
                assert!(false, "Spawn_Tuned 应成功：{e}");
                return;
            }
        };
        Wait_For(&source.polls, 1, "先让它跑起来").await;
        device.shutdown();
        device.shutdown(); // 幂等
        tokio::time::sleep(Duration::from_millis(20)).await;
        let after = source.polls.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(
            source.polls.load(Ordering::Relaxed),
            after,
            "shutdown 之后不得再有轮询（task 已退出）"
        );
    }

    /// 判据（§四.6.4）：通过保护的框 → `latest_targets()` 有世界坐标（pitch=-15°、2.5 m 高、v=390）
    #[tokio::test]
    async fn test_targets_written_for_lower_half_box() {
        let source = Fake_Source::New(vec![Frame_Of(b'A', 3, 3_000)]);
        let engine = Arc::new(Fake_Person_Engine::default());
        let mut rs = RobotState::default();
        rs.z = 2.5;
        let state = Arc::new(RwLock::new(rs));
        let device = match DetectorDevice::Spawn_Tuned(Enabled_Cfg(), source, state, engine.clone(), 5) {
            Ok(d) => d,
            Err(e) => {
                assert!(false, "Spawn_Tuned 应成功：{e}");
                return;
            }
        };
        Wait_For(&engine.calls, 1, "一帧要推理").await;
        let targets = device.latest_targets();
        assert_eq!(targets.len(), 1, "下半部的 person 框必须产出一个目标");
        assert!((targets[0].x - 3.384).abs() < 0.05, "x={}", targets[0].x);
        assert!(targets[0].y.abs() < 0.05, "y={}", targets[0].y);
        assert!((targets[0].range_m - 3.384).abs() < 0.05, "range={}", targets[0].range_m);
        assert_eq!(targets[0].frame_seq, 3);
        assert_eq!(targets[0].frame_ts_ms, 3_000);
        assert_eq!(device.latest_dets().len(), 1, "框本身照旧在");
        device.shutdown();
    }

    /// 判据（§四.5）：被拒答的框**不进目标槽位**（不输出坏点），但框本身照旧
    #[tokio::test]
    async fn test_no_target_when_rejected() {
        let source = Fake_Source::New(vec![Frame_Of(b'A', 4, 4_000)]); // 假引擎给的框在画面上半部
        let engine = Arc::new(Fake_Engine::default());
        let device = match DetectorDevice::Spawn_Tuned(
            Enabled_Cfg(),
            source,
            Robot_State_For_Test(),
            engine.clone(),
            5,
        ) {
            Ok(d) => d,
            Err(e) => {
                assert!(false, "Spawn_Tuned 应成功：{e}");
                return;
            }
        };
        Wait_For(&engine.calls, 1, "一帧要推理").await;
        assert_eq!(device.latest_dets().len(), 1, "框必须照旧输出（Step 2 判据不回归）");
        assert!(device.latest_targets().is_empty(), "上半部的框必须被拒答，不得写目标");
        device.shutdown();
    }
}
