//Presented by KeJi
//Created Date ： 2026-07-20
//Modified Date ： 2026-07-20
//
//Vendored from YDLidar-SDK/rust/tmini-protocol/src/types.rs
//对齐 C++ `node_info`, `LaserPoint`, `LaserScan`

/// 单个采样点（对齐 C++ `node_info`）
#[derive(Debug, Clone, Copy, Default)]
pub struct NodeInfo {
    /// 首包标记 (对齐 `node_info::sync`)
    pub sync: u8,
    /// 抗干扰标志: 0x03=阳光, 0x02=玻璃 (对齐 `node_info::is`)
    pub is: u8,
    /// 信号强度 (对齐 `node_info::qual`)
    pub qual: u16,
    /// 角度 (°) 编码为 `angle * SDK_UNIT128` (对齐 `node_info::angle`)
    pub angle: u16,
    /// 距离原始值 `distance_mm / 4` (对齐 `node_info::dist`)
    pub dist: u16,
    /// 时间戳 纳秒 (对齐 `node_info::stamp`)
    pub stamp: u64,
    /// 延迟时间 (对齐 `node_info::delayTime`)
    pub delay_time: u32,
    /// 扫描频率 ×10 (对齐 `node_info::scanFreq`)，即 Hz = scan_freq / 10.0
    pub scan_freq: u8,
}

/// 激光点（输出格式，对齐 `LaserPoint`）
#[derive(Debug, Clone, Copy, Default)]
pub struct LaserPoint {
    /// 角度 (弧度)
    pub angle: f32,
    /// 距离 (米)
    pub range: f32,
    /// 信号强度 (0-255)
    pub intensity: f32,
}

/// 扫描结果（对齐 `LaserScan`）
#[derive(Debug, Clone)]
pub struct LaserScan {
    /// 点云数据
    pub points: Vec<LaserPoint>,
    /// 时间戳 (纳秒)
    pub stamp: u64,
    /// 扫描频率 (Hz)
    pub scan_freq: f32,
    /// 一圈总时间 (秒)
    pub scan_time: f32,
}

impl Default for LaserScan {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            stamp: 0,
            scan_freq: 0.0,
            scan_time: 0.0,
        }
    }
}

/// 原始扫描数据包（对齐 C++ scan data packet）
#[derive(Debug, Clone)]
pub struct ScanPacket {
    /// 原始字节
    pub raw: Vec<u8>,
    /// 时间戳 (纳秒)
    pub stamp: u64,
}

/// 一帧完整扫描的元数据
#[derive(Debug, Clone, Default)]
pub struct ScanConfig {
    /// 最小角度 (弧度)
    pub min_angle: f32,
    /// 最大角度 (弧度)
    pub max_angle: f32,
    /// 角度增量 (弧度)
    pub angle_increment: f32,
    /// 扫描时间 (秒)
    pub scan_time: f32,
    /// 最小距离 (米)
    pub min_range: f32,
    /// 最大距离 (米)
    pub max_range: f32,
}
