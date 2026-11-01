//Presented by KeJi
//Created Date ： 2026-10-28
//Modified Date ： 2026-10-28

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! 像素 → 世界 `(x, y)` 的地面平面投影（task_37 §四 Step 3）：**纯函数，无 I/O、无锁、无线程**。
//!
//! ```text
//! 框底边中点 (u, v)（≈脚）
//!   → 针孔反投影（内参由 [detector.camera].fov_deg + fov_axis + 帧自带 width/height 解析）
//!   → 机体系射线（mount_yaw_deg / mount_pitch_deg 的基向量，spawn 时一次算好）
//!   → 世界系射线（pose.yaw）
//!   → 与 z = ground_z 求交 → 世界 (x, y)
//! ```
//!
//! 四条坐标约定（每一处都有源码依据，改之前先看这里）：
//! 1. 世界 Z-up，`yaw` 绕 `+z` **逆时针为正**（`WorldTransform.gd:3-5`），`RobotState.attitude.yaw`
//!    就是**弧度**（`pleiades-base/src/Robot/core/state.rs:65`）；
//! 2. 机体系 `x` 前 / `y` 右 / `z` 高，mount 位置按 `x + mx·cos − my·sin`、
//!    `y + mx·sin + my·cos` 旋转 —— 与 sim 建图的唯一实现
//!    `pleiades-sim/src/sim/sensors/lidar/mapping_task.rs:192-201`（`Sensor_Pose`）**同式**；
//! 3. 相机光轴 = 传感器局部 **−Z**、`+X` 右、`+Y` 上；mount `yaw` 正把光轴从车尾转向**车头**
//!    （**前视 yaw = +90°**）、`pitch` **负 = 低头**（`Vehicle.gd:149-152,181-184`、
//!    `orion-simulation-world/src/tests/check_vehicle.gd:340-380` 实测）；
//! 4. 本文件的角度参数**一律用度**（键名带 `_deg`），只有 `Pose::yaw` 用弧度
//!    （它直接来自 `RobotState`，不换算、不二次转换）。
//!
//! 为什么不是「手工标的行号→距离表」：机体系基向量由 mount `yaw/pitch` **解析生成**（§四.2
//! 「启动时由参数解析生成，不手工标表」），1 次 `sin/cos` 建表、逐帧只做乘加；
//! `f` 由 `fov` 与**帧自带**宽高解析（分辨率不单独配，§四.3）。
//!
//! 边界：本文件**不**读时钟、不读配置、不碰 `RobotState`、不打日志 —— 拒答理由由调用方打。
//! 不补偿 roll、不校畸变（§四.5：只信画面中部，sim 无风，够用）。

use crate::config::DetectorCameraConfig;

/// 射线「够陡」的下限（度）：世界系射线的俯角小于它 = 接近地平线（`|ray_z|` 过小）→ 拒答。
///
/// 1° 是**数值保护**而不是精度门（默认 2.5 m 高、12 m 上限时真正的门是 `max_range_m`）：
/// 没有它，`w.z → 0` 会让 `t → ∞`，虽然最终也会被距离门拦下，但会先算出 inf/NaN 中间量。
pub const MIN_DEPRESSION_DEG: f32 = 1.0;

/// 相机离地高度下限（米）：`pose.z + mount_z − ground_z` 低于它 → 拒答（§四.5 的补充守门）。
///
/// 依据：`RobotState` 默认全 0（`state.rs:53-86`）。uav 上 mavlink 未连、sim 上世界还没发过
/// `pose` 时 `pose.z = 0` → 相机离地 0.045 m → 反投影会给出「人离我 0.09 m」这种**看起来合法**
/// 的坏点（`v = cy+108` 时 `t ≈ 0.087 m`），四条 required 判据一条都拦不住。
/// 0.10 m 远低于 UAV 巡航 2 m 与 ugv 相机高度，不会误杀正常工况。
pub const MIN_HEIGHT_M: f32 = 0.10;

