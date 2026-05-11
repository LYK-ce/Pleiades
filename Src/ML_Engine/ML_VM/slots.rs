//Presented by KeJi
//Date ： 2026-05-11

//! ML 领域槽位系统。
//!
//! 与 OrchestratorSlots 同构——HashMap<SlotId, MlSlotValue>。
//! 基础类型（String/bool/f64）走 Vm_Base::SlotFile，领域类型走 MlSlots。

use crate::vm_base::SlotId;
use candle_core::Tensor;
use std::collections::HashMap;

// ─── 约定槽位常量 ─────────────────────────────────────────────

pub const SLOT_TEXT1: SlotId = SlotId(1000);
pub const SLOT_TEXT2: SlotId = SlotId(1001);
pub const SLOT_TEXT3: SlotId = SlotId(1002);
pub const SLOT_TEXT4: SlotId = SlotId(1003);

pub const SLOT_TOKENS1: SlotId = SlotId(1010);
pub const SLOT_TOKENS2: SlotId = SlotId(1011);
pub const SLOT_TOKENS3: SlotId = SlotId(1012);
pub const SLOT_TOKENS4: SlotId = SlotId(1013);

pub const SLOT_TENSOR1: SlotId = SlotId(1020);
pub const SLOT_TENSOR2: SlotId = SlotId(1021);
pub const SLOT_TENSOR3: SlotId = SlotId(1022);
pub const SLOT_TENSOR4: SlotId = SlotId(1023);

pub const SLOT_FLAG1: SlotId = SlotId(1030);
pub const SLOT_FLAG2: SlotId = SlotId(1031);
pub const SLOT_FLAG3: SlotId = SlotId(1032);
pub const SLOT_FLAG4: SlotId = SlotId(1033);

pub const SLOT_META1: SlotId = SlotId(1040);
pub const SLOT_META2: SlotId = SlotId(1041);
pub const SLOT_META3: SlotId = SlotId(1042);
pub const SLOT_META4: SlotId = SlotId(1043);
pub const SLOT_META5: SlotId = SlotId(1044);
pub const SLOT_META6: SlotId = SlotId(1045);
pub const SLOT_META7: SlotId = SlotId(1046);
pub const SLOT_META8: SlotId = SlotId(1047);

// ─── MlSlotValue ──────────────────────────────────────────────

#[derive(Debug)]
pub enum MlSlotValue {
    TokenIds(Vec<u32>),
    Tensor(Tensor),
}

// ─── MlSlots ──────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct MlSlots {
    slots: HashMap<SlotId, MlSlotValue>,
}

