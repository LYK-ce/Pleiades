//Presented by KeJi
//Date ： 2026-05-24

//! Session 结构 — 模型级会话容器
//!
//! 每个 Session spawn 自己的 select! 任务，通过 LocalStreamHub 等待连接。

use std::sync::Arc;

use super::slot::{Slot, SlotState};
use crate::event_bus::{Bus_Event, EventBus, NotifyLevel};
use crate::ml_engine::context::MlSession;
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

    /// 启动 Session 的 task
    ///
    /// Task 内部：加载 tokenizer → 等 TUI 连接 → 收 prompt → encode
    /// 所有 I/O 在 task 内完成，不阻塞调用方。
    pub fn spawn(
        &self,
        stream_hub: Arc<LocalStreamHub>,
        event_bus: Arc<EventBus>,
        storage: Arc<dyn StorageCapability>,
    ) {
        let session_id = self.session_id;
        let model_path = self.model_id.clone();
        let stream_id = format!("session-{}", session_id);

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
            drop(_guard); // 释放 Storage 读锁

            tracing::info!("Session {} tokenizer loaded from {}", session_id, model_path);

            // ── 2. 等 TUI 连接 ──────────────────────────────
            let mut stream = match stream_hub.accept_async(&stream_id, 30_000).await {
                Ok(s) => {
                    tracing::info!("Session {} connected", session_id);
                    s
                }
                Err(e) => {
                    tracing::warn!("Session {} accept error: {}", session_id, e);
                    return;
                }
            };

            // ── 3. 收 prompt → encode → 发布（loop）──────
            let mut buf = crate::network::tensor_stream::protocol::Tensor_Buffer::New(4096);
            loop {
                match crate::orchestrator::local_tensor_stream::frames::local_recv_frame(
                    &mut stream, &mut buf,
                ).await {
                    Ok(_offset) => {
                        let text = String::from_utf8_lossy(buf.As_Slice());
                        tracing::info!("Session {} received: {}", session_id, text);

                        match ml.encode(&text) {
                            Ok(token_ids) => {
                                event_bus.Publish(Bus_Event::Notify {
                                    level: NotifyLevel::Info,
                                    message: format!("Session {}: {} ({} tokens)",
                                        session_id, text, token_ids.len()),
                                });
                            }
                            Err(e) => {
                                event_bus.Publish(Bus_Event::Notify {
                                    level: NotifyLevel::Error,
                                    message: format!("Session {} encode failed: {}", session_id, e),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Session {} recv error: {}", session_id, e);
                        break;
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
