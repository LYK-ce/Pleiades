//Presented by KeJi
//Date ： 2026-05-22

//! Slot 管理 — 对话级隔离单元
//!
//! 每个 Session 内包含固定数量的 Slot。
//! Slot 是对话的最小隔离单元：持有独立 token_buf 和返回通道。

use tokio::sync::mpsc;

/// 槽位状态
pub enum SlotState {
    Vacant,
    Occupied(Slot),
}

/// 活跃槽位
pub struct Slot {
    /// 在当前 Session 内的索引
    pub id: usize,
    /// 累积的 token 历史（下一轮 flush 送入 ML）
    pub token_buf: Vec<u32>,
    /// 返回 token 给接入方
    pub token_tx: mpsc::UnboundedSender<String>,
    /// token_buf 有新内容，等待 flush
    pub dirty: bool,
    /// 采样温度
    pub temperature: f64,
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_creation() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let slot = Slot {
            id: 0,
            token_buf: Vec::new(),
            token_tx: tx,
            dirty: false,
            temperature: 0.8,
        };
        assert_eq!(slot.id, 0);
        assert!(!slot.dirty);
        assert_eq!(slot.temperature, 0.8);
    }

    #[test]
    fn test_slot_state_vacant_and_occupied() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = SlotState::Vacant;
        assert!(matches!(state, SlotState::Vacant));

        state = SlotState::Occupied(Slot {
            id: 1,
            token_buf: vec![101, 204],
            token_tx: tx,
            dirty: true,
            temperature: 0.3,
        });

        match state {
            SlotState::Occupied(ref s) => {
                assert_eq!(s.id, 1);
                assert_eq!(s.token_buf, vec![101, 204]);
                assert!(s.dirty);
                assert_eq!(s.temperature, 0.3);
            }
            _ => panic!("expected Occupied"),
        }
    }
}