/// 视场角口径：`fov_deg` 指的是**纵向**还是**横向**的全视角（§四.4 的 `fov_axis`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FovAxis {
    /// 纵向（默认，§四.4 的示例值）
    #[default]
    Vertical,
    /// 横向
    Horizontal,
}

impl FovAxis {
    /// 大小写不敏感；除 `"horizontal"` 外一律 `Vertical`（非法值不 panic、不告警，与各 `unwrap_or` 访问器同风格）
    pub fn Parse(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case("horizontal") {
            Self::Horizontal
        } else {
            Self::Vertical
        }
    }

    /// 日志/文案用（与 [`FovAxis::Parse`] 互逆）
    pub fn As_Str(&self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
        }
    }
}

/// 投影参数（`[detector.camera]` 的**最终生效值**，全部已填默认、已剔非法值）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectionParams {
    /// 视场角（度，全视角）
    pub fov_deg: f32,
    /// 视场角口径
    pub fov_axis: FovAxis,
    /// 光心在机体系的位置（米）
    pub mount_x: f32,
    pub mount_y: f32,
    pub mount_z: f32,
    /// 光轴相对车头的偏航（度；**前视 = +90°**，见模块头第 3 条）
    pub mount_yaw_deg: f32,
    /// 光轴俯仰（度；**负 = 低头**）
    pub mount_pitch_deg: f32,
    /// 地面高度（米，世界 z）
    pub ground_z: f32,
    /// 超过它就拒答（米，水平距离）
    pub max_range_m: f32,
}

impl Default for ProjectionParams {
    /// §四.4 的示例值 = `[detector.camera]` 的默认值 = 两侧 `config.rs` 的 `DEFAULT_DETECTOR_*`
    fn default() -> Self {
        Self {
            fov_deg: 70.0,
            fov_axis: FovAxis::Vertical,
            mount_x: 0.20,
            mount_y: 0.0,
            mount_z: 0.045,
            mount_yaw_deg: 90.0,
            mount_pitch_deg: -15.0,
            ground_z: 0.0,
            max_range_m: 12.0,
        }
    }
}

impl ProjectionParams {
    /// 从 `[detector.camera]` 解析：段缺失 / 键缺失 / 非法 → 默认值（`None` = 整段没写）
    pub fn From_Config(cfg: Option<&DetectorCameraConfig>) -> Self {
        let Some(cfg) = cfg else {
            return Self::default();
        };
        Self {
            fov_deg: cfg.Fov_Deg(),
            fov_axis: FovAxis::Parse(cfg.Fov_Axis()),
            mount_x: cfg.Mount_X(),
            mount_y: cfg.Mount_Y(),
            mount_z: cfg.Mount_Z(),
            mount_yaw_deg: cfg.Mount_Yaw_Deg(),
            mount_pitch_deg: cfg.Mount_Pitch_Deg(),
            ground_z: cfg.Ground_Z(),
            max_range_m: cfg.Max_Range_M(),
        }
    }
}

/// 内参（针孔、方形像素：`fx = fy = f`）
///
/// 只留**投影真正要用的三个数**：帧宽高由调用方拿着（`Frame::width/height`），
/// 分辨率变了就重建一个 —— 不把宽高存进来，免得 bin crate 里多出两条 `field never read`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intrinsics {
    /// 焦距（像素）
    pub f: f32,
    /// 主点 = 画幅中心
    pub cx: f32,
    pub cy: f32,
}

impl Intrinsics {
    /// 由 `fov_deg + fov_axis` 与**帧自带**宽高解析；宽高 0 / fov 退化 → `None`（调用方只 warn）
    pub fn New(p: &ProjectionParams, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let tan_half = (p.fov_deg * 0.5).to_radians().tan();
        if !tan_half.is_finite() || tan_half <= 0.0 {
            return None;
        }
        let w = width as f32;
        let h = height as f32;
        let f = match p.fov_axis {
            FovAxis::Vertical => (h * 0.5) / tan_half,
            FovAxis::Horizontal => (w * 0.5) / tan_half,
        };
        if !f.is_finite() || f <= 0.0 {
            return None;
        }
        Some(Self { f, cx: w * 0.5, cy: h * 0.5 })
    }
}

