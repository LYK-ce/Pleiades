//Presented by KeJi
//Created Date ： 2026-08-07
//Modified Date ： 2026-08-10

//! 帧编解码（MAVLink 风格 + 按需扩展，协议文档 §2）
//!
//! 帧结构（Task 13 阶段一：sysid 1B → 完整 peer_id 变长）：
//! ```
//! magic(1B 0x4F) | len(4B BE u32) | seq(1B) | sysid_len(1B) | sysid(N) | compid(1B) | msgid(2B BE) | payload(len) | checksum(2B)
//! ```
//!
//! - `len` 放宽为 4 字节（原始 MAVLink 255B 上限无法承载全量栅格地图）
//! - `seq` / `checksum` 第一版恒填 0（libp2p/TCP 可靠传输，2026-08-07 决策）
//! - `sysid` = 发送方完整 libp2p PeerId 二进制（multihash，Ed25519 下 38B）；
//!   `sysid_len` = 0 表示无身份（地面站上行命令，配合 compid=200 使用）

/// 帧 magic 字节（'O' = Orion）
pub const MAGIC: u8 = 0x4F;

/// 固定头长度（不含 sysid 与 payload/checksum）：
/// magic(1) + len(4) + seq(1) + sysid_len(1) + compid(1) + msgid(2) = 10
pub const FRAME_FIXED_HEADER_LEN: usize = 10;

/// 解码后的帧
#[derive(Debug, Clone)]
pub struct Frame {
    pub msgid: u16,
    /// 完整 peer_id 二进制（libp2p PeerId multihash）；空 = 无身份（地面站上行）
    pub sysid: Vec<u8>,
    pub compid: u8,
    pub payload: Vec<u8>,
}

/// 编码一帧（seq 恒 0，checksum 恒 0，第一版）
pub fn encode_frame(msgid: u16, sysid: &[u8], compid: u8, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FRAME_FIXED_HEADER_LEN + sysid.len() + payload.len() + 2);
    buf.push(MAGIC);
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes()); // len u32 BE
    buf.push(0); // seq（第一版恒 0）
    buf.push(sysid.len() as u8); // sysid_len（u8，peer_id ≤ 255B）
    buf.extend_from_slice(sysid);
    buf.push(compid);
    buf.extend_from_slice(&msgid.to_be_bytes()); // msgid u16 BE
    buf.extend_from_slice(payload);
    buf.extend_from_slice(&[0u8, 0u8]); // checksum（第一版恒 0）
    buf
}

/// 解码一帧：校验 magic / len，返回 Frame（含完整 sysid 字节）
pub fn decode_frame(bytes: &[u8]) -> Option<Frame> {
    // 最小长度：固定头(10) + checksum(2) = 12（sysid 为空时）
    if bytes.len() < FRAME_FIXED_HEADER_LEN + 2 {
        return None;
    }
    if bytes[0] != MAGIC {
        return None;
    }
    let len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
    let sysid_len = bytes[6] as usize;
    let header_len = FRAME_FIXED_HEADER_LEN + sysid_len;
    if bytes.len() != header_len + len + 2 {
        return None;
    }
    let _seq = bytes[5]; // 第一版不校验
    let sysid = bytes[7..7 + sysid_len].to_vec();
    let compid = bytes[7 + sysid_len];
    let msgid = u16::from_be_bytes([bytes[8 + sysid_len], bytes[9 + sysid_len]]);
    let payload = bytes[header_len..header_len + len].to_vec();
    Some(Frame {
        msgid,
        sysid,
        compid,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用假 peer_id（模拟 Ed25519 multihash 前缀 + 内容）
    const TEST_PEER: &[u8] = &[0x00, 0x24, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

    #[test]
    fn test_frame_roundtrip() {
        let payload = [1u8, 2, 3, 4, 5];
        let frame = encode_frame(3, TEST_PEER, 1, &payload);
        let decoded = decode_frame(&frame).expect("解码应成功");
        assert_eq!(decoded.msgid, 3);
        assert_eq!(decoded.sysid, TEST_PEER);
        assert_eq!(decoded.compid, 1);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_frame_empty_identity() {
        // 空身份（地面站上行）：sysid_len=0 + compid=200
        let frame = encode_frame(4, &[], 200, &[1, 2]);
        let decoded = decode_frame(&frame).expect("解码应成功");
        assert!(decoded.sysid.is_empty());
        assert_eq!(decoded.compid, 200);
        assert_eq!(decoded.payload, vec![1, 2]);
    }

    #[test]
    fn test_frame_bad_magic() {
        let frame = encode_frame(1, &[], 0, &[]);
        assert!(decode_frame(&frame[1..]).is_none());
    }

    #[test]
    fn test_frame_len_mismatch() {
        let frame = encode_frame(1, TEST_PEER, 0, &[1, 2, 3]);
        // 截断一字节 → len 字段与内容不符
        assert!(decode_frame(&frame[..frame.len() - 1]).is_none());
    }

    #[test]
    fn test_frame_empty_payload() {
        let frame = encode_frame(5, TEST_PEER, 200, &[]);
        let decoded = decode_frame(&frame).unwrap();
        assert_eq!(decoded.msgid, 5);
        assert!(decoded.payload.is_empty());
        assert_eq!(decoded.sysid, TEST_PEER);
    }
}
