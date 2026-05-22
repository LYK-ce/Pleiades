//Presented by KeJi
//Date ： 2026-05-22

//! SessionManager — 会话槽位管理器
//!
//! 持有所有 Session，通过 tokio::select! 主循环处理六类事件：
//! - A: 本地 open_slot
//! - B: 收 Prompt → tokenize 存入 slot token_buf
//! - C: Flush batch → 拼 tensor 送 ML thread
//! - D: 收 Logits → sample + decode + 分发
//! - E: 网络 open_slot（Task 6.4）
//! - F: 销毁 slot
//!
//! ML thread 接口暂留 stub，后续 Task 6.3 实现。

use std::collections::HashMap;
use tokio::sync::mpsc;

use super::capability::{Session_Error, SlotHandle};
use super::session::Session;

// ─── 消息类型 ───────────────────────────────────────────────

/// open_slot 请求
pub(crate) struct OpenSlotRequest {
    pub session_id: String,
    pub reply_tx: tokio::sync::oneshot::Sender<Result<SlotHandle, Session_Error>>,
}

/// close_slot 请求
pub(crate) struct CloseSlotRequest {
    pub session_id: String,
    pub slot_id: usize,
}

// ─── SessionManager ─────────────────────────────────────────

pub struct SessionManager {
    /// session_id → Session
    pub sessions: HashMap<String, Session>,

    /// 共享 prompt 通道 — 所有 SlotHandle 共用发送端
    shared_prompt_tx: mpsc::UnboundedSender<(String, usize, String)>,
    pub shared_prompt_rx: mpsc::UnboundedReceiver<(String, usize, String)>,

    /// open_slot 请求通道
    open_slot_tx: mpsc::UnboundedSender<OpenSlotRequest>,
    open_slot_rx: mpsc::UnboundedReceiver<OpenSlotRequest>,

    /// close_slot 请求通道
    close_slot_tx: mpsc::UnboundedSender<CloseSlotRequest>,
    close_slot_rx: mpsc::UnboundedReceiver<CloseSlotRequest>,

    /// ML thread — batch 输入（Task 6.3 对接，当前占位）
    #[allow(dead_code)]
    batch_tx: mpsc::Sender<(Vec<u32>, Vec<(String, usize)>)>,
    #[allow(dead_code)]
    logits_rx: mpsc::Receiver<(Vec<u32>, Vec<(String, usize)>)>,

    /// 全局最大槽位数（每个 Session）
    max_slots: usize,

    /// Session ID 计数器
    session_counter: u64,

    /// 是否正在运行
    running: bool,
}

impl SessionManager {
    pub fn new(max_slots: usize) -> Self {
        let (shared_prompt_tx, shared_prompt_rx) = mpsc::unbounded_channel();
        let (open_slot_tx, open_slot_rx) = mpsc::unbounded_channel();
        let (close_slot_tx, close_slot_rx) = mpsc::unbounded_channel();
        let (batch_tx, _batch_rx) = mpsc::channel(1);
        let (_logits_tx, logits_rx) = mpsc::channel(1);

        SessionManager {
            sessions: HashMap::new(),
            shared_prompt_tx,
            shared_prompt_rx,
            open_slot_tx,
            open_slot_rx,
            close_slot_tx,
            close_slot_rx,
            batch_tx,
            logits_rx,
            max_slots,
            session_counter: 1,
            running: false,
        }
    }

    /// 获取 open_slot 的发送端（供外部调用）
    pub fn open_slot_sender(&self) -> mpsc::UnboundedSender<OpenSlotRequest> {
        self.open_slot_tx.clone()
    }

    /// 获取 close_slot 的发送端
    pub fn close_slot_sender(&self) -> mpsc::UnboundedSender<CloseSlotRequest> {
        self.close_slot_tx.clone()
    }

    /// 创建 Session
    pub fn create_session(&mut self, model_id: &str) -> String {
        let session_id = format!("sess-{}", self.session_counter);
        self.session_counter += 1;

        // TODO: Task 6.3 — 加载 tokenizer，获取 eos_token_id
        let session = Session::new(
            session_id.clone(),
            model_id.to_string(),
            self.max_slots,
            1, // placeholder eos
        );

        self.sessions.insert(session_id.clone(), session);
        session_id
    }

    /// 销毁 Session
    pub fn destroy_session(&mut self, session_id: &str) -> Result<(), Session_Error> {
        self.sessions.remove(session_id);
        Ok(())
    }

    /// 列出所有 Session
    pub fn list_sessions(&self) -> Vec<super::session::SessionInfo> {
        self.sessions
            .values()
            .map(|s| super::session::SessionInfo {
                session_id: s.session_id.clone(),
                model_id: s.model_id.clone(),
                total_slots: s.max_slots,
                occupied_slots: s.occupied_count(),
            })
            .collect()
    }

    // ─── 内部方法 ────────────────────────────────────────────

