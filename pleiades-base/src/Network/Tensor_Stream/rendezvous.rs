//Presented by KeJi
//Date ： 2026-05-14

//! Tensor Stream Rendezvous — 张量流双向匹配
//!
//! 三种匹配模式共存：
//! 1. 通知模式 (v2): Session 注册 mpsc → stream 到达时直接 push
//! 2. oneshot 模式: Pipeline accept 阻塞等待
//! 3. 存货模式: 流先到，等后续 accept 来取

#![allow(non_snake_case)]

use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

pub struct RendezvousMap {
    inner: std::sync::Mutex<RendezvousInner>,
}

struct RendezvousInner {
    pending_inbound: HashMap<u64, libp2p::Stream>,
    pending_accept: HashMap<u64, oneshot::Sender<libp2p::Stream>>,
    notifiers: HashMap<u64, mpsc::UnboundedSender<libp2p::Stream>>,
}

impl RendezvousMap {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(RendezvousInner {
                pending_inbound: HashMap::new(),
                pending_accept: HashMap::new(),
                notifiers: HashMap::new(),
            }),
        }
    }

    /// 注册通知通道（Session.select! 模式）
    pub fn register_notify(&self, id: u64, tx: mpsc::UnboundedSender<libp2p::Stream>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.notifiers.insert(id, tx);
    }

    /// 入站流到达（三级优先级分发）
    pub fn insert_inbound(&self, id: u64, stream: libp2p::Stream) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // 1. 通知模式 — Session.select! 在等
        if let Some(tx) = inner.notifiers.remove(&id) {
            let _ = tx.send(stream);
            return;
        }
        // 2. oneshot 模式 — Pipeline accept 阻塞等
        if let Some(tx) = inner.pending_accept.remove(&id) {
            let _ = tx.send(stream);
            return;
        }
        // 3. 存货 — 等后续 accept 来取
        inner.pending_inbound.insert(id, stream);
    }

    /// accept 调用（Pipeline 模式，阻塞等待）
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
