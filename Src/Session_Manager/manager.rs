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
use super::batch::{BatchRequest, BatchResult, assemble_batch, sample_batch};

// ─── 消息类型 ───────────────────────────────────────────────

/// open_slot 请求
pub(crate) struct OpenSlotRequest {
    pub session_id: String,
    pub reply_tx: tokio::sync::oneshot::Sender<Result<SlotHandle, Session_Error>>,
}

// ─── SessionManagerHandle ───────────────────────────────────

/// SessionManager 的外部句柄（可安全放入 Arc 供多组件共享）
///
/// 持有所有通道的发送端，不包含主循环状态。
pub struct SessionManagerHandle {
    /// open_slot 请求通道
    pub open_slot_tx: mpsc::UnboundedSender<OpenSlotRequest>,
    /// 注入 ML Thread 返回 logits（供外部模拟或桥接）
    pub logits_tx: mpsc::Sender<BatchResult>,
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
    close_slot_tx: mpsc::UnboundedSender<(String, usize)>,
    close_slot_rx: mpsc::UnboundedReceiver<(String, usize)>,

    /// ML thread — batch 输入
    batch_tx: mpsc::Sender<BatchRequest>,
    batch_rx: mpsc::Receiver<BatchRequest>,

    /// ML thread — logits 输出
    logits_tx: mpsc::Sender<BatchResult>,
    logits_rx: mpsc::Receiver<BatchResult>,

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
        let (batch_tx, batch_rx) = mpsc::channel(1);
        let (logits_tx, logits_rx) = mpsc::channel(1);

