//Presented by KeJi
//Created Date ： 2026-08-31
//Modified Date ： 2026-08-31

//! RTCM 3.x 二进制帧切分器（Task 24，基站侧）
//!
//! RTCM 3.x 无换行分界，串口 read 分块随机。本解析器攒缓冲、按帧头 `0xD3` + 长度字段切帧。
//! 不校验 CRC（CRC 由 LG290P 内部校验）。

/// 缓冲上限：超限丢最旧字节重同步（防止坏帧导致无限攒）
pub const RTCM_BUFFER_MAX: usize = 4096;

/// RTCM 3.x 帧切分器
pub struct Rtc3Parser {
    buf: Vec<u8>,
}

impl Rtc3Parser {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// 清空缓冲（切 Broadcasting 时丢弃 GGA 残留）
    pub fn reset(&mut self) {
        self.buf.clear();
    }

    /// 喂入一块字节，返回本次切出的所有完整 RTCM 帧。
    ///
    /// RTCM 3.x 帧结构：`0xD3 | byte1 | byte2 | payload(length) | crc(3)`，
    /// 其中 `length = ((byte1 & 0x03) << 8) | byte2`，完整帧长 = `length + 6`。
    /// 帧头前的杂散字节丢弃；坏长度跳过一个字节重同步；缓冲超上限时清空重同步。
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();

        loop {
            // 找帧头 0xD3
            let Some(pos) = self.buf.iter().position(|&b| b == 0xD3) else {
                // 无帧头：全是杂散（GGA 残留等）。超上限则清空，否则保留等下一块
                if self.buf.len() > RTCM_BUFFER_MAX {
                    self.buf.clear();
                }
                break;
            };
            // 丢弃帧头前的杂散字节
            if pos > 0 {
                self.buf.drain(..pos);
            }
            // 头不足 3 字节，等下一块
            if self.buf.len() < 3 {
                break;
            }
            let length = (((self.buf[1] & 0x03) as usize) << 8) | (self.buf[2] as usize);
            let total = length + 6;
            // 坏长度（超上限）→ 跳过一个字节重同步
            if total > RTCM_BUFFER_MAX {
                self.buf.drain(..1);
                continue;
            }
            // 帧不完整，等下一块
            if self.buf.len() < total {
                break;
            }
            // 切出完整帧
            let frame = self.buf[..total].to_vec();
            self.buf.drain(..total);
            frames.push(frame);
        }

        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_frame() -> Vec<u8> {
        // length = 2（byte1=0x00, byte2=0x02），payload 2B + crc 3B = 总长 8
        vec![0xD3, 0x00, 0x02, 0xAA, 0xBB, 0x00, 0x00, 0x00]
    }

    #[test]
    fn test_feed_single_frame() {
        let mut p = Rtc3Parser::new();
        let frame = sample_frame();
        let frames = p.feed(&frame);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], frame);
    }

    #[test]
    fn test_feed_fragmented() {
        let mut p = Rtc3Parser::new();
        let frame = sample_frame();
        assert!(p.feed(&frame[..3]).is_empty());
        assert!(p.feed(&frame[3..5]).is_empty());
        let frames = p.feed(&frame[5..]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], frame);
    }

    #[test]
    fn test_feed_drops_garbage_before_sync() {
        let mut p = Rtc3Parser::new();
        let frame = sample_frame();
        let mut input = b"$GNGGA".to_vec(); // GGA 杂散
        input.extend_from_slice(&frame);
        let frames = p.feed(&input);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], frame);
    }

    #[test]
    fn test_feed_multiple_frames_in_one_chunk() {
        let mut p = Rtc3Parser::new();
        let frame = sample_frame();
        let mut input = Vec::new();
        input.extend_from_slice(&frame);
        input.extend_from_slice(&frame);
        let frames = p.feed(&input);
        assert_eq!(frames.len(), 2);
    }
}
