//Presented by KeJi
//Date ： 2026-05-24

//! Session 结构 — 模型级会话容器
//!
//! 每个 Session spawn 自己的 select! 任务，通过 slot mpsc 通道对接收 prompt。

use std::sync::Arc;

use super::capability::SessionRequest;
use super::slot::{Slot, SlotState};
use crate::event_bus::{Bus_Event, EventBus, NotifyLevel};
use crate::ml_engine::context::MlSession;
use crate::ml_engine::lua_tensor::{bytes_to_tensor, tensor_to_bytes};
use crate::storage::StorageCapability;
use tokio::sync::mpsc;

/// 会话容器（对应一个模型）
pub struct Session {
    pub session_id: u64,
    pub model_id: String,
    pub max_slots: usize,
    pub slots: Vec<SlotState>,
    pub eos_token_id: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// slot_notify_tx: allocate_slot 时把 (slot_id, prompt_rx, token_tx) 发给 spawn task
    pub slot_notify_tx: mpsc::UnboundedSender<(usize, mpsc::UnboundedReceiver<SessionRequest>, mpsc::UnboundedSender<String>)>,
}

impl Session {
    pub fn new(
        session_id: u64,
        model_id: String,
        max_slots: usize,
        eos_token_id: u32,
        slot_notify_tx: mpsc::UnboundedSender<(usize, mpsc::UnboundedReceiver<SessionRequest>, mpsc::UnboundedSender<String>)>,
    ) -> Self {
        use std::sync::Arc;
        use std::sync::atomic::AtomicU32;
        let slots = (0..max_slots).map(|_| SlotState::Vacant).collect();
        Session {
            session_id,
            model_id,
            max_slots,
            slots,
            eos_token_id: Arc::new(AtomicU32::new(eos_token_id)),
            slot_notify_tx,
        }
    }

