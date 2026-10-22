//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 传感器层（task_31 §五.3 / §五.4.7）：trait + 按配置构造的工厂 + 连接端点 + 限流日志
//!
//! 认知前提（§五.1）：**写 `robot.grid` 的是传感器，不是决策链**。每个传感器自带自己的
//! 感知 task（lidar 的建图 task 由 [`lidar::LidarSensor::Spawn_Mapping`] 拉起），
//! 与真机 `LidarDevice::spawn(…, Some(SlamContext))` 内部 `spawn_slam` 同构。
//!
//! 三个角色：
//! - [`Sensor`]：最小接口（本 step 只有 `lidar_2d`；相机以后加同一 trait）
//! - [`SensorSet`]：`[sim_world.sensor]`（已过 catalog 过滤）→ 实例，按 slot 索引，统一 `Shutdown`
//! - [`SensorEndpoint`]：**一条传感器连接 = 一个端点**，连接收包 task 持有 `(slot, sensor_id)`，
//!   二进制帧就地投递（校验帧头 `sensor_id`），不进 supervisor 的 mpsc（R7）
//!
//! 限流日志 [`Warn_Throttled`] 是全 step 共用的唯一实现：同 key 每 `period_ms` 最多一条。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use tracing::{debug, warn};

pub mod lidar;

use crate::sim::sensors::lidar::frame;
use crate::sim::sensors::lidar::mapping_task::MappingCtx;
use crate::sim::sensors::lidar::{LidarParams, LidarSensor};
use crate::world_link::config::ResolvedSensor;

// ================================================================ 限流日志

/// 同 key 每 `period_ms` 最多一条（纯同步，不持锁跨 await）
///
/// 实现：`OnceLock<Mutex<HashMap<key, Instant>>>`；`Instant` 是进程内单调钟。
/// 单测用不同 key 互相隔离。
pub fn Should_Log(key: &'static str, period_ms: u64) -> bool {
    static LAST: OnceLock<Mutex<HashMap<&'static str, Instant>>> = OnceLock::new();
    let table = LAST.get_or_init(|| Mutex::new(HashMap::new()));
    // 限流表只是日志节流，锁中毒也不该拖垮收包 task：直接取回内层数据
    let mut g = match table.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    match g.get(key) {
        Some(last) if now.duration_since(*last).as_millis() < period_ms as u128 => false,
        _ => {
            g.insert(key, now);
            true
        }
    }
}

/// 限流 warning：`Should_Log` 通过才求值 `msg`（闭包内的 `format!` 不进热路径）
pub fn Warn_Throttled(key: &'static str, period_ms: u64, msg: impl FnOnce() -> String) {
    if Should_Log(key, period_ms) {
        warn!("{}", msg());
    }
}

// ================================================================ Sensor trait

/// 传感器最小接口（本 step 只有 `lidar_2d`；相机以后加同一 trait）
///
/// 实现者必须**同步**处理一帧（`try_write` 抢不到就丢）：收包 task 只做同步调用，
/// 不 `await` → 不阻塞心跳 poll（`world_link/link.rs` 模块头的纪律）。
pub trait Sensor: Send + Sync {
    /// slot = `[sim_world.sensor].slot`，也是世界 `Sensors/<slot>` 的节点名
    fn Slot(&self) -> &str;
    /// 一帧二进制进来（解析失败由实现自己限流告警）
    fn On_Binary(&self, bytes: &[u8]);
    /// 停止本传感器持有的 task（建图 / 后续急停）
    fn Shutdown(&self);
}

// ================================================================ 传感器集合（工厂）

/// `[sim_world.sensor]`（已过 catalog 过滤）→ 传感器实例；按 slot 索引
pub struct SensorSet {
    all: Vec<Arc<dyn Sensor>>,
    by_slot: HashMap<String, Arc<dyn Sensor>>,
    /// 类型化 lidar 句柄（按 `[sim_world.sensor]` 声明顺序）：急停（task_31 Step 6）要 `Get_Scan()`，
    /// 而 `Arc<dyn Sensor>` 把 `Get_Scan()` 擦掉了 —— 这里**多留一份同一个 `Arc` 实例**，
    /// 不改任何既有字段/签名/行为（数据零拷贝分歧）
    lidars: Vec<Arc<LidarSensor>>,
}