        SessionManager {
            sessions: HashMap::new(),
            shared_prompt_tx,
            shared_prompt_rx,
            open_slot_tx,
            open_slot_rx,
            close_slot_tx,
            close_slot_rx,
            batch_tx,
            batch_rx,
            logits_tx,
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
    pub fn close_slot_sender(&self) -> mpsc::UnboundedSender<(String, usize)> {
        self.close_slot_tx.clone()
    }

    /// 获取 batch 接收端（供 ML Thread 或测试使用）
    pub fn take_batch_rx(&mut self) -> mpsc::Receiver<BatchRequest> {
        std::mem::replace(&mut self.batch_rx, mpsc::channel(1).1)
    }

    /// 获取 logits 发送端（供 ML Thread 或测试使用）
    pub fn logits_tx(&self) -> mpsc::Sender<BatchResult> {
        self.logits_tx.clone()
    }

    /// 创建外部句柄，供其他组件（Core、Network）使用
    pub fn handle(&self) -> SessionManagerHandle {
        SessionManagerHandle {
            open_slot_tx: self.open_slot_tx.clone(),
            logits_tx: self.logits_tx.clone(),
        }
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

        let slot_id = session
            .allocate(token_tx)
            .ok_or_else(|| Session_Error::SlotExhausted(session_id.to_string()))?;

        Ok(SlotHandle::new(
            session_id.to_string(),
            slot_id,
            self.shared_prompt_tx.clone(),
            self.close_slot_tx.clone(),
            token_rx,
        ))
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

    /// 分支 C+D: flush batch 组装 + 发送；分发结果
    async fn flush_and_dispatch(&mut self) {
        // ── 收集 dirty slots ──
        let mut dirty: Vec<(String, usize, Vec<u32>)> = Vec::new();
        for (sess_id, session) in self.sessions.iter() {
            for (slot_id, state) in session.slots.iter().enumerate() {
                if let super::slot::SlotState::Occupied(ref slot) = state {
                    if slot.dirty && !slot.token_buf.is_empty() {
                        dirty.push((sess_id.clone(), slot_id, slot.token_buf.clone()));
                    }
                }
            }
        }

        if let Some(batch) = assemble_batch(&dirty) {
            // 发送到 ML Thread
            if self.batch_tx.send(batch).await.is_ok() {
                // 清空已发送的 slot token_buf + dirty 标记
                for (sess_id, slot_id, _) in &dirty {
                    if let Some(session) = self.sessions.get_mut(sess_id) {
                        if let Some(slot) = session.get_slot_mut(*slot_id) {
                            slot.token_buf.clear();
                            slot.dirty = false;
                        }
                    }
                }
            }
        }
    }

    /// 分支 D: 接收 logits，sample + decode + 分发
    async fn dispatch_logits(&mut self, result: BatchResult) {
        // 收集各 slot 的 temperature
        let mut temperatures = std::collections::HashMap::new();
        for (sess_id, slot_id) in &result.slot_order {
            if let Some(session) = self.sessions.get(sess_id) {
                if let Some(slot) = session.get_slot(*slot_id) {
                    temperatures.insert((sess_id.clone(), *slot_id), slot.temperature);
                }
            }
        }

        let tokens = sample_batch(&result, &temperatures);

        for ((sess_id, slot_id), token) in tokens {
            // decode — 占位：把 token 当字符输出
            let text = format!("[{}]", token);

            if let Some(session) = self.sessions.get_mut(&sess_id) {
                if let Some(slot) = session.get_slot_mut(slot_id) {
                    slot.token_tx.send(text).ok();
                    // 追加 token 到历史（为下一轮 prefill 做准备）
                    slot.token_buf.push(token);
                }
            }
        }
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

                // D — 收 Logits → sample + decode + 分发
                Some(result) = self.logits_rx.recv() => {
                    self.dispatch_logits(result).await;
                }

                // F — 销毁 slot
                Some((session_id, slot_id)) = self.close_slot_rx.recv() => {
                    self.close_slot(&session_id, slot_id);
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
        handle.submit("hello".into());
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

    // ─── Batch 流程测试 ──────────────────────────────────

    #[tokio::test]
    async fn test_flush_assembles_and_sends_batch() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");
        mgr.allocate_slot(&sess_id).unwrap();
        mgr.allocate_slot(&sess_id).unwrap();

        // 往 slot 0 和 slot 1 各写入 tokens
        mgr.handle_prompt(&sess_id, 0, "a");
        mgr.handle_prompt(&sess_id, 1, "bc");

        let mut batch_rx = mgr.take_batch_rx();
        mgr.flush_and_dispatch().await;

        // 应该收到一个 BatchRequest
        let req = batch_rx.try_recv().expect("should receive batch");
        assert_eq!(req.slot_order.len(), 2);
        // 验证 slot 的 token_buf 已清空
        let session = mgr.sessions.get(&sess_id).unwrap();
        assert!(session.get_slot(0).unwrap().token_buf.is_empty());
        assert!(!session.get_slot(0).unwrap().dirty);
        assert!(session.get_slot(1).unwrap().token_buf.is_empty());
        assert!(!session.get_slot(1).unwrap().dirty);
    }

    #[tokio::test]
    async fn test_dispatch_logits_sends_tokens_to_slots() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");
        mgr.allocate_slot(&sess_id).unwrap();  // slot 0
        mgr.allocate_slot(&sess_id).unwrap();  // slot 1

        // 注入 mock result
        let result = BatchResult {
            logits_batches: vec![
                vec![0.1, 0.9, 0.0],   // slot 0 → 几乎一定是 index 1
                vec![0.5, 0.1, 0.4],   // slot 1 → 几乎一定是 index 0
            ],
            slot_order: vec![
                (sess_id.clone(), 0),
                (sess_id.clone(), 1),
            ],
        };

        // 设置低温度使 sample 确定
        {
            let session = mgr.sessions.get_mut(&sess_id).unwrap();
            session.get_slot_mut(0).unwrap().temperature = 0.01;
            session.get_slot_mut(1).unwrap().temperature = 0.01;
        }

        mgr.dispatch_logits(result).await;

        // 验证 token 被追加到 token_buf
        let session = mgr.sessions.get(&sess_id).unwrap();
        let s0 = session.get_slot(0).unwrap();
        let s1 = session.get_slot(1).unwrap();
        assert_eq!(s0.token_buf, vec![1]);  // index 1
        assert_eq!(s1.token_buf, vec![0]);  // index 0
    }

    #[tokio::test]
    async fn test_flush_no_dirty_slots_sends_nothing() {
        let mut mgr = make_mgr();
        let sess_id = mgr.create_session("qwen3");

        let mut batch_rx = mgr.take_batch_rx();
        mgr.flush_and_dispatch().await;

        // 没有 dirty slot → 不应该有 batch
        assert!(batch_rx.try_recv().is_err());
    }
}
