//Presented by KeJi
//Created Date ： 2026-07-20
//Modified Date ： 2026-07-20
//
//Vendored from YDLidar-SDK/rust/tmini-protocol/src/constants.rs
//对齐 C++ `ydlidar_protocol.h` 和 `YDlidarDriver.cpp`

// ─── 命令码 ──────────────────────────────────────────
pub const CMD_STOP: u8 = 0x65;
pub const CMD_SCAN: u8 = 0x60;
pub const CMD_FORCE_SCAN: u8 = 0x61;
pub const CMD_RESET: u8 = 0x80;
pub const CMD_FORCE_STOP: u8 = 0x00;

// ─── 协议头 ──────────────────────────────────────────
pub const PHA5: u8 = 0xA5; // 命令 同步字节
pub const PH5A: u8 = 0x5A; // 响应 同步字节2

// ─── 数据包头 ────────────────────────────────────────
pub const PH1: u8 = 0xAA; // 扫描数据包头字节0
pub const PH2: u8 = 0x55; // 扫描数据包头字节1  (注释: AA55是点云数据)
pub const PH3: u8 = 0x66; // 时间戳数据包头

// ─── CT 标志位 ───────────────────────────────────────
pub const LIDAR_RESP_SYNCBIT: u8 = 0x01; // CT 字节 bit[0] = 零位包标记
pub const LIDAR_RESP_CHECKBIT: u8 = 0x01; // 角度字节 bit[0] 必须为 1

// ─── 响应类型 ────────────────────────────────────────
pub const ANS_TYPE_MEASUREMENT: u8 = 0x81;
pub const ANS_TYPE_DEVINFO: u8 = 0x04;
pub const ANS_TYPE_DEVHEALTH: u8 = 0x06;

// ─── 包尺寸 ──────────────────────────────────────────
pub const TRI_PACKHEADSIZE: usize = 10; // 包头 10 字节
pub const TRI_PACKMAXNODES: usize = 80; // 单包最大点数

// ─── 强度位数 ────────────────────────────────────────
pub const NODE_QUAL0: u8 = 0;
pub const NODE_QUAL8: u8 = 8;
pub const NODE_QUAL10: u8 = 10;
pub const NODE_QUAL16: u8 = 16;

// ─── 阳光/玻璃噪声标记 ─────────────────────────────────
pub const SUNNOISEINTENSITY: u8 = 0x03;
pub const GLASSNOISEINTENSITY: u8 = 0x02;

// ─── 角度位移 ────────────────────────────────────────
pub const LIDAR_RESP_ANGLE_SHIFT: u32 = 1;

// ─── 换算因子 ────────────────────────────────────────
pub const SDK_UNIT64: f32 = 64.0;
pub const SDK_UNIT128: f32 = 128.0;
pub const SDK_ANGLE360: f32 = 360.0;
pub const SDK_ANGLE180: f32 = 180.0;

// ─── 时间戳包尺寸 ─────────────────────────────────────
pub const STAMPPACKSIZE: usize = 8;

// ─── Tmini 默认参数 ──────────────────────────────────
pub const TMINI_DEFAULT_BAUDRATE: u32 = 230400;
pub const TMINI_MODEL_ID: u8 = 140;
pub const TMINI_SAMPLE_RATE: u32 = 4; // 4K
pub const TMINI_INTENSITY_BIT: u8 = NODE_QUAL8;
