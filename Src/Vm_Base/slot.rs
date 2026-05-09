//Presented by KeJi
//Date ： 2026-05-09

//! Vm 槽位系统
//!
//! 提供通用的槽位文件（SlotFile）用于在指令间传递基础类型数据。
//! 领域类型（Tensor、IoHandle、PipelinePlan 等）由各模块自行管理，
//! 挂在持有 Vm 的结构体字段上，Vm 不依赖任何领域模块。

use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================
// SlotId
// ============================================================

/// 槽位编号，由各领域模块自行定义语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId(pub u32);

// ============================================================
// ConstValue — Const 指令可写入的字面量值
// ============================================================

/// 可用于 `Const` 指令的值子集（全部可 Clone）。
#[derive(Debug, Clone)]
pub enum ConstValue {
    Nil,
    Bool(bool),
    U64(u64),
    F64(f64),
    String(String),
    PathBuf(PathBuf),
    Error(String),
}

// ============================================================
// SlotValue — 可存入槽位的基础类型
// ============================================================

/// 封装可存入槽位的基础类型。
///
/// 领域类型不在此枚举中，由各模块在自身结构体上管理。
#[derive(Debug)]
pub enum SlotValue {
    Nil,
    Bool(bool),
    U64(u64),
    F64(f64),
    String(String),
    PathBuf(PathBuf),
    Error(String),
}

impl From<ConstValue> for SlotValue {
    fn from(cv: ConstValue) -> Self {
        match cv {
            ConstValue::Nil => SlotValue::Nil,
            ConstValue::Bool(b) => SlotValue::Bool(b),
            ConstValue::U64(v) => SlotValue::U64(v),
            ConstValue::F64(v) => SlotValue::F64(v),
            ConstValue::String(s) => SlotValue::String(s),
            ConstValue::PathBuf(p) => SlotValue::PathBuf(p),
            ConstValue::Error(e) => SlotValue::Error(e),
        }
    }
}

// ============================================================
// SlotFile
// ============================================================

/// `HashMap<SlotId, SlotValue>` 的包装，提供类型安全的访问方法。
#[derive(Debug, Default)]
pub struct SlotFile {
    slots: HashMap<SlotId, SlotValue>,
}

