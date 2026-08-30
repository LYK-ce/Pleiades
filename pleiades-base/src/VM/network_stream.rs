//Presented by KeJi
//Date ： 2026-05-18

//! NetworkStream — libp2p::Stream 的 UserData 包装器
//!
//! stream 用 Mutex 包装，使 send_tensor / recv_tensor 等异步能力函数
//! 可通过共享引用获得可变访问。
//!
//! 操作方法（send_tensor / recv_tensor / send_eof）在 capability_binding.rs
//! 中注册为 caps.network.* 全局异步函数。

use crate::orchestrator::Capabilities;
use std::sync::{Arc, Mutex};

pub struct NetworkStream {
    pub stream: Mutex<libp2p::Stream>,
    caps: Arc<Capabilities>,
}

impl NetworkStream {
    pub fn new(stream: libp2p::Stream, caps: Arc<Capabilities>) -> Self {
        Self {
            stream: Mutex::new(stream),
            caps,
        }
    }
}

impl mlua::UserData for NetworkStream {}
