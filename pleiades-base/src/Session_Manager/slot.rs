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
    /// 返回 token 给接入方
    pub token_tx: mpsc::UnboundedSender<String>,
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
            token_tx: tx,
        };
        assert_eq!(slot.id, 0);
    }

    #[test]
    fn test_slot_state_vacant_and_occupied() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = SlotState::Vacant;
        assert!(matches!(state, SlotState::Vacant));

        state = SlotState::Occupied(Slot {
            id: 1,
            token_tx: tx,
        });

        match state {
            SlotState::Occupied(ref s) => {
                assert_eq!(s.id, 1);
            }
            _ => panic!("expected Occupied"),
        }
    }
}
