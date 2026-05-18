//Presented by KeJi
//Date ： 2026-05-18

//! NetworkStream — libp2p::Stream 的 mLua UserData 包装器
//!
//! 持有 Stream 和 Arc<Capabilities>，供张量流等低层流操作使用。
//! 文件传送高层语义通过 `caps.send_file` 实现，不走此类型。

use crate::orchestrator::Capabilities;
use std::sync::Arc;

pub struct NetworkStream {
    pub stream: libp2p::Stream,
    caps: Arc<Capabilities>,
}

impl NetworkStream {
    pub fn new(stream: libp2p::Stream, caps: Arc<Capabilities>) -> Self {
        Self { stream, caps }
    }
}

impl mlua::UserData for NetworkStream {
    // 张量流 read/write 方法后续在此添加
}