    pub fn allocate_slot(&mut self, session_id: &str) -> Result<SlotHandle, Session_Error> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| Session_Error::SessionNotFound(session_id.to_string()))?;

        let (token_tx, token_rx) = mpsc::unbounded_channel();

        let _slot_id = session
            .allocate(token_tx)
            .ok_or_else(|| Session_Error::SlotExhausted(session_id.to_string()))?;

        Ok(SlotHandle {
            prompt_tx: self.shared_prompt_tx.clone(),
            token_rx,
        })
    }

    pub fn close_slot(&mut self, session_id: &str, slot_id: usize) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.release(slot_id);
        }
    }

    // ─── 主循环分支方法 ──────────────────────────────────────

    /// 分支 B: 处理 prompt
    pub fn handle_prompt(&mut self, session_id: &str, slot_id: usize, text: &str) {
        let session = match self.sessions.get_mut(session_id) {
            Some(s) => s,
            None => return,
        };

        let slot = match session.get_slot_mut(slot_id) {
            Some(s) => s,
            None => return,
        };

        // TODO: 接入 tokenizer 做真实 encode
        // 占位：简单按字节转 u32
        let tokens: Vec<u32> = text.bytes().map(|b| b as u32).collect();
        slot.token_buf.extend(tokens);
        slot.dirty = true;
    }

    /// 分支 C+D: 目前占位（Task 6.3 实现）
    async fn flush_and_dispatch(&mut self) {
        // TODO: Task 6.3 — 收集 dirty slots，拼 batch，送 ML thread
        // TODO: Task 6.3 — 接收 logits，sample，decode，分发
        let _ = &self.batch_tx;
        let _ = &mut self.logits_rx;
    }

    // ══════════════════════════════════════════════════════════
    // 主循环
    // ══════════════════════════════════════════════════════════

    pub async fn run(mut self) {
        self.running = true;
        let mut flush_timer = tokio::time::interval(std::time::Duration::from_millis(100));

        loop {
            if !self.running {
                break;
            }

            tokio::select! {
                // A — 本地 open_slot
                Some(req) = self.open_slot_rx.recv() => {
                    let result = self.allocate_slot(&req.session_id);
                    req.reply_tx.send(result).ok();
                }

                // B — 收 Prompt
                Some((session_id, slot_id, text)) = self.shared_prompt_rx.recv() => {
                    self.handle_prompt(&session_id, slot_id, &text);
                }

                // C — Flush Batch（占位）
                _ = flush_timer.tick() => {
                    self.flush_and_dispatch().await;
                }

                // D — 收 Logits（占位，logits channel 暂无数据，不会被触发）
                _result = self.logits_rx.recv(), if false => {
                    // TODO: Task 6.3
                }

                // F — 销毁 slot
                Some(req) = self.close_slot_rx.recv() => {
                    self.close_slot(&req.session_id, req.slot_id);
                }
            }
        }
    }

    /// 停止主循环
    pub fn stop(&mut self) {
        self.running = false;
    }
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mgr() -> SessionManager {
        SessionManager::new(4)
    }

    #[test]
    fn test_create_and_list_sessions() {
        let mut mgr = make_mgr();
        let id = mgr.create_session("qwen3");
        assert!(id.starts_with("sess-"));

        let list = mgr.list_sessions();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_id, id);
        assert_eq!(list[0].model_id, "qwen3");
        assert_eq!(list[0].total_slots, 4);
        assert_eq!(list[0].occupied_slots, 0);
    }

    #[test]
    fn test_destroy_session() {
        let mut mgr = make_mgr();
        let id = mgr.create_session("qwen3");
        mgr.destroy_session(&id).unwrap();
        assert!(mgr.list_sessions().is_empty());
    }

    #[test]
    fn test_open_slot_success() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");

        let handle = mgr.allocate_slot(&sess_id).expect("should allocate");
        handle.submit(&sess_id, 0, "hello".into());
        // slot 0 被占用
        assert_eq!(mgr.list_sessions()[0].occupied_slots, 1);
    }

    #[test]
    fn test_open_slot_session_not_found() {
        let mut mgr = make_mgr();
        let result = mgr.allocate_slot("no-such");
        assert!(matches!(result, Err(Session_Error::SessionNotFound(_))));
    }

    #[test]
    fn test_open_slot_exhausted() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");

        // max_slots = 4, allocate 4 times
        assert!(mgr.allocate_slot(&sess_id).is_ok());
        assert!(mgr.allocate_slot(&sess_id).is_ok());
        assert!(mgr.allocate_slot(&sess_id).is_ok());
        assert!(mgr.allocate_slot(&sess_id).is_ok());
        // 第 5 次应该失败
        assert!(matches!(
            mgr.allocate_slot(&sess_id),
            Err(Session_Error::SlotExhausted(_))
        ));
    }

    #[test]
    fn test_close_slot_and_reallocate() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");

        mgr.allocate_slot(&sess_id).unwrap(); // slot 0
        assert_eq!(mgr.list_sessions()[0].occupied_slots, 1);

        mgr.close_slot(&sess_id, 0);
        assert_eq!(mgr.list_sessions()[0].occupied_slots, 0);

        // slot 0 重新可用
        assert!(mgr.allocate_slot(&sess_id).is_ok());
    }

    #[test]
    fn test_handle_prompt_appends_tokens() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");
        mgr.allocate_slot(&sess_id).unwrap();

        mgr.handle_prompt(&sess_id, 0, "hi");

        let session = mgr.sessions.get(&sess_id).unwrap();
        let slot = session.get_slot(0).unwrap();
        assert!(!slot.token_buf.is_empty());
        assert!(slot.dirty);
    }
}
