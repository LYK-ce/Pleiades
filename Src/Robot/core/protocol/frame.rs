//Presented by KeJi
//Created Date ： 2026-08-07
//Modified Date ： 2026-08-07

//! 帧编解码（MAVLink 风格 + 按需扩展，协议文档 §2）
//!
//! 帧结构：
//! ```
//! magic(1B 0x4F) | len(4B BE u32) | seq(1B) | sysid(1B) | compid(1B) | msgid(2B BE) | payload(N) | checksum(2B)
//! ```
//!
//! - `len` 放宽为 4 字节（原始 MAVLink 255B 上限无法承载全量栅格地图）
//! - `seq` / `checksum` 第一版恒填 0（libp2p/TCP 可靠传输，2026-08-07 决策）

/// 帧 magic 字节（'O' = Orion）
pub const MAGIC: u8 = 0x4F;

/// 帧头长度（不含 payload 与 checksum）
pub const FRAME_HEADER_LEN: usize = 10;

/// 解码后的帧
#[derive(Debug, Clone)]
pub struct Frame {
    pub msgid: u16,
    pub sysid: u8,
    pub compid: u8,
    pub payload: Vec<u8>,
}

/// 编码一帧（seq 恒 0，checksum 恒 0，第一版）
pub fn encode_frame(msgid: u16, sysid: u8, compid: u8, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload.len() + 2);
    buf.push(MAGIC);
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes()); // len u32 BE
    buf.push(0); // seq（第一版恒 0）
    buf.push(sysid);
    buf.push(compid);
    buf.extend_from_slice(&msgid.to_be_bytes()); // msgid u16 BE
    buf.extend_from_slice(payload);
    buf.extend_from_slice(&[0u8, 0u8]); // checksum（第一版恒 0）
    buf
}

/// 解码一帧：校验 magic / len，返回 (msgid, sysid, compid, payload)
pub fn decode_frame(bytes: &[u8]) -> Option<Frame> {
    if bytes.len() < FRAME_HEADER_LEN + 2 {
        return None;
    }
    if bytes[0] != MAGIC {
        return None;
    }
    let len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
    if bytes.len() != FRAME_HEADER_LEN + len + 2 {
        return None;
    }
    let seq = bytes[5];
    let _ = seq; // 第一版不校验
    let sysid = bytes[6];
    let compid = bytes[7];
    let msgid = u16::from_be_bytes([bytes[8], bytes[9]]);
    let payload = bytes[FRAME_HEADER_LEN..FRAME_HEADER_LEN + len].to_vec();
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

    #[test]
    fn test_frame_roundtrip() {
        let payload = [1u8, 2, 3, 4, 5];
        let frame = encode_frame(3, 7, 1, &payload);
        let decoded = decode_frame(&frame).expect("解码应成功");
        assert_eq!(decoded.msgid, 3);
        assert_eq!(decoded.sysid, 7);
        assert_eq!(decoded.compid, 1);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_frame_bad_magic() {
        let frame = encode_frame(1, 0, 0, &[]);
        assert!(decode_frame(&frame[1..]).is_none());
    }

    #[test]
    fn test_frame_len_mismatch() {
        let frame = encode_frame(1, 0, 0, &[1, 2, 3]);
        // 截断一字节 → len 字段与内容不符
        assert!(decode_frame(&frame[..frame.len() - 1]).is_none());
    }

    #[test]
    fn test_frame_empty_payload() {
        let frame = encode_frame(5, 1, 200, &[]);
        let decoded = decode_frame(&frame).unwrap();
        assert_eq!(decoded.msgid, 5);
        assert!(decoded.payload.is_empty());
    }
}
