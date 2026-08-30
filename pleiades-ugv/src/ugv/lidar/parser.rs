//Presented by KeJi
//Created Date ： 2026-07-20
//Modified Date ： 2026-07-20
//
//Vendored from YDLidar-SDK/rust/tmini-protocol/src/parser.rs
//对齐 C++ `YDlidarDriver::parseData()` + `parsePoints()` + `CYdLidar::doProcessSimple()`

use super::constants::*;
use super::types::*;
use super::checksum::verify_check_sum;

/// 状态机状态（对齐 C++ `parseData()` 中的 `recvPos`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RxPos {
    /// case 0: 等待 PH1 (0xAA)
    Ph1,
    /// case 1: PH2 (0x55) 或时间戳 (0x66) 或连续 PH1
    Ph2,
    /// case 2: CT 字节
    Ct,
    /// case 3: LSN 采样点数
    Lsn,
    /// case 4-5: FSA 起始角 (2 字节, bit[0] 必须为 1)
    FsaLo,
    FsaHi,
    /// case 6-7: LSA 结束角
    LsaLo,
    LsaHi,
    /// case 8-9: CS 校验和 (2 字节)
    CsLo,
    CsHi,
    /// 收集采样数据字节中 (header done, collecting sample bytes)
    Samples,
}

/// 解析器状态
pub struct ParseState {
    pos: RxPos,
    buf: Vec<u8>,
    /// 当前包中累计的字段
    ct: u8,
    lsn: u8,
    fsa_raw: u16,
    lsa_raw: u16,
    cs: u16,
    /// 零位包标记
    zero: bool,
    /// 收到的完整包列表
    packets: Vec<ScanPacket>,
    /// 当前时间戳 (纳秒)
    stamp: u64,
    /// 阻塞检测: 连续收到 PHA5 的次数
    block_rev_size: u8,
    /// 是否有阻塞错误
    block_error: bool,
    /// 采样数据待收集的字节数 (Samples 状态用)
    samples_remaining: usize,
}

impl ParseState {
    pub fn new() -> Self {
        Self {
            pos: RxPos::Ph1,
            buf: Vec::with_capacity(1024),
            ct: 0,
            lsn: 0,
            fsa_raw: 0,
            lsa_raw: 0,
            cs: 0,
            zero: false,
            packets: Vec::new(),
            stamp: 0,
            block_rev_size: 0,
            block_error: false,
            samples_remaining: 0,
        }
    }

    /// 获取已缓存的完整包数量
    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    /// 获取并清空已缓存的包
    pub fn take_packets(&mut self) -> Vec<ScanPacket> {
        std::mem::take(&mut self.packets)
    }

    /// 检查是否检测到阻塞错误 (连续收到 PHA5)
    pub fn is_blocked(&self) -> bool {
        self.block_error
    }

    /// 清除阻塞错误标记
    pub fn clear_block_error(&mut self) {
        self.block_error = false;
    }
}