    /// 启动 Session 的 task。
    ///
    /// `select!` 分支：
    /// - slot prompt_rx 收到 prompt → encode → tensorize → send to ML
    /// - ML 流收到 logits → sample → decode → slot.token_tx
    pub fn spawn(
        &self,
        slot_notify_rx: mpsc::UnboundedReceiver<(usize, mpsc::UnboundedReceiver<SessionRequest>, mpsc::UnboundedSender<String>)>,
        stream_hub: Arc<crate::orchestrator::local_tensor_stream::LocalStreamHub>,
        event_bus: Arc<EventBus>,
        storage: Arc<dyn StorageCapability>,
    ) {
        let session_id = self.session_id;
        let model_path = self.model_id.clone();
        let ml_stream_id = format!("ml-{}", session_id);
        let eos = self.eos_token_id.clone();

        tokio::spawn(async move {
            // ── 1. 加载 tokenizer ──────────────────────────
            let (path, _guard) = match storage.acquire_read(&model_path).await {
                Ok(p) => p,
                Err(e) => {
                    event_bus.Publish(Bus_Event::Notify {
                        level: NotifyLevel::Error,
                        message: format!("Session {} storage error: {}", session_id, e),
                    });
                    return;
                }
            };

            let mut ml = match MlSession::new("cpu") {
                Ok(s) => s,
                Err(e) => {
                    event_bus.Publish(Bus_Event::Notify {
                        level: NotifyLevel::Error,
                        message: format!("Session {} ml init error: {}", session_id, e),
                    });
                    return;
                }
            };

            if let Err(e) = ml.load_tokenizer(&path) {
                event_bus.Publish(Bus_Event::Notify {
                    level: NotifyLevel::Error,
                    message: format!("Session {} tokenizer error: {}", session_id, e),
                });
                return;
            }
            drop(_guard);
            eos.store(ml.get_eos(), std::sync::atomic::Ordering::Relaxed);
            tracing::info!("Session {} tokenizer loaded from {}", session_id, model_path);

            // ── 2. 等 ML Thread 连接 ──────────────────────
            let mut ml_stream = match stream_hub.accept_async(&ml_stream_id, 120_000).await {
                Ok(s) => {
                    tracing::info!("Session {} ML connected", session_id);
                    s
                }
                Err(e) => {
                    tracing::warn!("Session {} ML accept error: {}", session_id, e);
                    return;
                }
            };

            // ── 3. 等 slot 分配 ──────────────────────────
            // 第一个 slot: 阻塞等待
            let mut slot_notify_rx = slot_notify_rx;
            let (mut slot_id, mut prompt_rx, token_tx) = match slot_notify_rx.recv().await {
                Some(info) => info,
                None => {
                    tracing::warn!("Session {} slot notify channel closed", session_id);
                    return;
                }
            };
            tracing::info!("Session {} slot {} allocated", session_id, slot_id);

            // 后续可动态增加 slot，用 HashMap 管理
            use std::collections::HashMap;
            let mut slot_tokens: HashMap<usize, mpsc::UnboundedSender<String>> = HashMap::new();
            slot_tokens.insert(slot_id, token_tx);

            // ── 4. main loop ──────────────────────────────
            let mut ml_buf =
                crate::network::tensor_stream::protocol::Tensor_Buffer::New(16 * 1024 * 1024);

            loop {
                // 检查是否有新 slot 注册
                while let Ok((new_id, new_rx, new_tx)) = slot_notify_rx.try_recv() {
                    tracing::info!("Session {} slot {} allocated (dynamic)", session_id, new_id);
                    slot_tokens.insert(new_id, new_tx);
                    slot_id = new_id;
                    prompt_rx = new_rx;
                }

                // 收 SessionRequest（包含完整 messages + max_tokens）
                match prompt_rx.recv().await {
                    Some(req) => {
                        let messages = req.messages;
                        let max_tokens = req.max_tokens;
                        tracing::info!("Session {} request: {} messages, max_tokens={}", session_id, messages.len(), max_tokens);

                        // ── encode 客户端传入的完整 messages ──
                        let token_ids = match ml.encode_messages(&messages) {
                            Ok(ids) => ids,
                            Err(e) => {
                                event_bus.Publish(Bus_Event::Notify {
                                    level: NotifyLevel::Error,
                                    message: format!("Session {} encode failed: {}", session_id, e),
                                });
                                continue;
                            }
                        };

                        let tensor = match ml.tensorize(&token_ids) {
                            Ok(t) => t,
                            Err(e) => {
                                tracing::warn!("Session {} tensorize error: {}", session_id, e);
                                continue;
                            }
                        };

                        let data = match tensor_to_bytes(&tensor) {
                            Ok(d) => d,
                            Err(e) => {
                                tracing::warn!("Session {} tensor_to_bytes error: {}", session_id, e);
                                continue;
                            }
                        };

                        if let Err(e) = crate::orchestrator::local_tensor_stream::frames::local_send_frame(
                            &mut ml_stream, 0, &data,
                        ).await {
                            tracing::warn!("Session {} send tensor error: {}", session_id, e);
                            break;
                        }

                        let mut offset = token_ids.len();
                        let eos = ml.get_eos();
                        tracing::info!("Session {} autoregression start: tokens={}, EOS={}", session_id, offset, eos);

                        // ── 自回归生成 loop ───────────────────
                        let gen_limit = max_tokens.min(2048) as usize;
                        for _ in 0..gen_limit {
                            match crate::orchestrator::local_tensor_stream::frames::local_recv_frame(
                                &mut ml_stream, &mut ml_buf,
                            ).await {
                                Ok(_offset) => {},
                                Err(e) => {
                                    tracing::warn!("Session {} ML recv error in gen: {}", session_id, e);
                                    break;
                                }
                            }

                            let logits = match bytes_to_tensor(ml_buf.As_Slice(), &candle_core::Device::Cpu) {
                                Ok(l) => l,
                                Err(e) => {
                                    tracing::warn!("Session {} bytes_to_tensor error: {}", session_id, e);
                                    break;
                                }
                            };

                            let token_id = match ml.sample(&logits, 0.0) {
                                Ok(t) => t,
                                Err(e) => {
                                    tracing::warn!("Session {} sample error: {}", session_id, e);
                                    break;
                                }
                            };

                            if token_id == eos {
                                tracing::info!("Session {} EOS (token_id={}) at offset {}", session_id, token_id, offset);
                                break;
                            }

                            match ml.decode(token_id) {
                                Ok(text) => {
                                    let _ = slot_tokens[&slot_id].send(text);
                                }
                                Err(e) => {
                                    tracing::warn!("Session {} decode error: {}", session_id, e);
                                }
                            }

                            let next_t = match ml.tensorize(&[token_id]) {
                                Ok(t) => t,
                                Err(e) => {
                                    tracing::warn!("Session {} tensorize error: {}", session_id, e);
                                    break;
                                }
                            };

                            let next_data = match tensor_to_bytes(&next_t) {
                                Ok(d) => d,
                                Err(e) => {
                                    tracing::warn!("Session {} tensor_to_bytes error: {}", session_id, e);
                                    break;
                                }
                            };

                            if let Err(e) = crate::orchestrator::local_tensor_stream::frames::local_send_frame(
                                &mut ml_stream, offset as u64, &next_data,
                            ).await {
                                tracing::warn!("Session {} send token error: {}", session_id, e);
                                break;
                            }
                            offset += 1;
                        }

                        // 空哨兵
                        let _ = slot_tokens[&slot_id].send("\0".into());
                    }
                    None => {
                        tracing::warn!("Session {} prompt channel closed", session_id);
                        break;
                    }
                }
            }
        });
    }

