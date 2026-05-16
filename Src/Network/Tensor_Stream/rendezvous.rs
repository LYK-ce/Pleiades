//Presented by KeJi
//Date ： 2026-05-14

//! Tensor Stream Rendezvous — 张量流双向匹配
//!
//! 解决入站流从 Network Event Loop (tokio task) 到 ML Thread (OS thread)
//! 的跨上下文交接问题。两个 HashMap 用同一把 `std::sync::Mutex` 保护，
//! 保证检查+插入原子。
//!
//! ## 匹配逻辑
//! ```text
//! 流先到:  pending_accept[id] 有 Tx?  Yes → tx.send(stream) 唤醒  No → 存入 pending_inbound[id]
//! 人先到:  pending_inbound[id] 有流?  Yes → 立即返回            No → 创建 Tx 存入 pending_accept[id]
//! ```

#![allow(non_snake_case)]

use std::collections::HashMap;
use tokio::sync::oneshot;

/// 张量流双向匹配器
///
/// 以 `inference_id` 为 key，Network Event Loop 和 ML Thread 各自调用
/// 对应方法完成流的交接。
pub struct RendezvousMap {
    inner: std::sync::Mutex<RendezvousInner>,
}

struct RendezvousInner {
    /// 流先到，等人取
    pending_inbound: HashMap<u64, libp2p::Stream>,
    /// 人先到，等流来
    pending_accept: HashMap<u64, oneshot::Sender<libp2p::Stream>>,
}

impl RendezvousMap {
    /// 创建空的 RendezvousMap
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(RendezvousInner {
                pending_inbound: HashMap::new(),
                pending_accept: HashMap::new(),
            }),
        }
    }

    /// 入站流到达时调用（Network Event Loop，tokio task）
    ///
    /// 检查是否有 accept 在等待该 inference_id：
    /// - 有 → 通过 oneshot 发送 stream 唤醒等待方
    /// - 无 → 存入 pending_inbound 等待后续 accept
    pub fn insert_inbound(&self, id: u64, stream: libp2p::Stream) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = inner.pending_accept.remove(&id) {
            let _ = tx.send(stream);
        } else {
            inner.pending_inbound.insert(id, stream);
        }
    }

    /// accept 调用时调用（ML Thread，OS thread，通过 Capability）
    ///
    /// 检查是否有入站流已到达该 inference_id：
    /// - 有 → 立即通过 oneshot 返回 stream
    /// - 无 → 创建 oneshot 存入 pending_accept，返回 Receiver 供等待
    ///
    /// 调用方需 `block_on` 等待返回的 Receiver（可配合 timeout）。
    pub fn register_accept(&self, id: u64) -> oneshot::Receiver<libp2p::Stream> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stream) = inner.pending_inbound.remove(&id) {
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(stream);
            rx
        } else {
            let (tx, rx) = oneshot::channel();
            inner.pending_accept.insert(id, tx);
            rx
        }
    }
}