impl MlSlots {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
        }
    }

    pub fn set(&mut self, slot: SlotId, value: MlSlotValue) {
        self.slots.insert(slot, value);
    }

    pub fn take(&mut self, slot: SlotId) -> Option<MlSlotValue> {
        self.slots.remove(&slot)
    }

    // ─── TokenIds 访问 ──────────────────────────────────────

    pub fn set_token_ids(&mut self, slot: SlotId, ids: Vec<u32>) {
        self.set(slot, MlSlotValue::TokenIds(ids));
    }

    pub fn take_token_ids(&mut self, slot: SlotId) -> Option<Vec<u32>> {
        match self.take(slot) {
            Some(MlSlotValue::TokenIds(ids)) => Some(ids),
            _ => None,
        }
    }

    pub fn get_token_ids(&self, slot: SlotId) -> Option<&[u32]> {
        match self.slots.get(&slot) {
            Some(MlSlotValue::TokenIds(ids)) => Some(ids.as_slice()),
            _ => None,
        }
    }

    pub fn append_token_id(&mut self, slot: SlotId, id: u32) {
        match self.slots.get_mut(&slot) {
            Some(MlSlotValue::TokenIds(ids)) => {
                ids.push(id);
            }
            _ => {
                let mut ids = Vec::new();
                ids.push(id);
                self.set(slot, MlSlotValue::TokenIds(ids));
            }
        }
    }

    pub fn clear_token_ids(&mut self, slot: SlotId) {
        if let Some(MlSlotValue::TokenIds(ids)) = self.slots.get_mut(&slot) {
            ids.clear();
        }
    }

    // ─── Tensor 访问 ────────────────────────────────────────

    pub fn set_tensor(&mut self, slot: SlotId, tensor: Tensor) {
        self.set(slot, MlSlotValue::Tensor(tensor));
    }

    pub fn take_tensor(&mut self, slot: SlotId) -> Option<Tensor> {
        match self.take(slot) {
            Some(MlSlotValue::Tensor(t)) => Some(t),
            _ => None,
        }
    }

    pub fn get_tensor(&self, slot: SlotId) -> Option<&Tensor> {
        match self.slots.get(&slot) {
            Some(MlSlotValue::Tensor(t)) => Some(t),
            _ => None,
        }
    }

    pub fn clear_tensor(&mut self, slot: SlotId) {
        self.slots.remove(&slot);
    }

    // ─── 全局操作 ──────────────────────────────────────────

    pub fn reset_all(&mut self) {
        self.slots.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_take_token_ids_roundtrip() {
        let mut slots = MlSlots::new();
        slots.set_token_ids(SLOT_TOKENS1, vec![1, 2, 3]);
        let taken = slots.take_token_ids(SLOT_TOKENS1).unwrap();
        assert_eq!(taken, vec![1, 2, 3]);
        assert!(slots.take_token_ids(SLOT_TOKENS1).is_none());
    }

    #[test]
    fn get_token_ids_readonly() {
        let mut slots = MlSlots::new();
        slots.set_token_ids(SLOT_TOKENS1, vec![5, 6]);
        let ids = slots.get_token_ids(SLOT_TOKENS1).unwrap();
        assert_eq!(ids, &[5, 6]);
        assert!(slots.get_token_ids(SLOT_TOKENS1).is_some());
    }

    #[test]
    fn append_token_id_new_slot() {
        let mut slots = MlSlots::new();
        slots.append_token_id(SLOT_TOKENS1, 42);
        assert_eq!(slots.get_token_ids(SLOT_TOKENS1).unwrap(), &[42]);
    }

    #[test]
    fn append_token_id_existing() {
        let mut slots = MlSlots::new();
        slots.set_token_ids(SLOT_TOKENS1, vec![1, 2]);
        slots.append_token_id(SLOT_TOKENS1, 3);
        assert_eq!(slots.get_token_ids(SLOT_TOKENS1).unwrap(), &[1, 2, 3]);
    }

    #[test]
    fn clear_token_ids() {
        let mut slots = MlSlots::new();
        slots.set_token_ids(SLOT_TOKENS1, vec![1, 2, 3]);
        slots.clear_token_ids(SLOT_TOKENS1);
        assert!(slots.get_token_ids(SLOT_TOKENS1).unwrap().is_empty());
    }

    #[test]
    fn set_and_take_tensor_roundtrip() {
        let mut slots = MlSlots::new();
        let t = Tensor::new(&[1.0f32, 2.0, 3.0], &candle_core::Device::Cpu).unwrap();
        let dims = t.dims().to_vec();
        slots.set_tensor(SLOT_TENSOR1, t);
        let taken = slots.take_tensor(SLOT_TENSOR1).unwrap();
        assert_eq!(taken.dims().to_vec(), dims);
        assert!(slots.take_tensor(SLOT_TENSOR1).is_none());
    }

    #[test]
    fn get_tensor_readonly() {
        let mut slots = MlSlots::new();
        let t = Tensor::new(&[1.0f32, 2.0, 3.0], &candle_core::Device::Cpu).unwrap();
        assert_eq!(t.dims(), &[3]);
        slots.set_tensor(SLOT_TENSOR1, t);
        let t_ref = slots.get_tensor(SLOT_TENSOR1).unwrap();
        assert_eq!(t_ref.dims(), &[3]);
    }

    #[test]
    fn clear_tensor() {
        let mut slots = MlSlots::new();
        let t = Tensor::new(&[1.0f32], &candle_core::Device::Cpu).unwrap();
        slots.set_tensor(SLOT_TENSOR1, t);
        slots.clear_tensor(SLOT_TENSOR1);
        assert!(slots.get_tensor(SLOT_TENSOR1).is_none());
    }

    #[test]
    fn take_wrong_type_returns_none() {
        let mut slots = MlSlots::new();
        slots.set_token_ids(SLOT_TOKENS1, vec![1, 2]);
        assert!(slots.take_tensor(SLOT_TOKENS1).is_none());
    }

    #[test]
    fn take_nonexistent_returns_none() {
        let mut slots = MlSlots::new();
        assert!(slots.take_token_ids(SlotId(9999)).is_none());
    }

    #[test]
    fn reset_all_clears_slots() {
        let mut slots = MlSlots::new();
        slots.set_token_ids(SLOT_TOKENS1, vec![1, 2, 3]);
        let t = Tensor::new(&[1.0f32], &candle_core::Device::Cpu).unwrap();
        slots.set_tensor(SLOT_TENSOR1, t);
        slots.reset_all();
        assert!(slots.get_token_ids(SLOT_TOKENS1).is_none());
        assert!(slots.get_tensor(SLOT_TENSOR1).is_none());
    }
}