/// 位姿（**只取地面投影要用的四个量**；`yaw` 弧度，直接来自 `RobotState`）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
}

/// 地面落点（§四.6.1 的「估计 (x,y) + 距离/方位」）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ground {
    /// 世界坐标（米）
    pub x: f32,
    pub y: f32,
    /// 水平距离（米，**以机体原点为基准**，与 §五 的距离带同基准）
    pub range_m: f32,
    /// 世界系方位角（度，`atan2(dy, dx)`，`+x` 为 0）
    pub bearing_deg: f32,
    /// 相对机头的方位角（度，`[-180, 180)`，0 = 正前方）
    pub rel_bearing_deg: f32,
}

/// 拒答理由（§四.5：**不满足就拒答，不输出坏点**）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reject {
    /// 框底边不在画面下半部
    NotLowerHalf { v: f32, cy: f32 },
    /// 射线接近地平线（世界系射线 `z` 分量不够负）
    NearHorizon { ray_z: f32 },
    /// 交点落在相机背后（`t <= 0`，即相机在 `ground_z` 以下）
    BehindGround { t: f32 },
    /// 相机离地过低（位姿未就位），见 [`MIN_HEIGHT_M`]
    TooLow { height_m: f32 },
    /// 超出 `max_range_m`
    TooFar { range_m: f32, max_m: f32 },
}

impl Reject {
    /// 中文可读文案（进日志的 `判定=拒答（…）`，也让单测能断言「拒的是哪一条」）
    pub fn Reason(&self) -> String {
        match self {
            Self::NotLowerHalf { v, cy } => {
                format!("框底边不在画面下半部（v={v:.0} 需 > cy={cy:.0}）")
            }
            Self::NearHorizon { ray_z } => format!(
                "射线接近地平线（ray_z={ray_z:.4}，需 <= -{:.4}）",
                MIN_DEPRESSION_DEG.to_radians().sin()
            ),
            Self::BehindGround { t } => format!("交点在相机背后（t={t:.2} m <= 0）"),
            Self::TooLow { height_m } => format!(
                "相机离地过低（{height_m:.2} m < {MIN_HEIGHT_M:.2} m，位姿可能未就位）"
            ),
            Self::TooFar { range_m, max_m } => {
                format!("超出最大距离（{range_m:.2} m > {max_m:.2} m）")
            }
        }
    }
}

/// 框底边中点（≈脚；像素坐标按「中心 = (u, v)」的整数近似，半像素差异远小于判据 1 m）
pub fn Bottom_Center(xmin: f32, xmax: f32, ymax: f32) -> (f32, f32) {
    ((xmin + xmax) * 0.5, ymax)
}

/// 投影器：**除内参外的一切都在 `New` 里算好**（机体系基向量 = 「机体系常量单应」的解析形式）
pub struct Projector {
    params: ProjectionParams,
    /// 相机 `+X`（画面右）在机体系的单位向量
    right: [f32; 3],
    /// 相机 `+Y`（画面上）
    up: [f32; 3],
    /// 光轴（相机 `−Z`）
    optic: [f32; 3],
}

impl Projector {
    /// 解析 mount yaw/pitch → 机体系基向量（一次 `sin/cos`，逐帧零成本）
    pub fn New(params: ProjectionParams) -> Self {
        let y = params.mount_yaw_deg.to_radians();
        let p = params.mount_pitch_deg.to_radians();
        Self {
            right: [y.cos(), y.sin(), 0.0],
            up: [-y.sin() * p.sin(), -y.cos() * p.sin(), p.cos()],
            optic: [y.sin() * p.cos(), -y.cos() * p.cos(), p.sin()],
            params,
        }
    }

    /// 生效参数（spawn 时打一行日志用）
    pub fn Params(&self) -> &ProjectionParams {
        &self.params
    }

    /// 每帧算一次内参（`fov` × 帧自带宽高；分辨率变了自动跟着变）
    pub fn Intrinsics_For(&self, width: u32, height: u32) -> Option<Intrinsics> {
        Intrinsics::New(&self.params, width, height)
    }

