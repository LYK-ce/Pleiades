//Presented by KeJi
//Date ： 2026-05-25

//! Session 流协议模块
//!
//! 用于远端 Chat ↔ 本地 Session 的持久双向流通信。
//! 与 Tensor Stream 不同，Session 流通过 Network_Inbound_Event 路由到 Core B3，
//! 再由 Core 分配 slot 建立 bridge，实现 stream ↔ mpsc 的双向 relay。
//!
//! ## 协议格式
//!
//! Handshake（发起方→接收方，流建立时立即发送）:
//! +------------------------+
//! |   session_id           |
//! |   8 bytes u64 BE       |
//! +------------------------+
//!
//! 数据帧（双向）:
//! +------------------------+---------------------+
//! |   Payload Length       |   Payload (UTF-8)   |
//! |   4 bytes u32 BE       |   Length bytes      |
//! +------------------------+---------------------+

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use futures::prelude::*;
use std::io;

// ===== 协议标识符 =====
pub const SESSION_STREAM_PROTOCOL: &str = "/pleiades/session/1.0.0";

// ===== 最大帧大小 (64KB) =====
const MAX_FRAME_SIZE: u32 = 64 * 1024;

// ============================================================
// Handshake 读写
// ============================================================

/// 写入 Session Stream Handshake（8 字节 u64 BE session_id）
pub async fn Write_Session_Handshake(
    stream: &mut libp2p::Stream,
    session_id: u64,
) -> io::Result<()> {
    stream.write_all(&session_id.to_be_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// 读取 Session Stream Handshake（8 字节 u64 BE session_id）
pub async fn Read_Session_Handshake(
    stream: &mut libp2p::Stream,
) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    stream.read_exact(&mut buf).await?;
    Ok(u64::from_be_bytes(buf))
}

// ============================================================
// 数据帧读写
// ============================================================

/// 写入 Session 数据帧: [4B BE u32 len][UTF-8 payload]
pub async fn write_session_frame(
    stream: &mut libp2p::Stream,
    text: &str,
) -> io::Result<()> {
    let payload = text.as_bytes();
    if payload.len() > MAX_FRAME_SIZE as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Session 帧过大: {} bytes (max {})", payload.len(), MAX_FRAME_SIZE),
        ));
    }
    stream.write_all(&(payload.len() as u32).to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

/// 读取 Session 数据帧: [4B BE u32 len][UTF-8 payload]
pub async fn read_session_frame(
    stream: &mut libp2p::Stream,
) -> io::Result<String> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_FRAME_SIZE as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Session 帧过大: {} bytes (max {})", len, MAX_FRAME_SIZE),
        ));
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;

    String::from_utf8(buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
