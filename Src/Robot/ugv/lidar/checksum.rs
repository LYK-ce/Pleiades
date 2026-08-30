//Presented by KeJi
//Created Date ： 2026-07-20
//Modified Date ： 2026-07-20
//
//Vendored from YDLidar-SDK/rust/tmini-protocol/src/checksum.rs
//对齐 C++ `YDlidarDriver::calcCheckSum`

use crate::robot::ugv::lidar::constants::{NODE_QUAL8, NODE_QUAL10, NODE_QUAL16, TRI_PACKHEADSIZE};

/// 计算扫描数据包的 XOR 校验和
///
/// 对齐 C++ `YDlidarDriver::calcCheckSum()`:
/// - 包头 4 个 uint16_t LE 字 XOR
/// - 每个采样点按强度位数分别处理
pub fn calc_check_sum(data: &[u8], intensity_bit: u8) -> (u16, u16) {
    let mut ccs: u16 = 0;

    // ── 包头 4 个 uint16 字 (PH, CT+LSN, FSA, LSA) ──
    for i in 0..4 {
        let word = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
        ccs ^= word;
    }

    let ocs = u16::from_le_bytes([data[8], data[9]]);

    // ── 采样数据 ──
    let count = data[3] as usize; // LSN
    let mut offset = TRI_PACKHEADSIZE;

    for _ in 0..count {
        if offset >= data.len() {
            break;
        }
        match intensity_bit {
            NODE_QUAL16 => {
                if offset + 1 < data.len() {
                    let word = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    ccs ^= word;
                    offset += 2;
                }
                if offset + 1 < data.len() {
                    let word = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    ccs ^= word;
                    offset += 2;
                }
            }
            NODE_QUAL8 | NODE_QUAL10 => {
                ccs ^= data[offset] as u16;
                offset += 1;
                if offset + 1 < data.len() {
                    let word = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    ccs ^= word;
                    offset += 2;
                }
            }
            _ => {
                if offset + 1 < data.len() {
                    let word = u16::from_le_bytes([data[offset], data[offset + 1]]);
                    ccs ^= word;
                    offset += 2;
                }
            }
        }
    }

    (ccs, ocs)
}

/// 验证校验和是否正确
pub fn verify_check_sum(data: &[u8], intensity_bit: u8) -> bool {
    let (ccs, ocs) = calc_check_sum(data, intensity_bit);
    ccs == ocs
}

/// 计算并填入校验和到包中 (用于构建数据包)
///
/// 传入包（CS 字段已置零），计算后回填到 bytes 8-9
pub fn fill_check_sum(packet: &mut [u8], intensity_bit: u8) {
    packet[8] = 0;
    packet[9] = 0;
    let (cs, _) = calc_check_sum(packet, intensity_bit);
    packet[8] = cs as u8;
    packet[9] = (cs >> 8) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::ugv::lidar::constants::*;

    #[test]
    fn test_checksum_roundtrip() {
        let lsn: u8 = 2;
        let fsa_deg: f32 = 0.0;
        let lsa_deg: f32 = 0.9;
        let ct: u8 = (10 * 10) << 1;

        let fsa_raw = ((fsa_deg * 64.0) as u16) << 1 | (LIDAR_RESP_CHECKBIT as u16);
        let lsa_raw = ((lsa_deg * 64.0) as u16) << 1 | (LIDAR_RESP_CHECKBIT as u16);

        let mut packet = vec![0u8; TRI_PACKHEADSIZE + lsn as usize * 3];

        packet[0] = PH1;
        packet[1] = PH2;
        packet[2] = ct;
        packet[3] = lsn;
        packet[4] = fsa_raw as u8;
        packet[5] = (fsa_raw >> 8) as u8;
        packet[6] = lsa_raw as u8;
        packet[7] = (lsa_raw >> 8) as u8;
        packet[8] = 0;
        packet[9] = 0;

        for i in 0..lsn as usize {
            let off = TRI_PACKHEADSIZE + i * 3;
            packet[off] = 200;
            let dist_raw: u16 = 1000;
            packet[off + 1] = dist_raw as u8;
            packet[off + 2] = (dist_raw >> 8) as u8;
        }

        fill_check_sum(&mut packet, NODE_QUAL8);
        assert!(verify_check_sum(&packet, NODE_QUAL8));

        let mut bad = packet.clone();
        bad[TRI_PACKHEADSIZE] ^= 0x01;
        assert!(!verify_check_sum(&bad, NODE_QUAL8));
    }
}