    /// 像素 → 世界 `(x, y)`；不满足 §四.5 的保护一律 `Err`（**绝不返回坏点**）
    ///
    /// 比较一律写成「取反」形式（`if !(a > b)`），让 `NaN` 也落进拒答分支。
    pub fn Project(&self, intr: &Intrinsics, pose: &Pose, u: f32, v: f32) -> Result<Ground, Reject> {
        // ① 框底边必须在画面下半部（NaN 也会被这一条拦下）
        if !(v > intr.cy) {
            return Err(Reject::NotLowerHalf { v, cy: intr.cy });
        }

        // ② 像素 → 机体系射线（a 右、b 上、光轴 1；未归一）
        let a = (u - intr.cx) / intr.f;
        let b = -(v - intr.cy) / intr.f;
        let mut d = [0.0_f32; 3];
        for i in 0..3 {
            d[i] = a * self.right[i] + b * self.up[i] + self.optic[i];
        }
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if !(len > 0.0) || !len.is_finite() {
            return Err(Reject::NearHorizon { ray_z: 0.0 });
        }
        let n = [d[0] / len, d[1] / len, d[2] / len];

        // ③ 机体系 → 世界系（只有 yaw：不补偿 roll / pitch，§四.5）
        let (c, s) = (pose.yaw.cos(), pose.yaw.sin());
        let dir = [n[0] * c - n[1] * s, n[0] * s + n[1] * c, n[2]];

        // ④ 射线接近地平线 → 拒答（|ray_z| 过小）
        if !(dir[2] <= -MIN_DEPRESSION_DEG.to_radians().sin()) {
            return Err(Reject::NearHorizon { ray_z: dir[2] });
        }

        // ⑤ 相机光心（mount 位置按 yaw 旋转；与 mapping_task.rs:192-201 的 Sensor_Pose 同式）
        let cam_x = pose.x + self.params.mount_x * c - self.params.mount_y * s;
        let cam_y = pose.y + self.params.mount_x * s + self.params.mount_y * c;
        let cam_z = pose.z + self.params.mount_z;

        // ⑥ 与 z = ground_z 求交（t = 光心到落点的**斜距**，因为 dir 已归一）
        let t = (self.params.ground_z - cam_z) / dir[2];
        if !(t > 0.0) {
            return Err(Reject::BehindGround { t });
        }

        // ⑦ 相机离地过低（位姿未就位）→ 拒答（排在 ⑥ 之后：相机低于地面时报「t<0」而非本条）
        let height_m = cam_z - self.params.ground_z;
        if !(height_m >= MIN_HEIGHT_M) {
            return Err(Reject::TooLow { height_m });
        }

        let x = cam_x + t * dir[0];
        let y = cam_y + t * dir[1];
        let range_m = (x - pose.x).hypot(y - pose.y);
        if !(range_m <= self.params.max_range_m) {
            return Err(Reject::TooFar { range_m, max_m: self.params.max_range_m });
        }

        let bearing_deg = (y - pose.y).atan2(x - pose.x).to_degrees();
        Ok(Ground {
            x,
            y,
            range_m,
            bearing_deg,
            rel_bearing_deg: Wrap_Deg(bearing_deg - pose.yaw.to_degrees()),
        })
    }
}

