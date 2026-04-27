//Presented by KeJi
//Date ： 2026-04-24

//! 流式传输协议模块
//!
//! 定义文件流式传输协议，用于大文件的分块传输。
//! 与 data_protocol.rs 中的请求响应协议不同，
//! 本模块使用 libp2p::stream 提供的原始双向流进行分块传输，
//! 数据不需要全部加载到内存。
//!
//! ## 纯数据传输
//!
//! 文件元数据（文件名、大小）通过 Request-Response 协商完成，
//! 流中只包含分块的文件原始数据：
//! - `Send_File_Data`: 发送纯 raw data（无 header）
//! - `Receive_File_Data`: 接收纯 raw data（无 header）

use futures::prelude::*;
use std::io;
use std::path::Path;
use tracing::{debug, info};

// ===== 协议标识符 =====
pub const FILE_STREAM_PROTOCOL: &str = "/pleiades/file-stream/1.0.0";

// ===== 分块大小 (64KB) =====
pub const CHUNK_SIZE: usize = 64 * 1024;

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
/// - 不上报进度（进度上报将在 Phase 3 通过 Orchestrator 机制实现）
/// - 在调用方的 tokio task 中执行，不 spawn 新任务
pub async fn Send_File_Data(
    stream: &mut libp2p::Stream,
    file_path: &Path,
) -> io::Result<()> {
    // 1. 打开文件
    let mut file = tokio::fs::File::open(file_path).await?;
    let file_size = tokio::fs::metadata(file_path).await?.len();

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
/// - 不上报进度（进度上报将在 Phase 3 通过 Orchestrator 机制实现）
/// - 在调用方的 tokio task 中执行，不 spawn 新任务
/// - 如果流在传输中断（read 返回 0 但 remaining > 0），返回 UnexpectedEof
pub async fn Receive_File_Data(
    stream: &mut libp2p::Stream,
    dest_path: &Path,
    file_size: u64,
) -> io::Result<()> {
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
