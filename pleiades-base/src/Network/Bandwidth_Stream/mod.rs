//Presented by KeJi
//Date ： 2026-05-16

//! Bandwidth_Stream 子模块
//!
//! 提供 iperf 风格固定时长推流带宽测速协议。
//! 与 File_Stream、Tensor_Stream 对称，属于 Network 层的子目录。

pub mod protocol;

pub use protocol::{
    BANDWIDTH_STREAM_PROTOCOL, CHUNK_SIZE,
    DEFAULT_DURATION_SECS, MAX_DURATION_SECS,
    Send_Bandwidth_Test, Receive_And_Count,
    Write_Bandwidth_Result, Read_Bandwidth_Result,
    Run_Bandwidth_Test,
};