impl SensorSet {
    /// 只认 `lidar_2d`；未知名字 → `Err`（Step 2 的 catalog 已先拦，这里只是兜底）
    ///
    /// 构造成功时每个 lidar 已经拉起了自己的建图 task（200 ms，位姿不新鲜时空转）；
    /// 失败时**先** `Shutdown()` 已经建好的传感器再 `Err`（R18：不留悬空 task）。
    pub fn Build(decls: &[ResolvedSensor], ctx: &MappingCtx) -> Result<Self, String> {
        let mut all: Vec<Arc<dyn Sensor>> = Vec::new();
        let mut by_slot: HashMap<String, Arc<dyn Sensor>> = HashMap::new();
        let mut lidars: Vec<Arc<LidarSensor>> = Vec::new();

        for decl in decls {
            match decl.sensor.trim() {
                "lidar_2d" => {
                    let params = LidarParams::From_Json(&decl.params);
                    // 外参存两份：建图 task 用 `sctx.mount`（下面），急停用传感器自带的这份。
                    // 同源同值（都来自 `decl.mount`），保证「急停标的格 = 建图标出的格」。
                    let sensor =
                        Arc::new(LidarSensor::New(&decl.slot, params).With_Mount(decl.mount));
                    // 每个传感器带**自己的**外参进建图 task（世界侧雷达从挂载点起算）
                    let mut sctx = ctx.clone();
                    sctx.mount = decl.mount;
                    sensor.Spawn_Mapping(sctx);
                    // 类型化句柄与擦除后的句柄是**同一个实例**（急停读的扫描 = 建图读的扫描）
                    lidars.push(sensor.clone());
                    let erased: Arc<dyn Sensor> = sensor;
                    by_slot.insert(decl.slot.clone(), erased.clone());
                    all.push(erased);
                }
                other => {
                    // 兜底：catalog 已经拦过；这里保证"不建半成品传感器集合"（先停已建好的，R18）
                    Self { all, by_slot, lidars }.Shutdown();
                    return Err(format!(
                        "未知传感器 '{other}'（slot='{}'）：本 crate 只实现 lidar_2d",
                        decl.slot
                    ));
                }
            }
        }

        Ok(Self { all, by_slot, lidars })
    }

    /// 按 slot 取传感器（连接任务用它把连接和传感器绑在一起）
    pub fn Get(&self, slot: &str) -> Option<Arc<dyn Sensor>> {
        self.by_slot.get(slot).cloned()
    }

    /// 全部 lidar（声明顺序）。急停取哪一个由 `sim_handler::Select_Lidar` 决定（§八.5.2 注）
    pub fn Lidars(&self) -> &[Arc<LidarSensor>] {
        &self.lidars
    }

    /// 全部传感器（`shutdown()` 逐个 `Shutdown`，也是"所有权"唯一持有处之一）
    pub fn All(&self) -> &[Arc<dyn Sensor>] {
        &self.all
    }

    /// 传感器个数（`start()` 的 `[world] 传感器就绪 slots=N` 日志）
    pub fn Len(&self) -> usize {
        self.all.len()
    }

    /// 逐个 `sensor.Shutdown()`（`SimDeviceHandler::shutdown` 调）
    pub fn Shutdown(&self) {
        for s in &self.all {
            s.Shutdown();
        }
    }
}

// ================================================================ 连接端点

/// `sensor_id` 尚未由 `sensor_ready` 下发的哨兵值
const SENSOR_ID_UNSET: u32 = u32::MAX;

/// **一条传感器连接 = 一个端点**：连接任务持有 `(slot, sensor_id)`，二进制帧直接交给该 slot 的传感器
///
/// 收包 task 里就地投递（不进 supervisor 的 mpsc）：`LinkEvent::Binary` → 本类型 →
/// [`Sensor::On_Binary`]，只做同步调用（`try_write`），不 `await`（R7）。
pub struct SensorEndpoint {
    slot: String,
    sensor: Arc<dyn Sensor>,
    /// `sensor_ready` 下发值；[`SENSOR_ID_UNSET`] = 尚未就绪
    sensor_id: AtomicU32,
    /// 已投递帧数（5 s 统计用）
    frames: AtomicU64,
}

