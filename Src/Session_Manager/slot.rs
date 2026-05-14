//Presented by KeJi
//Date ： 2026-05-14

/// 槽位状态
#[derive(Debug, Clone)]
pub enum SlotState {
    Vacant,
    Occupied { owner: String },
}

/// 槽位结构
#[derive(Debug, Clone)]
pub struct Slot {
    pub slot_id: u32,
    pub state: SlotState,
}

impl Slot {
    pub fn new(slot_id: u32) -> Self {
        Slot {
            slot_id,
            state: SlotState::Vacant,
        }
    }
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_new_is_vacant() {
        let slot = Slot::new(0);
        assert_eq!(slot.slot_id, 0);
        assert!(matches!(slot.state, SlotState::Vacant));
    }

    #[test]
    fn test_slot_occupied() {
        let mut slot = Slot::new(1);
        slot.state = SlotState::Occupied {
            owner: "tui".to_string(),
        };
        match &slot.state {
            SlotState::Occupied { owner } => assert_eq!(owner, "tui"),
            _ => panic!("expected occupied"),
        }
    }
}
