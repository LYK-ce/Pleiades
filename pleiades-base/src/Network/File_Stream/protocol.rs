//Presented by KeJi
//Date ： 2026-05-15

//! 文件流式传输协议
//!
//! 定义文件流式传输的 in-band header、ACK 握手和分块数据传输协议。
//! 位于 `File_Stream` 子目录下，与 `Tensor_Stream` 对称。
//!
//! ## In-band Header 方案
//!
//! 文件元数据（文件名、大小、校验和）通过流内 header 传输（与张量流 handshake 同构），
//! 接收方读取 header 后检查空间并回复 1-byte ACK，发送方收到 ACCEPT 后传输文件数据：
//!
//! ```text
//! Sender → Receiver: [2B name_len BE][name][8B file_size LE][2B checksum_len BE][checksum]
//! Receiver → Sender: [1B ACK]  (0x01=ACCEPT, 0x00=REJECT)
//! Sender → Receiver: raw file data (64KB chunks)
//! ```
//!
//! ## 数据传输函数
//! - `Write_File_Stream_Header` / `Read_File_Stream_Header`: header 读写
//! - `Write_File_Stream_Ack` / `Read_File_Stream_Ack`: ACK 读写
//! - `Send_File_Data`: 发送纯 raw data
//! - `Receive_File_Data`: 接收纯 raw data

use futures::prelude::*;
use std::io;
use std::path::Path;
use tracing::{debug, info};

use crate::event_bus::{Bus_Event, EventBus};

// ===== 协议标识符 =====
pub const FILE_STREAM_PROTOCOL: &str = "/pleiades/file-stream/1.0.0";

// ===== 分块大小 (64KB) =====
pub const CHUNK_SIZE: usize = 64 * 1024;

// ===== ACK 常量 =====
/// ACK 字节：接收方接受文件传输
pub const FILE_HEADER_ACCEPT: u8 = 0x01;
/// ACK 字节：接收方拒绝文件传输
pub const FILE_HEADER_REJECT: u8 = 0x00;

// ===== In-band Header 读写函数 =====

/// 写入 File Stream Header（发送方在 open_file_stream 后立即调用）
///
/// 格式: `[2B name_len BE][name UTF-8][8B file_size LE][2B checksum_len BE][checksum UTF-8]`
///
/// # 参数
/// - `stream`: 已打开的出站流
/// - `file_name`: 文件名（Storage file_id）
/// - `file_size`: 文件总大小（字节）
/// - `checksum`: 校验和字符串（如 "blake3:abcdef..."）
pub async fn Write_File_Stream_Header(
    stream: &mut libp2p::Stream,
    file_name: &str,
    file_size: u64,
    checksum: &str,
) -> io::Result<()> {
    let name_bytes = file_name.as_bytes();
    let checksum_bytes = checksum.as_bytes();

    // 1. 写入 name_len (2 bytes, u16 BE)
    let name_len = name_bytes.len() as u16;
    stream.write_all(&name_len.to_be_bytes()).await?;

    // 2. 写入 file_name
    stream.write_all(name_bytes).await?;

    // 3. 写入 file_size (8 bytes, u64 LE)
    stream.write_all(&file_size.to_le_bytes()).await?;

    // 4. 写入 checksum_len (2 bytes, u16 BE)
    let checksum_len = checksum_bytes.len() as u16;
    stream.write_all(&checksum_len.to_be_bytes()).await?;

    // 5. 写入 checksum
    stream.write_all(checksum_bytes).await?;

    // 6. flush
    stream.flush().await?;

    Ok(())
}

/// 读取 File Stream Header（接收方 Core 在 FileStreamArrived 时调用）
///
/// 格式: `[2B name_len BE][name UTF-8][8B file_size LE][2B checksum_len BE][checksum UTF-8]`
///
/// # 返回
/// `(file_name, file_size, checksum_string)`
pub async fn Read_File_Stream_Header(
    stream: &mut libp2p::Stream,
) -> io::Result<(String, u64, String)> {
    // 1. 读取 name_len (2 bytes, u16 BE)
    let mut name_len_buf = [0u8; 2];
    stream.read_exact(&mut name_len_buf).await?;
    let name_len = u16::from_be_bytes(name_len_buf) as usize;

    // 2. 读取 file_name
    let mut name_buf = vec![0u8; name_len];
    stream.read_exact(&mut name_buf).await?;
    let file_name = String::from_utf8(name_buf).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("file_name 非 UTF-8: {}", e))
    })?;

    // 3. 读取 file_size (8 bytes, u64 LE)
    let mut size_buf = [0u8; 8];
    stream.read_exact(&mut size_buf).await?;
    let file_size = u64::from_le_bytes(size_buf);

    // 4. 读取 checksum_len (2 bytes, u16 BE)
    let mut checksum_len_buf = [0u8; 2];
    stream.read_exact(&mut checksum_len_buf).await?;
    let checksum_len = u16::from_be_bytes(checksum_len_buf) as usize;

    // 5. 读取 checksum
    let mut checksum_buf = vec![0u8; checksum_len];
    stream.read_exact(&mut checksum_buf).await?;
    let checksum = String::from_utf8(checksum_buf).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("checksum 非 UTF-8: {}", e))
    })?;

    Ok((file_name, file_size, checksum))
}