impl SensorEndpoint {
    pub fn New(sensor: Arc<dyn Sensor>) -> Arc<Self> {
        Arc::new(Self {
            slot: sensor.Slot().to_string(),
            sensor,
            sensor_id: AtomicU32::new(SENSOR_ID_UNSET),
            frames: AtomicU64::new(0),
        })
    }

    /// slot（= 连接身份 `ConnKind::Sensor(slot)`）
    pub fn Slot(&self) -> &str {
        &self.slot
    }

    /// `sensor_ready` 命中后由 supervisor 立即写（早于任何稳态收包）
    pub fn Set_Sensor_Id(&self, id: u16) {
        self.sensor_id.store(id as u32, Ordering::Relaxed);
    }

    /// 已投递帧数
    pub fn Frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// 连接收包 task 的入口
    ///
    /// | 情况 | 行为 |
    /// |---|---|
    /// | `sensor_ready` 还没被 supervisor 处理 | `debug` + 丢弃（μs 级竞态，最多丢 1 帧，R6） |
    /// | 帧头 `sensor_id` ≠ 登记值 | 限流 warning + 丢弃 |
    /// | 相符，或帧短到取不到 id | `frames += 1` → [`Sensor::On_Binary`]（短包由传感器统一报错误分支） |
    pub fn Push_Binary(&self, bytes: &[u8]) {
        let want = self.sensor_id.load(Ordering::Relaxed);
        if want == SENSOR_ID_UNSET {
            debug!(
                "[world] {} 二进制帧在 sensor_ready 之前到达（{} B），丢弃",
                self.Slot(),
                bytes.len()
            );
            return;
        }
        if let Some(got) = frame::Sensor_Id_Of(bytes) {
            if got as u32 != want {
                Warn_Throttled("world.sensor_id_mismatch", 5000, || {
                    format!(
                        "[world] slot={} 帧头 sensor_id={got} 与连接登记值={want} 不符，丢弃该帧",
                        self.slot
                    )
                });
                return;
            }
        }
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.sensor.On_Binary(bytes);
    }
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    use tokio::sync::{broadcast, RwLock};

    use pleiades_base::robot::core::cluster::ClusterInfoTable;
    use pleiades_base::robot::core::grid::OccupancyGrid;
    use pleiades_base::robot::core::map_delta::MapDelta;
    use pleiades_base::robot::core::state::RobotState;

    use crate::sim::sensors::lidar::frame::Test_Frame;
    use crate::world_link::protocol::MountOut;

    /// 假传感器：只记收到几帧，用于验证端点路由/丢弃
    struct FakeSensor {
        slot: String,
        hits: AtomicU64,
        last_len: AtomicU64,
    }

    impl FakeSensor {
        fn new(slot: &str) -> Arc<Self> {
            Arc::new(Self {
                slot: slot.to_string(),
                hits: AtomicU64::new(0),
                last_len: AtomicU64::new(0),
            })
        }
    }