/// 逐字节喂入状态机
///
/// 对齐 C++ `YDlidarDriver::parseData()` 中的 switch(recvPos) 状态机。
/// 收到完整包时返回 `Some(())` 表示有数据可用。
pub fn feed_byte(state: &mut ParseState, byte: u8) -> Option<()> {
    // ── 阻塞检测 ────────────────────────────────────────
    check_block_status(state, byte);

    match state.pos {
        // case 0: 等待 PH1 (0xAA)
        RxPos::Ph1 => {
            if byte != PH1 {
                return None;
            }
            state.pos = RxPos::Ph2;
            state.buf.push(byte);
        }

        // case 1: PH2 (0x55) 或 时间戳 或 连续 PH1
        RxPos::Ph2 => {
            if byte == PH2 {
                if state.block_error {
                    state.block_error = false;
                }
                state.pos = RxPos::Ct;
                state.buf.push(byte);
            } else if byte == PH1 {
                return None;
            } else if byte == PH3 {
                state.pos = RxPos::Ph1;
                return None;
            } else {
                state.pos = RxPos::Ph1;
                state.buf.clear();
                return None;
            }
        }

        // case 2: CT 字节
        RxPos::Ct => {
            state.zero = (byte & LIDAR_RESP_SYNCBIT) != 0;
            state.ct = byte;
            state.pos = RxPos::Lsn;
            state.buf.push(byte);
        }

        // case 3: LSN 采样点数
        RxPos::Lsn => {
            if byte as usize > TRI_PACKMAXNODES {
                state.pos = RxPos::Ph1;
                state.buf.clear();
                return None;
            }
            state.lsn = byte;
            state.pos = RxPos::FsaLo;
            state.buf.push(byte);
        }

        // case 4: FSA 低字节 (bit[0] 必须为 1)
        RxPos::FsaLo => {
            if byte & LIDAR_RESP_CHECKBIT == 0 {
                state.pos = RxPos::Ph1;
                state.buf.clear();
                return None;
            }
            state.fsa_raw = byte as u16;
            state.pos = RxPos::FsaHi;
            state.buf.push(byte);
        }

        // case 5: FSA 高字节
        RxPos::FsaHi => {
            state.fsa_raw |= (byte as u16) << 8;
            state.fsa_raw >>= 1;
            state.pos = RxPos::LsaLo;
            state.buf.push(byte);
        }

        // case 6: LSA 低字节
        RxPos::LsaLo => {
            if byte & LIDAR_RESP_CHECKBIT == 0 {
                state.pos = RxPos::Ph1;
                state.buf.clear();
                return None;
            }
            state.lsa_raw = byte as u16;
            state.pos = RxPos::LsaHi;
            state.buf.push(byte);
        }

        // case 7: LSA 高字节
        RxPos::LsaHi => {
            state.lsa_raw |= (byte as u16) << 8;
            state.lsa_raw >>= 1;
            state.pos = RxPos::CsLo;
            state.buf.push(byte);
        }

        // case 8: CS 低字节
        RxPos::CsLo => {
            state.cs = byte as u16;
            state.pos = RxPos::CsHi;
            state.buf.push(byte);
        }

        // case 9: CS 高字节 → 包头完整，转采样数据收集
        RxPos::CsHi => {
            state.cs |= (byte as u16) << 8;
            state.buf.push(byte);

            let point_bytes = 3;
            state.samples_remaining = state.lsn as usize * point_bytes;

            if state.samples_remaining == 0 {
                state.pos = RxPos::Ph1;
                let raw = std::mem::take(&mut state.buf);
                if !verify_check_sum(&raw, NODE_QUAL8) {
                    return None;
                }
                state.packets.push(ScanPacket { raw, stamp: state.stamp, zero: state.zero });
                return Some(());
            }

            state.pos = RxPos::Samples;
        }

        // 收集采样数据
        RxPos::Samples => {
            state.buf.push(byte);
            state.samples_remaining -= 1;
            if state.samples_remaining == 0 {
                state.pos = RxPos::Ph1;
                let raw = std::mem::take(&mut state.buf);
                if !verify_check_sum(&raw, NODE_QUAL8) {
                    return None;
                }
                state.packets.push(ScanPacket { raw, stamp: state.stamp, zero: state.zero });
                return Some(());
            }
        }
    }

    None
}

/// 阻塞检测 — 对齐 C++ `checkBlockStatus`
fn check_block_status(state: &mut ParseState, byte: u8) {
    match state.block_rev_size {
        0 => {
            if byte == PHA5 {
                state.block_rev_size += 1;
            }
        }
        1 => {
            if byte == PH5A {
                state.block_error = true;
            }
            state.block_rev_size = 0; // 无论匹配与否都重置
        }
        _ => {}
    }
}

/// 解析累积的扫描包为点云 — 对齐 C++ `YDlidarDriver::parsePoints()`
///
/// 返回 Vec<NodeInfo>，不进行角度/距离转换（保持原始值）
pub fn parse_points(packets: &[ScanPacket], intensity_bit: u8) -> Vec<NodeInfo> {
    let mut nodes = Vec::new();

    for packet in packets {
        let data = &packet.raw;
        if data.len() < TRI_PACKHEADSIZE {
            continue;
        }

        let _head = u16::from_le_bytes([data[0], data[1]]);
        let freq = data[2] >> 1;
        let count = data[3] as usize;

        let angle = u16::from_le_bytes([data[4], data[5]]);
        let sa: f32 = (angle >> 1) as f32 / SDK_UNIT64;

        let angle = u16::from_le_bytes([data[6], data[7]]);
        let mut ea: f32 = (angle >> 1) as f32 / SDK_UNIT64;

        if ea < sa {
            ea += SDK_ANGLE360;
        }

        let step: f32 = if count > 1 {
            (ea - sa).abs() / (count as f32 - 1.0)
        } else {
            0.0
        };

        let mut offset = TRI_PACKHEADSIZE;
        for i in 0..count {
            if offset + 2 >= data.len() {
                break;
            }

            let angle_deg: f32 = sa + step * i as f32;

            let (qual, dist, is_flag) = match intensity_bit {
                NODE_QUAL16 => {
                    let p = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    offset += 2;
                    let d = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    offset += 2;
                    (p, d, 0u8)
                }
                NODE_QUAL10 => {
                    let p2 = data[offset];
                    offset += 1;
                    let d_raw = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    offset += 2;
                    let p = p2 as u16 | ((d_raw & 0x0003) << 8);
                    let d = d_raw & 0xFFFC;
                    (p, d, 0u8)
                }
                NODE_QUAL8 => {
                    let p2 = data[offset];
                    offset += 1;
                    let d_raw = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    offset += 2;
                    let p = p2 as u16;
                    let is_val = (d_raw & 0x0003) as u8;
                    let d = d_raw & 0xFFFC;
                    (p, d, is_val)
                }
                _ => {
                    let d = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    offset += 2;
                    (0u16, d, 0u8)
                }
            };

            // ── 角度二级校正 ──
            // Tmini 跳过二级角度校正
            let ca: f32 = 0.0;

            let mut a = angle_deg + ca;
            if a > SDK_ANGLE360 {
                a -= SDK_ANGLE360;
            }
            let node = NodeInfo {
                sync: if i == 0 { 1 } else { 0 },
                is: is_flag,
                qual,
                angle: (a * SDK_UNIT128) as u16,
                dist,
                stamp: packet.stamp,
                delay_time: 0,
                scan_freq: freq,
            };
            nodes.push(node);
        }
    }

    nodes
}