/// 写入 ACK 字节（接收方检查空间后调用）
///
/// # 参数
/// - `stream`: 入站流
/// - `accepted`: true → ACCEPT (0x01), false → REJECT (0x00)
pub async fn Write_File_Stream_Ack(
    stream: &mut libp2p::Stream,
    accepted: bool,
) -> io::Result<()> {
    let ack = if accepted { FILE_HEADER_ACCEPT } else { FILE_HEADER_REJECT };
    stream.write_all(&[ack]).await?;
    stream.flush().await?;
    Ok(())
}

/// 读取 ACK 字节（发送方写完 header 后调用）
///
/// # 返回
/// `true` = ACCEPT, `false` = REJECT
pub async fn Read_File_Stream_Ack(
    stream: &mut libp2p::Stream,
) -> io::Result<bool> {
    let mut buf = [0u8; 1];
    stream.read_exact(&mut buf).await?;
    Ok(buf[0] == FILE_HEADER_ACCEPT)
}

// ===== 纯数据传输函数 =====

/// 发送文件数据（纯 raw data，无 header）
///
/// 文件元数据（文件名、大小）已通过 Request-Response 协商完成，
/// 流中只包含分块的文件原始数据。
///
/// 打开文件 → 分块读取 → 写入流 → flush。
///
/// # 参数
/// - `stream`: 已打开的出站流（由 `Network_Capability::open_file_stream` 返回）
/// - `file_path`: 待发送文件的路径（由 StorageManager.acquire_read 返回）
///
/// # 注意
/// - 不包含流内嵌 header（文件名+文件大小已协商）
/// - 在调用方的 tokio task 中执行，不 spawn 新任务
pub async fn Send_File_Data(
    stream: &mut libp2p::Stream,
    file_path: &Path,
    event_bus: &EventBus,
) -> io::Result<()> {
    // 1. 打开文件
    let mut file = tokio::fs::File::open(file_path).await?;
    let file_size = tokio::fs::metadata(file_path).await?.len();
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string();

    info!(
        "开始发送文件数据: {} ({} bytes)",
        file_path.display(),
        file_size
    );

    // 2. 分块读取文件并写入流
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut sent: u64 = 0;

    loop {
        let n = tokio::io::AsyncReadExt::read(&mut file, &mut buf).await?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n]).await?;
        sent += n as u64;
        debug!("已发送: {}/{} bytes", sent, file_size);

        event_bus.Publish(Bus_Event::Stream {
            payload: serde_json::json!({
                "type": "file_progress",
                "name": file_name,
                "dir": "send",
                "peer": "",
                "sent": sent,
                "total": file_size,
            })
            .to_string(),
        });
    }

    // 3. flush 确保所有数据已写入
    stream.flush().await?;

    info!(
        "文件数据发送完成: {} ({} bytes)",
        file_path.display(),
        sent
    );

    Ok(())
}

/// 接收文件数据并写入指定路径（纯 raw data，无 header）
///
/// 文件元数据（file_name, file_size）已通过 Request-Response 协商获得，
/// 调用方据此通过 StorageManager 获取写锁和目标路径。
///
/// 从流中精确读取 `file_size` 字节 → 分块写入磁盘 → flush。
///
/// # 参数
/// - `stream`: 入站流（由 `Network_Inbound_Event::FileStreamArrived` 提供）
/// - `dest_path`: 目标文件路径（由 StorageManager.acquire_write 返回）
/// - `file_size`: 文件大小（由元数据协商获得）
///
/// # 注意
/// - 不包含流内嵌 header（文件名+文件大小已协商）
/// - 在调用方的 tokio task 中执行，不 spawn 新任务
/// - 如果流在传输中断（read 返回 0 但 remaining > 0），返回 UnexpectedEof
pub async fn Receive_File_Data(
    stream: &mut libp2p::Stream,
    dest_path: &Path,
    file_size: u64,
    event_bus: &EventBus,
) -> io::Result<()> {
    let file_name = dest_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string();

    info!(
        "开始接收文件数据: {} ({} bytes)",
        dest_path.display(),
        file_size
    );

    // 1. 确保父目录存在
    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // 2. 创建目标文件
    let mut file = tokio::fs::File::create(dest_path).await?;

    // 3. 分块接收并写入磁盘
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut remaining = file_size;

    while remaining > 0 {
        let to_read = std::cmp::min(remaining as usize, buf.len());
        let n = stream.read(&mut buf[..to_read]).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "文件接收中断: 已接收 {}/{} bytes",
                    file_size - remaining,
                    file_size
                ),
            ));
        }
        tokio::io::AsyncWriteExt::write_all(&mut file, &buf[..n]).await?;
        remaining -= n as u64;
        let received = file_size - remaining;
        debug!("已接收: {}/{} bytes", received, file_size);

        event_bus.Publish(Bus_Event::Stream {
            payload: serde_json::json!({
                "type": "file_progress",
                "name": file_name,
                "dir": "receive",
                "peer": "",
                "sent": received,
                "total": file_size,
            })
            .to_string(),
        });
    }

    // 4. flush 确保所有数据已落盘
    tokio::io::AsyncWriteExt::flush(&mut file).await?;

    info!(
        "文件数据接收完成: {} ({} bytes)",
        dest_path.display(),
        file_size
    );

    Ok(())
}