    impl Sensor for FakeSensor {
        fn Slot(&self) -> &str {
            &self.slot
        }
        fn On_Binary(&self, bytes: &[u8]) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            self.last_len.store(bytes.len() as u64, Ordering::Relaxed);
        }
        fn Shutdown(&self) {}
    }

    fn ctx_for_tests() -> MappingCtx {
        let (map_tx, _) = broadcast::channel::<Vec<MapDelta>>(4);
        MappingCtx {
            grid: Arc::new(RwLock::new(OccupancyGrid::new())),
            robot_state: Arc::new(RwLock::new(RobotState::default())),
            cluster_table: Arc::new(ClusterInfoTable::new()),
            map_tx,
            node_handle: None,
            local_peer_id: None,
            obstacle_inflation_radius: 0.2,
            pose_age: Arc::new(AtomicU64::new(0)),
            pose_history: crate::sim::chassis::pose_ingest::PoseHistory::New(),
            mount: MountOut::default(),
        }
    }

    fn decl(slot: &str, sensor: &str) -> ResolvedSensor {
        ResolvedSensor {
            slot: slot.to_string(),
            sensor: sensor.to_string(),
            mount: MountOut::default(),
            params: serde_json::json!({}),
        }
    }

    // ---------------------------------------------------------- 限流日志

    #[test]
    fn test_should_log_throttles() {
        // 首次放行
        assert!(Should_Log("test.throttle.a", 5000));
        // 窗口内不再放行
        assert!(!Should_Log("test.throttle.a", 5000));
        assert!(!Should_Log("test.throttle.a", 5000));
        // 不同 key 互不影响
        assert!(Should_Log("test.throttle.b", 5000));
        // period = 0 → 每次都放行
        assert!(Should_Log("test.throttle.c", 0));
        assert!(Should_Log("test.throttle.c", 0));
    }

    // ---------------------------------------------------------- SensorEndpoint

    #[test]
    fn test_endpoint_drops_before_sensor_ready() {
        let fake = FakeSensor::new("lidar_front");
        let ep = SensorEndpoint::New(fake.clone());
        // sensor_ready 之前：丢弃（debug，不 warn），count 也不涨
        ep.Push_Binary(&Test_Frame(1, 1, 0.0, &[1.0]));
        assert_eq!(fake.hits.load(Ordering::Relaxed), 0);
        assert_eq!(ep.Frames(), 0);
        assert_eq!(ep.Slot(), "lidar_front");
    }

    #[test]
    fn test_endpoint_routes_matching_id() {
        let fake = FakeSensor::new("lidar_front");
        let ep = SensorEndpoint::New(fake.clone());
        ep.Set_Sensor_Id(7);
        let frame = Test_Frame(7, 1, 0.0, &[1.0, 2.0]);
        ep.Push_Binary(&frame);
        assert_eq!(fake.hits.load(Ordering::Relaxed), 1);
        assert_eq!(ep.Frames(), 1);
        assert_eq!(fake.last_len.load(Ordering::Relaxed), frame.len() as u64);
    }

    #[test]
    fn test_endpoint_drops_mismatched_id() {
        let fake = FakeSensor::new("lidar_front");
        let ep = SensorEndpoint::New(fake.clone());
        ep.Set_Sensor_Id(7);
        // 帧头 id=9 ≠ 登记值 7 → 丢弃
        ep.Push_Binary(&Test_Frame(9, 1, 0.0, &[1.0]));
        assert_eq!(fake.hits.load(Ordering::Relaxed), 0);
        assert_eq!(ep.Frames(), 0);
        // 相符的帧照常投递（丢弃不影响后续）
        ep.Push_Binary(&Test_Frame(7, 2, 0.0, &[1.0]));
        assert_eq!(fake.hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_endpoint_short_packet_reaches_sensor() {
        // 短到取不到 sensor_id（< 3 B）：不丢弃，交给传感器统一报"短包"错误分支
        let fake = FakeSensor::new("lidar_front");
        let ep = SensorEndpoint::New(fake.clone());
        ep.Set_Sensor_Id(7);
        ep.Push_Binary(&[1u8, 2]);
        assert_eq!(fake.hits.load(Ordering::Relaxed), 1);
        assert_eq!(ep.Frames(), 1);
    }

    // ---------------------------------------------------------- SensorSet 工厂

    #[tokio::test]
    async fn test_sensor_set_builds_lidar() {
        let ctx = ctx_for_tests();
        let set = SensorSet::Build(&[decl("lidar_front", "lidar_2d"), decl("lidar_rear", "lidar_2d")], &ctx)
            .expect("lidar_2d 应能构造");
        assert_eq!(set.Len(), 2);
        assert_eq!(set.All().len(), 2);
        assert!(set.Get("lidar_front").is_some());
        assert!(set.Get("lidar_rear").is_some());
        assert!(set.Get("lidar_missing").is_none());
        // 收尾：不留建图 task
        set.Shutdown();
    }

    #[tokio::test]
    async fn test_sensor_set_rejects_unknown_sensor() {
        let ctx = ctx_for_tests();
        let e = SensorSet::Build(&[decl("cam_front", "camera_rgb")], &ctx)
            .err()
            .expect("未知传感器必须被拒");
        assert!(e.contains("camera_rgb"), "错误信息缺名字：{e}");
        assert!(e.contains("lidar_2d"), "错误信息缺可用目录：{e}");

        // 先建好 lidar、后一条非法 → 整体 Err，且已建好的传感器被 Shutdown（不留悬空建图 task，R18）
        let decls = [decl("lidar_front", "lidar_2d"), decl("cam_front", "camera_rgb")];
        assert!(SensorSet::Build(&decls, &ctx).is_err());
    }
}
