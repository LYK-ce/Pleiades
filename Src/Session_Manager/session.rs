//Presented by KeJi
//Date ： 2026-05-14

use super::slot::{Slot, SlotState};

/// 内部会话结构
///
/// IoHandle 不存储于此——create_session 创建后直接返回给调用方，
/// 由调用方自行传给 ML Engine。
pub(crate) struct Session {
    pub session_id: String,
    pub model_id: String,
    pub max_slots: usize,
    pub slots: Vec<Slot>,
    pub slot_counter: u32,
}

impl Session {
    pub fn new(session_id: String, model_id: String, max_slots: usize) -> Self {
        let slots: Vec<Slot> = (0..max_slots as u32).map(Slot::new).collect();
        Session {
            session_id,
            model_id,
            max_slots,
            slots,
            slot_counter: 0,
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
        let session = Session::new("sess-1".to_string(), "qwen3".to_string(), 4);
        assert_eq!(session.session_id, "sess-1");
        assert_eq!(session.slots.len(), 4);
        assert_eq!(session.occupied_count(), 0);
    }

    #[test]
    fn test_session_slot_occupied_count() {
        let mut session = Session::new("sess-2".to_string(), "qwen3".to_string(), 4);
        session.slots[0].state = SlotState::Occupied {
            owner: "tui".to_string(),
        };
        session.slots[2].state = SlotState::Occupied {
            owner: "http:8961".to_string(),
        };
        assert_eq!(session.occupied_count(), 2);
    }
}