impl SlotFile {
    /// 创建一个空的槽位文件。
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
        }
    }

    /// 写入或覆盖槽位。旧值自然 Drop，RAII 资源自动释放。
    pub fn set(&mut self, slot: SlotId, value: SlotValue) {
        self.slots.insert(slot, value);
    }

    /// 只读引用，不转移所有权。
    pub fn get(&self, slot: SlotId) -> Option<&SlotValue> {
        self.slots.get(&slot)
    }

    /// 取出所有权，原槽位置 Nil。
    pub fn take(&mut self, slot: SlotId) -> Option<SlotValue> {
        let taken = self.slots.remove(&slot);
        if taken.is_some() {
            self.slots.insert(slot, SlotValue::Nil);
        }
        taken
    }

    /// 删除槽位，返回旧值。
    pub fn remove(&mut self, slot: SlotId) -> Option<SlotValue> {
        self.slots.remove(&slot)
    }

    // ---- 类型安全的便利方法 ----

    /// 读取 Bool 类型的槽位值。
    pub fn get_bool(&self, slot: SlotId) -> Result<bool, String> {
        match self.get(slot) {
            Some(SlotValue::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("Slot {} is not a Bool", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取 U64 类型的槽位值。
    pub fn get_u64(&self, slot: SlotId) -> Result<u64, String> {
        match self.get(slot) {
            Some(SlotValue::U64(v)) => Ok(*v),
            Some(_) => Err(format!("Slot {} is not a U64", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取 F64 类型的槽位值。
    pub fn get_f64(&self, slot: SlotId) -> Result<f64, String> {
        match self.get(slot) {
            Some(SlotValue::F64(v)) => Ok(*v),
            Some(_) => Err(format!("Slot {} is not a F64", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取 String 类型的槽位值（只读引用）。
    pub fn get_string(&self, slot: SlotId) -> Result<&String, String> {
        match self.get(slot) {
            Some(SlotValue::String(s)) => Ok(s),
            Some(_) => Err(format!("Slot {} is not a String", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取 PathBuf 类型的槽位值（只读引用）。
    pub fn get_path(&self, slot: SlotId) -> Result<&PathBuf, String> {
        match self.get(slot) {
            Some(SlotValue::PathBuf(p)) => Ok(p),
            Some(_) => Err(format!("Slot {} is not a PathBuf", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取 Error 类型的槽位值（只读引用）。
    pub fn get_error(&self, slot: SlotId) -> Result<&String, String> {
        match self.get(slot) {
            Some(SlotValue::Error(e)) => Ok(e),
            Some(_) => Err(format!("Slot {} is not an Error", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slotid_equality() {
        assert_eq!(SlotId(0), SlotId(0));
        assert_ne!(SlotId(0), SlotId(1));
    }

    #[test]
    fn constvalue_to_slotvalue_conversion() {
        assert!(matches!(SlotValue::from(ConstValue::Nil), SlotValue::Nil));
        assert!(matches!(
            SlotValue::from(ConstValue::Bool(true)),
            SlotValue::Bool(true)
        ));
        assert!(matches!(
            SlotValue::from(ConstValue::U64(42)),
            SlotValue::U64(42)
        ));
        assert!(matches!(SlotValue::from(ConstValue::F64(3.14)), SlotValue::F64(v) if v == 3.14));
        assert!(matches!(
            SlotValue::from(ConstValue::String("hi".into())),
            SlotValue::String(ref s) if s == "hi"
        ));
        assert!(matches!(
            SlotValue::from(ConstValue::Error("err".into())),
            SlotValue::Error(ref s) if s == "err"
        ));
    }

    #[test]
    fn slotfile_set_and_get() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::U64(100));
        assert_eq!(sf.get_u64(SlotId(0)).unwrap(), 100);
    }

    #[test]
    fn slotfile_get_bool() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::Bool(true));
        assert!(sf.get_bool(SlotId(0)).unwrap());
    }

    #[test]
    fn slotfile_get_f64() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::F64(1.5));
        assert_eq!(sf.get_f64(SlotId(0)).unwrap(), 1.5);
    }

    #[test]
    fn slotfile_get_string() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::String("hello".into()));
        assert_eq!(sf.get_string(SlotId(0)).unwrap(), "hello");
    }

    #[test]
    fn slotfile_get_path() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::PathBuf(PathBuf::from("/tmp")));
        assert_eq!(sf.get_path(SlotId(0)).unwrap(), &PathBuf::from("/tmp"));
    }

    #[test]
    fn slotfile_get_error() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::Error("boom".into()));
        assert_eq!(sf.get_error(SlotId(0)).unwrap(), "boom");
    }

    #[test]
    fn slotfile_type_mismatch_returns_err() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::U64(1));
        assert!(sf.get_bool(SlotId(0)).is_err());
        assert!(sf.get_string(SlotId(0)).is_err());
        assert!(sf.get_f64(SlotId(0)).is_err());
    }

    #[test]
    fn slotfile_empty_slot_returns_err() {
        let sf = SlotFile::new();
        assert!(sf.get_u64(SlotId(0)).is_err());
    }

    #[test]
    fn slotfile_take_removes_value() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::U64(42));
        let taken = sf.take(SlotId(0));
        assert!(matches!(taken, Some(SlotValue::U64(42))));
        assert!(sf.get_u64(SlotId(0)).is_err());
    }

    #[test]
    fn slotfile_take_leaves_nil() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::U64(42));
        sf.take(SlotId(0));
        assert!(matches!(sf.get(SlotId(0)), Some(SlotValue::Nil)));
    }

    #[test]
    fn slotfile_remove_deletes_entry() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::U64(42));
        sf.remove(SlotId(0));
        assert!(sf.get(SlotId(0)).is_none());
    }

    #[test]
    fn slotfile_set_overwrites() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::U64(1));
        sf.set(SlotId(0), SlotValue::U64(2));
        assert_eq!(sf.get_u64(SlotId(0)).unwrap(), 2);
    }

    #[test]
    fn multiple_slots_independent() {
        let mut sf = SlotFile::new();
        sf.set(SlotId(0), SlotValue::U64(1));
        sf.set(SlotId(1), SlotValue::String("hi".into()));
        sf.set(SlotId(2), SlotValue::Bool(true));
        assert_eq!(sf.get_u64(SlotId(0)).unwrap(), 1);
        assert_eq!(sf.get_string(SlotId(1)).unwrap(), "hi");
        assert!(sf.get_bool(SlotId(2)).unwrap());
    }

    #[test]
    fn slotfile_get_returns_none_for_nonexistent() {
        let sf = SlotFile::new();
        assert!(sf.get(SlotId(999)).is_none());
    }

    #[test]
    fn take_on_nonexistent_returns_none() {
        let mut sf = SlotFile::new();
        assert!(sf.take(SlotId(999)).is_none());
    }
}
