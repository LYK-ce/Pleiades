//Presented by KeJi
//Date ： 2026-05-16

//! 带宽测速流协议
//!
//! iperf 风格固定时长推流测速。发送方推送固定时长（默认 3 秒），
//! 接收方只计数字节、期满回传总量。单次出结果，无需多档逼近。
//!
//! ## 线上帧格式
//! ```text
//! Sender → Receiver: [2B duration LE][持续 64KB zero chunks][close]
//! Receiver → Sender: [8B total_bytes LE]
//! ```

use futures::prelude::*;
use std::io;

// ===== 协议标识符 =====
pub const BANDWIDTH_STREAM_PROTOCOL: &str = "/pleiades/bandwidth/1.0.0";

// ===== 分块大小 (64KB) =====
pub const CHUNK_SIZE: usize = 64 * 1024;

// ===== 时长常量 =====
/// 默认测试时长（秒）
pub const DEFAULT_DURATION_SECS: u16 = 3;
/// 最大测试时长（秒），防止恶意节点指定超长时长
pub const MAX_DURATION_SECS: u16 = 10;

// ============================================================
// Send_Bandwidth_Test — 发送方推送数据
// ============================================================

/// 发起带宽测试：写 duration，持续推送 64KB zero chunk，关闭写端
///
/// 写入格式: [2B duration LE][持续 64KB chunks][close]
///
/// # 参数
/// - `stream`: 已打开的出站流
/// - `duration_secs`: 测试时长（秒），调用方保证 ≤ MAX_DURATION_SECS
pub async fn Send_Bandwidth_Test(
    stream: &mut libp2p::Stream,
    duration_secs: u16,
) -> io::Result<()> {
    stream.write_all(&duration_secs.to_le_bytes()).await?;
    stream.flush().await?;

    let chunk = vec![0u8; CHUNK_SIZE];
    let deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_secs(duration_secs as u64);

    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        stream.write_all(&chunk).await?;
    }

    stream.flush().await?;
    stream.close().await?;
    Ok(())
}

// ============================================================
// Receive_And_Count — 接收方计数
// ============================================================

/// 接收带宽测试数据并累加字节数
///
/// 读取格式: [2B duration LE]，然后持续读取 chunk 直到 read() == 0
///
/// # 返回
/// total_bytes（u64）—— 实际收到的数据字节数（不含 2 字节 header）
pub async fn Receive_And_Count(
    stream: &mut libp2p::Stream,
) -> io::Result<u64> {
    let mut dur_buf = [0u8; 2];
    stream.read_exact(&mut dur_buf).await?;
    let duration_secs = u16::from_le_bytes(dur_buf);

    if duration_secs > MAX_DURATION_SECS {
        let _ = stream.close().await;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("duration {}s exceeds max {}", duration_secs, MAX_DURATION_SECS),
        ));
    }

    let mut total: u64 = 0;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }

    Ok(total)
}

// ============================================================
// Write_Bandwidth_Result / Read_Bandwidth_Result — 结果回传
// ============================================================

/// 写入测速结果（接收方调用）
pub async fn Write_Bandwidth_Result(
    stream: &mut libp2p::Stream,
    total_bytes: u64,
) -> io::Result<()> {
    stream.write_all(&total_bytes.to_le_bytes()).await?;
    stream.flush().await
}

/// 读取测速结果（发送方调用）
pub async fn Read_Bandwidth_Result(
    stream: &mut libp2p::Stream,
) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    stream.read_exact(&mut buf).await?;
    Ok(u64::from_le_bytes(buf))
}

// ============================================================
// Run_Bandwidth_Test — 完整单次测速（发送方调用）
// ============================================================

/// 执行一次完整带宽测试并返回 Mbps
///
/// 打开流已由调用方完成，本函数负责推送数据 → 读取结果 → 计算 Mbps。
///
/// # 参数
/// - `stream`: 已打开的出站流
/// - `duration_secs`: 测试时长（秒）
///
/// # 返回
/// 带宽（Mbps，整数）
pub async fn Run_Bandwidth_Test(
    stream: &mut libp2p::Stream,
    duration_secs: u16,
) -> io::Result<u64> {
    Send_Bandwidth_Test(stream, duration_secs).await?;
    let total_bytes = Read_Bandwidth_Result(stream).await?;
    let mbps = (total_bytes as f64 * 8.0) / duration_secs as f64 / 1_000_000.0;
    Ok(mbps as u64)
}
