//Presented by KeJi
//Date ： 2026-05-15

//! File Stream 子模块
//!
//! 提供文件流式传输协议，包括 in-band header、ACK 握手、分块数据传输。
//! 与 Tensor_Stream 对称，属于 Network 层的子目录。

pub mod protocol;

pub use protocol::{
    FILE_STREAM_PROTOCOL, CHUNK_SIZE,
    FILE_HEADER_ACCEPT, FILE_HEADER_REJECT,
    Write_File_Stream_Header, Read_File_Stream_Header,
    Write_File_Stream_Ack, Read_File_Stream_Ack,
    Send_File_Data, Receive_File_Data,
};
