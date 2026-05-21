//Presented by KeJi
//Date ： 2026-05-20

//! 本地帧读写函数
//!
//! 与 Tensor Stream 共享 `[8B offset LE][8B length LE][data]` 帧格式，
//! 但接受泛型 `S: AsyncRead + AsyncWrite + Unpin` 而非 `libp2p::Stream`。

use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::network::tensor_stream::protocol::{Tensor_Buffer, TENSOR_EOF_OFFSET};

/// 本地版 Send_Tensor_Frame
///
/// 写入格式: [8B offset LE][8B length LE][data bytes]
pub async fn local_send_frame<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
    offset: u64,
    data: &[u8],
) -> io::Result<()> {
    stream.write_all(&offset.to_le_bytes()).await?;
    stream.write_all(&(data.len() as u64).to_le_bytes()).await?;
    stream.write_all(data).await?;
    stream.flush().await?;
    Ok(())
}

/// 本地版 Receive_Tensor_Frame
///
/// 读取格式: [8B offset LE][8B length LE][data bytes]
/// 数据写入外部提供的 Tensor_Buffer。
pub async fn local_recv_frame<S: AsyncReadExt + Unpin>(
    stream: &mut S,
    buffer: &mut Tensor_Buffer,
) -> io::Result<u64> {
    let mut header = [0u8; 16];
    stream.read_exact(&mut header).await?;

    let offset = u64::from_le_bytes(header[0..8].try_into().unwrap());
    let length = u64::from_le_bytes(header[8..16].try_into().unwrap());

    if offset == TENSOR_EOF_OFFSET && length == 0 {
        buffer.Clear();
        return Ok(TENSOR_EOF_OFFSET);
    }

    let slice = buffer.As_Mut_Slice(length as usize);
    stream.read_exact(slice).await?;

    Ok(offset)
}

/// 本地版 Send_EOF
///
/// 写入 EOF 哨兵帧: [u64::MAX][0u64]
pub async fn local_send_eof<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
) -> io::Result<()> {
    stream.write_all(&TENSOR_EOF_OFFSET.to_le_bytes()).await?;
    stream.write_all(&0u64.to_le_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_local_send_recv_roundtrip() {
        let (mut a, mut b) = duplex(64 * 1024);

        let data = b"hello tensor stream locally";
        local_send_frame(&mut a, 42, data).await.expect("send");

        let mut buf = Tensor_Buffer::New(1024);
        let offset = local_recv_frame(&mut b, &mut buf).await.expect("recv");

        assert_eq!(offset, 42);
        assert_eq!(buf.As_Slice(), data);
    }

    #[tokio::test]
    async fn test_local_send_recv_eof() {
        let (mut a, mut b) = duplex(64 * 1024);

        local_send_eof(&mut a).await.expect("send eof");

        let mut buf = Tensor_Buffer::New(1024);
        let offset = local_recv_frame(&mut b, &mut buf).await.expect("recv");

        assert_eq!(offset, TENSOR_EOF_OFFSET);
        assert!(buf.Is_Empty());
    }
}
