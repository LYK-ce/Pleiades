//Presented by KeJi
//Date ： 2026-05-22

//! Session Stream 子模块
//!
//! 提供会话流协议，包括 handshake（session_id 交换 + ACK）、双向文本传输。
//! 与 Tensor_Stream / File_Stream 对称，属于 Network 层的子目录。

pub mod protocol;

pub use protocol::{
    SESSION_STREAM_PROTOCOL,
    SESSION_ACK_ACCEPT, SESSION_ACK_REJECT,
    Write_Session_Stream_Handshake, Read_Session_Stream_Handshake,
    Write_Session_Stream_Ack, Read_Session_Stream_Ack,
};
