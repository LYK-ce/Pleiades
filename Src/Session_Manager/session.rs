//Presented by KeJi
//Date ： 2026-05-14

use super::slot::{Slot, SlotState};
use tokio::sync::mpsc;

/// 内部会话结构
///
/// 持有通向 ML Thread 的通道对端（input 发送端、output 接收端），
/// 以及 connect 创建的各槽位通道对端。
pub(crate) struct Session {
    pub session_id: String,
    pub model_id: String,
    pub max_slots: usize,
    pub slots: Vec<Slot>,
    pub slot_counter: u32,
    /// ML Thread input 通道的发送端 — create_session 时存入，不再 drop
    pub(crate) ml_input_tx: mpsc::Sender<String>,
    /// ML Thread output 通道的接收端 — create_session 时存入，不再 drop
    pub(crate) ml_output_rx: mpsc::Receiver<String>,
    /// connect 时创建的槽位通道对端 (前端 input 的接收端, 前端 output 的发送端)
    pub(crate) frontend_pairs: Vec<(mpsc::Receiver<String>, mpsc::Sender<String>)>,
}

impl Session {
    pub fn new(
        session_id: String,
        model_id: String,
        max_slots: usize,
        ml_input_tx: mpsc::Sender<String>,
        ml_output_rx: mpsc::Receiver<String>,
    ) -> Self {
        let slots: Vec<Slot> = (0..max_slots as u32).map(Slot::new).collect();
        Session {
            session_id,
            model_id,
            max_slots,
            slots,
            slot_counter: 0,
            ml_input_tx,
            ml_output_rx,
            frontend_pairs: Vec::new(),
        }
    }

    /// 统计已占用槽位数
    pub fn occupied_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s.state, SlotState::Occupied { .. }))
            .count()
    }
}

/// 会话公开视图
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub model_id: String,
    pub total_slots: usize,
    pub occupied_slots: usize,
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_new_all_vacant() {
        let (tx, _rx) = mpsc::channel::<String>(1);
        let (_tx2, rx2) = mpsc::channel::<String>(1);
        let session = Session::new("sess-1".to_string(), "qwen3".to_string(), 4, tx, rx2);
        assert_eq!(session.session_id, "sess-1");
        assert_eq!(session.slots.len(), 4);
        assert_eq!(session.occupied_count(), 0);
    }

    #[test]
    fn test_session_slot_occupied_count() {
        let (tx, _rx) = mpsc::channel::<String>(1);
        let (_tx2, rx2) = mpsc::channel::<String>(1);
        let mut session = Session::new("sess-2".to_string(), "qwen3".to_string(), 4, tx, rx2);
        session.slots[0].state = SlotState::Occupied {
            owner: "tui".to_string(),
        };
        session.slots[2].state = SlotState::Occupied {
            owner: "http:8961".to_string(),
        };
        assert_eq!(session.occupied_count(), 2);
    }
}