    pub fn allocate(&mut self, token_tx: mpsc::UnboundedSender<String>) -> Option<usize> {
        for (i, state) in self.slots.iter_mut().enumerate() {
            if matches!(state, SlotState::Vacant) {
                *state = SlotState::Occupied(Slot {
                    id: i,
                    token_tx,
                });
                return Some(i);
            }
        }
        None
    }

    /// 释放槽位
    pub fn release(&mut self, slot_id: usize) {
        if slot_id < self.slots.len() {
            self.slots[slot_id] = SlotState::Vacant;
        }
    }

    /// 已占用槽位数
    pub fn occupied_count(&self) -> usize {
        self.slots.iter().filter(|s| matches!(s, SlotState::Occupied(_))).count()
    }

    /// 获取 slot 引用
    pub fn get_slot(&self, slot_id: usize) -> Option<&Slot> {
        match self.slots.get(slot_id)? {
            SlotState::Occupied(ref s) => Some(s),
            SlotState::Vacant => None,
        }
    }

    /// 获取 slot 可变引用
    pub fn get_slot_mut(&mut self, slot_id: usize) -> Option<&mut Slot> {
        match self.slots.get_mut(slot_id)? {
            SlotState::Occupied(ref mut s) => Some(s),
            SlotState::Vacant => None,
        }
    }
}

/// 会话公开视图
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: u64,
    pub model_id: String,
    pub total_slots: usize,
    pub occupied_slots: usize,
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn make_session(id: u64, model: &str, slots: usize) -> Session {
        let (tx, _rx) = mpsc::unbounded_channel();
        Session::new(id, model.into(), slots, 1, tx)
    }

    #[test]
    fn test_session_new_all_vacant() {
        let sess = make_session(1, "qwen3", 4);
        assert_eq!(sess.session_id, 1);
        assert_eq!(sess.max_slots, 4);
        assert_eq!(sess.slots.len(), 4);
        assert_eq!(sess.occupied_count(), 0);
        assert!(matches!(sess.slots[0], SlotState::Vacant));
    }

    #[test]
    fn test_allocate_and_release() {
        let mut sess = make_session(2, "qwen3", 4);
        let (tx, _rx) = mpsc::unbounded_channel();

        let id = sess.allocate(tx).expect("should allocate");
        assert_eq!(id, 0);
        assert_eq!(sess.occupied_count(), 1);
        assert!(sess.get_slot(0).is_some());

        sess.release(0);
        assert_eq!(sess.occupied_count(), 0);
        assert!(sess.get_slot(0).is_none());
    }

    #[test]
    fn test_allocate_exhausts_slots() {
        let mut sess = make_session(3, "qwen3", 2);

        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();
        let (tx3, _rx3) = mpsc::unbounded_channel();

        assert!(sess.allocate(tx1).is_some());
        assert!(sess.allocate(tx2).is_some());
        assert!(sess.allocate(tx3).is_none());
        assert_eq!(sess.occupied_count(), 2);
    }

    #[test]
    fn test_allocate_after_release() {
        let mut sess = make_session(4, "qwen3", 2);

        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();
        let (tx3, _rx3) = mpsc::unbounded_channel();

        sess.allocate(tx1);
        sess.allocate(tx2);
        sess.release(0);

        let id = sess.allocate(tx3).expect("should allocate");
        assert_eq!(id, 0);
    }

    #[test]
    fn test_slot_token_buf_operations() {
        let mut sess = make_session(5, "qwen3", 2);
        let (tx, _rx) = mpsc::unbounded_channel();
        sess.allocate(tx);

        let slot = sess.get_slot(0).unwrap();
        assert_eq!(slot.id, 0);
    }

    #[test]
    fn test_get_slot_out_of_bounds() {
        let sess = make_session(6, "qwen3", 2);
        assert!(sess.get_slot(5).is_none());
        let mut sess = sess;
        assert!(sess.get_slot_mut(5).is_none());
    }
}
