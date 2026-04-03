//Presented by KeJi
//Date ： 2026-04-01

//! 流式传输协议模块
//!
//! 定义文件流式传输协议，用于大文件的分块传输。
//! 与 data_protocol.rs 中的请求响应协议不同，
//! 本模块使用 libp2p::stream 提供的原始双向流进行分块传输，
//! 数据不需要全部加载到内存。
//!
//! 流帧格式:
//! +-------------------+--------------------+---------------------+
//! | FileName Length    |    FileName        |    File Size        |
//! | 4 bytes (u32 BE)  |    N bytes UTF-8   |    8 bytes (u64 BE) |
//! +-------------------+--------------------+---------------------+
//! |                   Raw Data Chunks                            |
//! |              (CHUNK_SIZE bytes per chunk)                    |
//! +--------------------------------------------------------------+
//!
//! 发送方打开流 → 写入 header（文件名+文件大小）→ 分块写入文件数据
//! 接收方读取 header → 边收边写磁盘

use futures::prelude::*;
use std::io;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

// ===== 协议标识符 =====
pub const FILE_STREAM_PROTOCOL: &str = "/pleiades/file-stream/1.0.0";

// ===== 分块大小 (64KB) =====
pub const CHUNK_SIZE: usize = 64 * 1024;

/// 流式发送文件
///
/// 通过已打开的 libp2p::Stream 发送文件，采用分块传输方式。
///
/// # Arguments
/// * `stream` - 已打开的双向流
/// * `file_path` - 待发送文件的路径
///
/// # 帧格式
/// [4B name_len][name_bytes][8B file_size][raw data chunks...]
pub async fn Send_File_Stream(
    stream: &mut libp2p::Stream,
    file_path: &Path,
) -> io::Result<()> {
    // 1. 获取文件名
    let file_name = file_path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无效的文件路径"))?
        .to_string_lossy();
    let name_bytes = file_name.as_bytes();

    // 2. 写入文件名长度 (4 bytes, u32 BE)
    let name_len = name_bytes.len() as u32;
    stream.write_all(&name_len.to_be_bytes()).await?;

    // 3. 写入文件名
    stream.write_all(name_bytes).await?;

    // 4. 获取并写入文件大小 (8 bytes, u64 BE)
    let file_metadata = tokio::fs::metadata(file_path).await?;
    let file_size = file_metadata.len();
    stream.write_all(&file_size.to_be_bytes()).await?;

    info!(
        "开始流式发送文件: {} ({} bytes)",
        file_name, file_size
    );

    // 5. 分块读取文件并写入流
    let mut file = tokio::fs::File::open(file_path).await?;
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

    stream.flush().await?;

    info!(
        "文件流式发送完成: {} ({} bytes)",
        file_name, sent
    );

    Ok(())
}

/// 流式接收文件
///
/// 从 libp2p::Stream 中接收文件，边收边写入磁盘。
///
/// # Arguments
/// * `stream` - 已打开的双向流
/// * `save_dir` - 文件保存目录
///
/// # Returns
/// 保存的文件路径
pub async fn Receive_File_Stream(
    stream: &mut libp2p::Stream,
    save_dir: &Path,
) -> io::Result<PathBuf> {
    // 1. 读取文件名长度 (4 bytes, u32 BE)
    let mut name_len_bytes = [0u8; 4];
    stream.read_exact(&mut name_len_bytes).await?;
    let name_len = u32::from_be_bytes(name_len_bytes) as usize;

    // 文件名长度合理性检查 (最大 4096 字节)
    if name_len > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("文件名过长: {} bytes", name_len),
        ));
    }

    // 2. 读取文件名
    let mut name_buf = vec![0u8; name_len];
    stream.read_exact(&mut name_buf).await?;
    let file_name = String::from_utf8(name_buf).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("文件名编码错误: {}", e))
    })?;

    // 3. 读取文件大小 (8 bytes, u64 BE)
    let mut size_bytes = [0u8; 8];
    stream.read_exact(&mut size_bytes).await?;
    let file_size = u64::from_be_bytes(size_bytes);

    info!(
        "开始流式接收文件: {} ({} bytes)",
        file_name, file_size
    );

    // 4. 确保保存目录存在
    tokio::fs::create_dir_all(save_dir).await?;

    // 5. 创建目标文件
    let save_path = save_dir.join(&file_name);
    let mut file = tokio::fs::File::create(&save_path).await?;

    // 6. 分块接收并写入磁盘
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
        debug!("已接收: {}/{} bytes", file_size - remaining, file_size);
    }

    tokio::io::AsyncWriteExt::flush(&mut file).await?;

    info!(
        "文件流式接收完成: {} ({} bytes) -> {}",
        file_name,
        file_size,
        save_path.display()
    );

    Ok(save_path)
}