/// 将 NodeInfo 转换为 LaserScan — 对齐 C++ `CYdLidar::doProcessSimple()`
///
/// 参数:
/// - nodes: parse_points 的输出
/// - angle_offset: 角度偏移 (度)
/// - fixed_resolution: 是否使用固定分辨率
/// - fixed_size: 固定分辨率下的点数
/// - fov: 视场角 (度)
pub fn do_process_simple(
    nodes: &[NodeInfo],
    angle_offset: f32,
    fixed_resolution: bool,
    fixed_size: usize,
    fov: f32,
) -> LaserScan {
    let mut scan = LaserScan::default();
    if nodes.is_empty() {
        return scan;
    }

    let count = nodes.len();
    let all_node_count = if fixed_resolution {
        fixed_size
    } else {
        count
    };

    let _angle_increment = if all_node_count > 1 {
        (fov.to_radians()) / (all_node_count as f32 - 1.0)
    } else {
        0.0
    };

    let mut scanfreq: f32 = 0.0;

    for node in nodes.iter() {
        let angle_deg = (node.angle >> LIDAR_RESP_ANGLE_SHIFT) as f32 / SDK_UNIT64 + angle_offset;
        let angle = angle_deg.to_radians();

        let range = node.dist as f32 / 4000.0;

        let intensity = node.qual as f32;

        if node.scan_freq != 0 {
            scanfreq = node.scan_freq as f32 / 10.0;
        }

        scan.points.push(LaserPoint {
            angle,
            range,
            intensity,
        });
    }

    scan.stamp = nodes.first().map(|n| n.stamp).unwrap_or(0);
    scan.scan_freq = scanfreq;
    scan.scan_time = if scanfreq > 0.0 { 1.0 / scanfreq } else { 0.0 };

    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_state_machine_simple() {
        let mut state = ParseState::new();

        let lsn: u8 = 2;
        let ct: u8 = (10 * 10) << 1;
        let fsa_raw = ((0.0f32 * 64.0) as u16) << 1 | 1;
        let lsa_raw = ((0.9f32 * 64.0) as u16) << 1 | 1;

        let mut pkt = vec![PH1, PH2, ct, lsn];
        pkt.extend_from_slice(&fsa_raw.to_le_bytes());
        pkt.extend_from_slice(&lsa_raw.to_le_bytes());
        pkt.extend_from_slice(&0u16.to_le_bytes());

        for _ in 0..lsn {
            pkt.push(200);
            pkt.extend_from_slice(&1000u16.to_le_bytes());
        }

        crate::ugv::lidar::checksum::fill_check_sum(&mut pkt, NODE_QUAL8);

        let mut got_packet = false;
        for &b in &pkt {
            if feed_byte(&mut state, b).is_some() {
                got_packet = true;
            }
        }

        assert!(got_packet, "应该解析出一个完整包");
        assert_eq!(state.packet_count(), 1);

        let packets = state.take_packets();
        let nodes = parse_points(&packets, NODE_QUAL8);
        assert_eq!(nodes.len(), lsn as usize);
    }

    #[test]
    fn test_parse_rejects_bad_checksum() {
        let mut state = ParseState::new();

        let lsn: u8 = 2;
        let ct: u8 = (10 * 10) << 1;
        let fsa_raw = ((0.0f32 * 64.0) as u16) << 1 | 1;
        let lsa_raw = ((0.9f32 * 64.0) as u16) << 1 | 1;

        let mut pkt = vec![PH1, PH2, ct, lsn];
        pkt.extend_from_slice(&fsa_raw.to_le_bytes());
        pkt.extend_from_slice(&lsa_raw.to_le_bytes());
        // 故意填错误的 CS
        pkt.extend_from_slice(&0xFFFFu16.to_le_bytes());

        for _ in 0..lsn {
            pkt.push(200);
            pkt.extend_from_slice(&1000u16.to_le_bytes());
        }

        for &b in &pkt {
            feed_byte(&mut state, b);
        }

        assert_eq!(state.packet_count(), 0, "坏校验和的包应被丢弃");
    }
}