/// 角度归一到 `[-180, 180)`
fn Wrap_Deg(deg: f32) -> f32 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 夹具（全部手写参数，零 I/O、不碰配置）

    const W: u32 = 640;
    const H: u32 = 480;

    /// §四.4 的示例参数
    fn Params() -> ProjectionParams {
        ProjectionParams::default()
    }

    fn Intr(p: &ProjectionParams) -> Intrinsics {
        match Intrinsics::New(p, W, H) {
            Some(i) => i,
            None => {
                assert!(false, "640x480 + fov=70 必须能算出内参");
                Intrinsics { f: 1.0, cx: 0.0, cy: 0.0 }
            }
        }
    }

    /// 机体在原点、朝向 +x、离地 2.5 m
    fn Pose_At_H() -> Pose {
        Pose { x: 0.0, y: 0.0, z: 2.5, yaw: 0.0 }
    }

    fn Ok_Of(r: Result<Ground, Reject>, why: &str) -> Ground {
        match r {
            Ok(g) => g,
            Err(e) => {
                assert!(false, "{why}：被拒答（{}）", e.Reason());
                Ground { x: 0.0, y: 0.0, range_m: 0.0, bearing_deg: 0.0, rel_bearing_deg: 0.0 }
            }
        }
    }

    fn Err_Of(r: Result<Ground, Reject>, why: &str) -> Reject {
        match r {
            Ok(g) => {
                assert!(false, "{why}：必须拒答，却算出 ({:.2},{:.2})", g.x, g.y);
                Reject::TooFar { range_m: 0.0, max_m: 0.0 }
            }
            Err(e) => e,
        }
    }

    fn Close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    // ---------------- 内参

    /// 内参：纵向/横向两种口径、退化输入 → None（不等 panic）
    #[test]
    fn test_fov_axis_parse_and_intrinsics() {
        // 纵向 70° @640x480：f = 240/tan(35°) = 342.7555
        let i = Intr(&Params());
        assert!(Close(i.f, 342.7555, 0.01), "f={}", i.f);
        assert_eq!((i.cx, i.cy), (320.0, 240.0));
        // 纵向 70° @1280x720：f = 360/tan(35°) = 514.1333
        let big = match Intrinsics::New(&Params(), 1280, 720) {
            Some(i) => i,
            None => {
                assert!(false, "1280x720 必须能算出内参");
                return;
            }
        };
        assert!(Close(big.f, 514.1333, 0.01), "f={}", big.f);
        // 横向 70° @1280x720：f = 640/tan(35°) = 914.0147（与纵向口径必须不同）
        let mut ph = Params();
        ph.fov_axis = FovAxis::Horizontal;
        let hor = match Intrinsics::New(&ph, 1280, 720) {
            Some(i) => i,
            None => {
                assert!(false, "横向 70° 必须能算出内参");
                return;
            }
        };
        assert!(Close(hor.f, 914.0147, 0.01), "f={}", hor.f);
        assert_ne!(big.f, hor.f, "fov_axis 必须真的改变内参");
        // 退化：宽高 0 / fov 非法
        assert!(Intrinsics::New(&Params(), 0, 480).is_none(), "宽 0 → None");
        assert!(Intrinsics::New(&Params(), 640, 0).is_none(), "高 0 → None");
        let mut p_bad = Params();
        p_bad.fov_deg = 0.0;
        assert!(Intrinsics::New(&p_bad, W, H).is_none(), "fov=0 → None（不 panic）");
        // fov_axis 解析：大小写不敏感，非法值回纵向
        assert_eq!(FovAxis::Parse("Horizontal"), FovAxis::Horizontal);
        assert_eq!(FovAxis::Parse("  hoRizontal "), FovAxis::Horizontal);
        assert_eq!(FovAxis::Parse("vertical"), FovAxis::Vertical);
        assert_eq!(FovAxis::Parse(""), FovAxis::Vertical);
        assert_eq!(FovAxis::Parse("斜的"), FovAxis::Vertical);
        assert_eq!(FovAxis::Vertical.As_Str(), "vertical");
        assert_eq!(FovAxis::Horizontal.As_Str(), "horizontal");
    }

    // ---------------- 判据①：给定像素 + 位姿 + 参数 → 固定结果

    /// 正前方 4.1965 m（与手算一致：2.5 m 高、pitch −15°、v=348 ⇒ 俯角 32.5°）
    #[test]
    fn test_golden_center_column() {
        let p = Params();
        let pr = Projector::New(p);
        let g = Ok_Of(pr.Project(&Intr(&p), &Pose_At_H(), 320.0, 348.0), "正前方地面点必须算得出");
        assert!(Close(g.x, 4.1965, 1e-2), "x={}", g.x);
        assert!(Close(g.y, 0.0, 1e-2), "y={}", g.y);
        assert!(Close(g.range_m, 4.1965, 1e-2), "range={}", g.range_m);
        assert!(Close(g.bearing_deg, 0.0, 1e-2), "bearing={}", g.bearing_deg);
        assert!(Close(g.rel_bearing_deg, 0.0, 1e-2), "rel={}", g.rel_bearing_deg);
    }

    /// 左右对称（u 偏 +80 / −80 像素 → y = ±1.0547，方位 ±14.1085°）
    #[test]
    fn test_golden_lateral_sign() {
        let p = Params();
        let pr = Projector::New(p);
        let intr = Intr(&p);
        let right = Ok_Of(pr.Project(&intr, &Pose_At_H(), 400.0, 348.0), "右侧像素必须算得出");
        let left = Ok_Of(pr.Project(&intr, &Pose_At_H(), 240.0, 348.0), "左侧像素必须算得出");
        assert!(Close(right.x, 4.1965, 1e-2), "x={}", right.x);
        assert!(Close(right.y, 1.0547, 1e-2), "y={}", right.y);
        assert!(Close(right.bearing_deg, 14.1085, 1e-2), "bearing={}", right.bearing_deg);
        assert!(Close(right.rel_bearing_deg, 14.1085, 1e-2), "rel={}", right.rel_bearing_deg);
        assert!(Close(left.y, -1.0547, 1e-2), "y={}", left.y);
        assert!(Close(left.bearing_deg, -14.1085, 1e-2), "bearing={}", left.bearing_deg);
        assert!(Close(left.x, right.x, 1e-3), "同一行像素的前方距离必须对称");
    }

    /// 位姿 yaw 一改，世界点跟着转（机体朝 +y 时，同一像素落到 (0, 4.1965)）
    #[test]
    fn test_yaw_rotates_world_point() {
        let p = Params();
        let pr = Projector::New(p);
        let intr = Intr(&p);
        let pose = Pose { x: 0.0, y: 0.0, z: 2.5, yaw: std::f32::consts::FRAC_PI_2 };
        let g = Ok_Of(pr.Project(&intr, &pose, 320.0, 348.0), "转 90° 后必须算得出");
        assert!(Close(g.x, 0.0, 1e-2), "x={}", g.x);
        assert!(Close(g.y, 4.1965, 1e-2), "y={}", g.y);
        assert!(Close(g.bearing_deg, 90.0, 1e-2), "bearing={}", g.bearing_deg);
        assert!(Close(g.rel_bearing_deg, 0.0, 1e-2), "相对机头应为 0：{}", g.rel_bearing_deg);
    }

    // ---------------- 判据：改 pitch / 改高度，结果随之变

    /// pitch −15° → −5°：同一个像素落到更远的地面（4.1965 → 6.3474）
    #[test]
    fn test_pitch_changes_result() {
        let mut p = Params();
        let near = Ok_Of(Projector::New(p).Project(&Intr(&p), &Pose_At_H(), 320.0, 348.0), "pitch=-15 必须算得出");
        p.mount_pitch_deg = -5.0;
        let far = Ok_Of(Projector::New(p).Project(&Intr(&p), &Pose_At_H(), 320.0, 348.0), "pitch=-5 必须算得出");
        assert!(Close(near.x, 4.1965, 1e-2), "x={}", near.x);
        assert!(Close(far.x, 6.3474, 1e-2), "x={}", far.x);
        assert!(far.x - near.x > 1.0, "抬高 pitch 必须让同一像素落到更远：{} → {}", near.x, far.x);
    }

    /// 高度 2.5 m → 1.5 m：同一个像素落到更近的地面（4.1965 → 2.6262）
    #[test]
    fn test_height_changes_result() {
        let p = Params();
        let pr = Projector::New(p);
        let intr = Intr(&p);
        let high = Ok_Of(pr.Project(&intr, &Pose_At_H(), 320.0, 348.0), "2.5 m 必须算得出");
        let low_pose = Pose { x: 0.0, y: 0.0, z: 1.5, yaw: 0.0 };
        let low = Ok_Of(pr.Project(&intr, &low_pose, 320.0, 348.0), "1.5 m 必须算得出");
        assert!(Close(high.x, 4.1965, 1e-2), "x={}", high.x);
        assert!(Close(low.x, 2.6262, 1e-2), "x={}", low.x);
        assert!(high.x - low.x > 1.0, "降高度必须让同一像素落到更近：{} → {}", low.x, high.x);
    }

    /// 画面最下一行 = 最近可见距离 2.3440 m（与 §四.4「pitch=−15° 时最近约 2.1 m」口径一致）
    #[test]
    fn test_bottom_row_is_near() {
        let p = Params();
        let pr = Projector::New(p);
        let g = Ok_Of(pr.Project(&Intr(&p), &Pose_At_H(), 320.0, 479.0), "最下一行必须算得出");
        assert!(Close(g.x, 2.3440, 1e-2), "x={}", g.x);
        assert!(g.x < 4.1965, "最下一行必须比画面中部更近");
    }

    // ---------------- 判据②：五条保护各有拒答用例

    /// 保护①：框底边不在画面下半部
    #[test]
    fn test_reject_not_lower_half() {
        let p = Params();
        let pr = Projector::New(p);
        let intr = Intr(&p);
        // v 正好压在 cy 上
        match Err_Of(pr.Project(&intr, &Pose_At_H(), 320.0, 240.0), "v = cy 必须拒答") {
            Reject::NotLowerHalf { v, cy } => {
                assert!(Close(v, 240.0, 1e-3));
                assert!(Close(cy, 240.0, 1e-3));
            }
            other => assert!(false, "必须是 NotLowerHalf：{other:?}"),
        }
        // 画面上半部
        assert!(matches!(
            Err_Of(pr.Project(&intr, &Pose_At_H(), 320.0, 100.0), "上半部必须拒答"),
            Reject::NotLowerHalf { .. }
        ));
        // NaN 像素也不能算出点
        assert!(matches!(
            Err_Of(pr.Project(&intr, &Pose_At_H(), 320.0, f32::NAN), "NaN 必须拒答"),
            Reject::NotLowerHalf { .. }
        ));
    }

    /// 保护②：射线接近地平线（pitch=0 时 v 刚过 cy → 俯角 0.17° < 1°）
    #[test]
    fn test_reject_near_horizon() {
        let mut p = Params();
        p.mount_pitch_deg = 0.0;
        let pr = Projector::New(p);
        match Err_Of(pr.Project(&Intr(&p), &Pose_At_H(), 320.0, 241.0), "近地平线必须拒答") {
            Reject::NearHorizon { ray_z } => assert!(ray_z < 0.0 && ray_z > -0.01, "ray_z={ray_z}"),
            other => assert!(false, "必须是 NearHorizon：{other:?}"),
        }
    }

    /// 保护③：t < 0（相机在 ground_z 以下 → 地面在光心背后）
    #[test]
    fn test_reject_behind_ground() {
        let p = Params();
        let pr = Projector::New(p);
        let below = Pose { x: 0.0, y: 0.0, z: -0.5, yaw: 0.0 };
        match Err_Of(pr.Project(&Intr(&p), &below, 320.0, 348.0), "相机在地面以下必须拒答") {
            Reject::BehindGround { t } => assert!(t < 0.0, "t={t}"),
            other => assert!(false, "必须是 BehindGround：{other:?}"),
        }
    }

    /// 保护④：相机离地过低（`RobotState::default()` 那种「位姿全是 0」的情形）
    #[test]
    fn test_reject_too_low() {
        let p = Params();
        let pr = Projector::New(p);
        let zero = Pose { x: 0.0, y: 0.0, z: 0.0, yaw: 0.0 };
        match Err_Of(pr.Project(&Intr(&p), &zero, 320.0, 348.0), "pose.z=0 必须拒答") {
            Reject::TooLow { height_m } => assert!(Close(height_m, 0.045, 1e-3), "height={height_m}"),
            other => assert!(false, "必须是 TooLow：{other:?}"),
        }
    }

    /// 保护⑤：超出 max_range_m
    #[test]
    fn test_reject_too_far() {
        let mut p = Params();
        p.max_range_m = 3.0;
        let pr = Projector::New(p);
        match Err_Of(pr.Project(&Intr(&p), &Pose_At_H(), 320.0, 348.0), "4.1965 m > 3 m 必须拒答") {
            Reject::TooFar { range_m, max_m } => {
                assert!(Close(range_m, 4.1965, 1e-2), "range={range_m}");
                assert!(Close(max_m, 3.0, 1e-3));
            }
            other => assert!(false, "必须是 TooFar：{other:?}"),
        }
    }

    /// 拒答文案可读（落到日志里能看出「拒的是哪一条」）
    #[test]
    fn test_reject_reason_text() {
        let cases = [
            (Reject::NotLowerHalf { v: 100.0, cy: 240.0 }, "下半部"),
            (Reject::NearHorizon { ray_z: -0.001 }, "地平线"),
            (Reject::BehindGround { t: -1.0 }, "背后"),
            (Reject::TooLow { height_m: 0.01 }, "过低"),
            (Reject::TooFar { range_m: 30.0, max_m: 12.0 }, "最大距离"),
        ];
        for (r, want) in cases {
            let text = r.Reason();
            assert!(text.contains(want), "文案应含「{want}」：{text}");
        }
    }

    // ---------------- `[detector.camera]` → 参数（段缺失/非法一律回默认）

    /// 段缺失 → §四.4 默认值；非法值回默认；合法值原样生效
    #[test]
    fn test_from_config_defaults_and_illegal() {
        // 段缺失
        assert_eq!(ProjectionParams::From_Config(None), ProjectionParams::default());
        // 空段：所有键缺失 → 默认
        let empty = DetectorCameraConfig::default();
        assert_eq!(ProjectionParams::From_Config(Some(&empty)), ProjectionParams::default());
        // 合法值原样生效
        let full = DetectorCameraConfig {
            fov_deg: Some(90.0),
            fov_axis: Some("Horizontal".to_string()),
            mount_x: Some(0.5),
            mount_y: Some(-0.1),
            mount_z: Some(0.3),
            mount_yaw_deg: Some(45.0),
            mount_pitch_deg: Some(-30.0),
            ground_z: Some(1.5),
            max_range_m: Some(20.0),
        };
        let p = ProjectionParams::From_Config(Some(&full));
        assert_eq!(p.fov_deg, 90.0);
        assert_eq!(p.fov_axis, FovAxis::Horizontal);
        assert_eq!(p.mount_x, 0.5);
        assert_eq!(p.mount_y, -0.1);
        assert_eq!(p.mount_z, 0.3);
        assert_eq!(p.mount_yaw_deg, 45.0);
        assert_eq!(p.mount_pitch_deg, -30.0);
        assert_eq!(p.ground_z, 1.5);
        assert_eq!(p.max_range_m, 20.0);
        // 非法值回默认（fov 越界 / NaN / max_range <= 0 / 空白 axis）
        let bad = DetectorCameraConfig {
            fov_deg: Some(200.0),
            fov_axis: Some("   ".to_string()),
            mount_x: Some(f32::NAN),
            mount_y: Some(f32::INFINITY),
            mount_z: Some(f32::NAN),
            mount_yaw_deg: Some(f32::NAN),
            mount_pitch_deg: Some(f32::NAN),
            ground_z: Some(f32::NAN),
            max_range_m: Some(0.0),
        };
        let p = ProjectionParams::From_Config(Some(&bad));
        assert_eq!(p.fov_deg, 70.0, "越界 fov 回默认");
        assert_eq!(p.fov_axis, FovAxis::Vertical, "空白 axis 回默认");
        assert_eq!(p.mount_x, 0.20, "NaN mount_x 回默认");
        assert_eq!(p.mount_y, 0.0);
        assert_eq!(p.mount_z, 0.045);
        assert_eq!(p.mount_yaw_deg, 90.0);
        assert_eq!(p.mount_pitch_deg, -15.0);
        assert_eq!(p.ground_z, 0.0);
        assert_eq!(p.max_range_m, 12.0, "max_range=0 回默认");
    }
}
