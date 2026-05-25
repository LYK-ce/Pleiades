//Presented by KeJi
//Date ： 2026-05-24

//! Session 结构 — 模型级会话容器
//!
//! 每个 Session spawn 自己的 select! 任务，通过 LocalStreamHub 等待连接。

use std::sync::Arc;

use super::slot::{Slot, SlotState};
use crate::event_bus::{Bus_Event, EventBus, NotifyLevel};
use crate::ml_engine::context::MlSession;
use crate::ml_engine::lua_tensor::{bytes_to_tensor, tensor_to_bytes};
use crate::orchestrator::local_tensor_stream::LocalStreamHub;
use crate::storage::StorageCapability;

/// 会话容器（对应一个模型）
pub struct Session {
    pub session_id: u64,
    pub model_id: String,
    pub max_slots: usize,
    pub slots: Vec<SlotState>,
    pub eos_token_id: u32,
}

impl Session {
    pub fn new(session_id: u64, model_id: String, max_slots: usize, eos_token_id: u32) -> Self {
        let slots = (0..max_slots).map(|_| SlotState::Vacant).collect();
        Session { session_id, model_id, max_slots, slots, eos_token_id }
    }

    /// 启动 Session 的 task，管理 chat 和 ML 两条流。
    ///
    /// `select!` 分支：
    /// - chat 流收到 prompt → encode → tensorize → send to ML
    /// - ML 流收到 logits → sample → decode → EventBus
    pub fn spawn(
        &self,
        stream_hub: Arc<LocalStreamHub>,
        event_bus: Arc<EventBus>,
        storage: Arc<dyn StorageCapability>,
    ) {
        let session_id = self.session_id;
        let model_path = self.model_id.clone();
        let chat_stream_id = format!("session-{}", session_id);
        let ml_stream_id = format!("ml-{}", session_id);

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
            tracing::info!("Session {} tokenizer loaded from {}", session_id, model_path);

            // ── 2. 等 chat 连接 ────────────────────────────
            let mut chat_stream = match stream_hub.accept_async(&chat_stream_id, 30_000).await {
                Ok(s) => {
                    tracing::info!("Session {} chat connected", session_id);
                    s
                }
                Err(e) => {
                    tracing::warn!("Session {} chat accept error: {}", session_id, e);
                    return;
                }
            };

            // ── 3. 等 ML Thread 连接 ──────────────────────
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

            // ── 4. select! loop ────────────────────────────
            let mut chat_buf =
                crate::network::tensor_stream::protocol::Tensor_Buffer::New(4096);
            let mut ml_buf =
                crate::network::tensor_stream::protocol::Tensor_Buffer::New(16 * 1024 * 1024);

            loop {
                tokio::select! {
                    // 收 prompt 来自 chat
                    result = crate::orchestrator::local_tensor_stream::frames::local_recv_frame(
                        &mut chat_stream, &mut chat_buf,
                    ) => {
                        match result {
                            Ok(_offset) => {
                                let text = String::from_utf8_lossy(chat_buf.As_Slice());
                                tracing::info!("Session {} chat: {}", session_id, text);

                                // ── encode → tensorize → prefill ──────
                                let token_ids = match ml.encode(&text) {
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

                                // ── 自回归生成 loop ───────────────────
                                for _ in 0..120 {
                                    // recv logits from ML
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

                                    let token_id = match ml.sample(&logits, 0.8) {
                                        Ok(t) => t,
                                        Err(e) => {
                                            tracing::warn!("Session {} sample error: {}", session_id, e);
                                            break;
                                        }
                                    };

                                    // EOS → end generation
                                    if token_id == eos {
                                        break;
                                    }

                                    match ml.decode(token_id) {
                                        Ok(text) => {
                                            tracing::info!("Session {} output: {}", session_id, text);
                                            event_bus.Publish(Bus_Event::Notify {
                                                level: NotifyLevel::Info,
                                                message: format!("Session {}: {}", session_id, text),
                                            });
                                        }
                                        Err(e) => {
                                            tracing::warn!("Session {} decode error: {}", session_id, e);
                                        }
                                    }

                                    // send next single token
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
                            }
                            Err(e) => {
                                tracing::warn!("Session {} chat recv error: {}", session_id, e);
                                break;
                            }
                        }
                    }

                    // 收 logits 来自 ML Thread（仅用于非自回归阶段，已废弃）
                    result = crate::orchestrator::local_tensor_stream::frames::local_recv_frame(
                        &mut ml_stream, &mut ml_buf,
                    ) => {
                        // 自回归循环已在 chat 分支内完成，此分支仅防止 select! 预检 panic
                        let _ = result;
                    }
                }
            }
        });
    }

    pub fn allocate(&mut self, token_tx: tokio::sync::mpsc::UnboundedSender<String>) -> Option<usize> {
        for (i, state) in self.slots.iter_mut().enumerate() {
            if matches!(state, SlotState::Vacant) {
                *state = SlotState::Occupied(Slot {
                    id: i,
                    token_buf: Vec::new(),
                    token_tx,
                    dirty: false,
                    temperature: 0.8,
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

    #[test]
    fn test_session_new_all_vacant() {
        let sess = Session::new(1, "qwen3".into(), 4, 1);
        assert_eq!(sess.session_id, 1);
        assert_eq!(sess.max_slots, 4);
        assert_eq!(sess.slots.len(), 4);
        assert_eq!(sess.occupied_count(), 0);
        assert!(matches!(sess.slots[0], SlotState::Vacant));
    }

    #[test]
    fn test_allocate_and_release() {
        let mut sess = Session::new(2, "qwen3".into(), 4, 1);
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
        let mut sess = Session::new(3, "qwen3".into(), 2, 1);

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
        let mut sess = Session::new(4, "qwen3".into(), 2, 1);

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
        let mut sess = Session::new(5, "qwen3".into(), 2, 1);
        let (tx, _rx) = mpsc::unbounded_channel();
        sess.allocate(tx);

        let slot = sess.get_slot_mut(0).unwrap();
        slot.token_buf.extend(vec![101, 204, 307]);
        slot.dirty = true;

        let slot = sess.get_slot(0).unwrap();
        assert_eq!(slot.token_buf, vec![101, 204, 307]);
        assert!(slot.dirty);
    }

    #[test]
    fn test_get_slot_out_of_bounds() {
        let sess = Session::new(6, "qwen3".into(), 2, 1);
        assert!(sess.get_slot(5).is_none());
        let mut sess = sess;
        assert!(sess.get_slot_mut(5).is_none());
    }
}
