//Presented by KeJi
//Date ： 2026-05-22

//! Session Stream — 会话流协议
//!
//! 远程节点通过此流向本节点申请会话槽位并建立长连接文本通道。
//! 与 FileStream / TensorStream 对称，属于 Network 层的子目录。
//!
//! ## Handshake 帧格式
//! ```text
//! Sender → Receiver: [1B id_len][session_id UTF-8]
//! Receiver → Sender: [1B ACK]  (0x01=ACCEPT, 0x00=REJECT)
//! ```
//! 之后文本自由双向流动：prompt 方向 + token 方向。

use futures::prelude::*;
use std::io;

// ===== 协议标识符 =====
pub const SESSION_STREAM_PROTOCOL: &str = "/pleiades/session/1.0.0";

// ===== ACK 常量 =====
pub const SESSION_ACK_ACCEPT: u8 = 0x01;
pub const SESSION_ACK_REJECT: u8 = 0x00;

// ===== Handshake 读写 =====

/// 写入 Session Stream Handshake（发起方在 open_session_stream 后调用）
///
/// 格式: `[1B id_len][session_id UTF-8]`
pub async fn Write_Session_Stream_Handshake(
    stream: &mut libp2p::Stream,
    session_id: &str,
) -> io::Result<()> {
    let id_bytes = session_id.as_bytes();
    let id_len = id_bytes.len() as u8;
    stream.write_all(&[id_len]).await?;
    stream.write_all(id_bytes).await?;
    stream.flush().await?;
    Ok(())
}

/// 读取 Session Stream Handshake（接收方收到入站流后调用）
///
/// 格式: `[1B id_len][session_id UTF-8]`
pub async fn Read_Session_Stream_Handshake(
    stream: &mut libp2p::Stream,
) -> io::Result<String> {
    let mut len_buf = [0u8; 1];
    stream.read_exact(&mut len_buf).await?;
    let id_len = len_buf[0] as usize;

    let mut id_buf = vec![0u8; id_len];
    stream.read_exact(&mut id_buf).await?;
    String::from_utf8(id_buf).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("session_id 非 UTF-8: {}", e))
    })
}

/// 写入 ACK 字节（接收方分配 slot 后调用）
pub async fn Write_Session_Stream_Ack(
    stream: &mut libp2p::Stream,
    accepted: bool,
) -> io::Result<()> {
    let ack = if accepted { SESSION_ACK_ACCEPT } else { SESSION_ACK_REJECT };
    stream.write_all(&[ack]).await?;
    stream.flush().await?;
    Ok(())
}

/// 读取 ACK 字节（发起方写完 handshake 后调用）
pub async fn Read_Session_Stream_Ack(
    stream: &mut libp2p::Stream,
) -> io::Result<bool> {
    let mut buf = [0u8; 1];
    stream.read_exact(&mut buf).await?;
    Ok(buf[0] == SESSION_ACK_ACCEPT)
}

// ===== 内联测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_constants() {
        assert_eq!(SESSION_STREAM_PROTOCOL, "/pleiades/session/1.0.0");
        assert_eq!(SESSION_ACK_ACCEPT, 0x01);
        assert_eq!(SESSION_ACK_REJECT, 0x00);
    }
}
