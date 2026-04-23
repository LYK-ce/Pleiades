// Presented by KeJi
// Date ： 2026-04-23

use std::collections::HashMap;
use std::path::PathBuf;
use crate::llm_io;

/// 槽位编号，类似寄存器索引。由 `TaskProgramBuilder` 在编译时分配。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId(pub u32);

/// Phase 2 设备租约的占位符。
#[derive(Debug, Clone)]
pub struct DeviceLease;

/// Phase 2 ML Session 句柄的占位符。
#[derive(Debug, Clone)]
pub struct SessionHandle;

/// 可用于 `Const` 指令的值子集（全部可 Clone）。
/// 不包含 IoHandle 等不可 Clone 的运行时资源。
#[derive(Debug, Clone)]
pub enum ConstValue {
    /// 空值
    Nil,
    /// 控制流条件
    Bool(bool),
    /// 数值参数
    U64(u64),
    /// 字符串参数
    String(String),
    /// 路径参数
    PathBuf(PathBuf),
    /// 错误信息
    Error(String),
}

/// 封装所有可存入槽位的类型。
/// **注意**：不实现 Clone，因为 IoHandle 含 mpsc::Receiver 不可克隆。
#[derive(Debug)]
pub enum SlotValue {
    /// 空槽位，初始状态或 `take` 后的残留
    Nil,
    /// 控制流条件（`JumpIf`）
    Bool(bool),
    /// 数值参数（如 `max_tokens`）
    U64(u64),
    /// 设备名、错误描述
    String(String),
    /// 模型路径、文件位置
    PathBuf(PathBuf),
    /// 计算设备租约（RAII，Phase 2 先用 Stub）
    DeviceLease(DeviceLease),
    /// ML Session 句柄（Phase 2 先用 Stub）
    SessionHandle(SessionHandle),
    /// 步骤执行失败的错误传递
    Error(String),
    /// LLM IO 句柄（含 mpsc::Receiver，只能 take 不能 clone/get）
    IoHandle(llm_io::IoHandle),
}

/// 从 ConstValue 到 SlotValue 的无损转换
impl From<ConstValue> for SlotValue {
    fn from(cv: ConstValue) -> Self {
        match cv {
            ConstValue::Nil => SlotValue::Nil,
            ConstValue::Bool(b) => SlotValue::Bool(b),
            ConstValue::U64(v) => SlotValue::U64(v),
            ConstValue::String(s) => SlotValue::String(s),
            ConstValue::PathBuf(p) => SlotValue::PathBuf(p),
            ConstValue::Error(e) => SlotValue::Error(e),
        }
    }
}

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

    /// 写入或覆盖槽位。旧值自然 `Drop`，RAII 资源自动释放。
    pub fn set(&mut self, slot: SlotId, value: SlotValue) {
        self.slots.insert(slot, value);
    }

    /// 只读引用，不转移所有权。
    pub fn get(&self, slot: SlotId) -> Option<&SlotValue> {
        self.slots.get(&slot)
    }

    /// 取出所有权，原槽位置 `Nil`。
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

    /// 读取字符串类型的槽位值，类型不匹配时返回错误。
    pub fn get_string(&self, slot: SlotId) -> Result<&String, String> {
        match self.get(slot) {
            Some(SlotValue::String(s)) => Ok(s),
            Some(_) => Err(format!("Slot {} is not a String", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取路径类型的槽位值，类型不匹配时返回错误。
    pub fn get_path(&self, slot: SlotId) -> Result<&PathBuf, String> {
        match self.get(slot) {
            Some(SlotValue::PathBuf(p)) => Ok(p),
            Some(_) => Err(format!("Slot {} is not a PathBuf", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取布尔类型的槽位值，类型不匹配时返回错误。
    pub fn get_bool(&self, slot: SlotId) -> Result<bool, String> {
        match self.get(slot) {
            Some(SlotValue::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("Slot {} is not a Bool", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 取出设备租约，类型不匹配时返回错误。
    pub fn take_device(&mut self, slot: SlotId) -> Result<DeviceLease, String> {
        match self.take(slot) {
            Some(SlotValue::DeviceLease(d)) => Ok(d),
            Some(_) => Err(format!("Slot {} is not a DeviceLease", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 取出 Session 句柄，类型不匹配时返回错误。
    pub fn take_session(&mut self, slot: SlotId) -> Result<SessionHandle, String> {
        match self.take(slot) {
            Some(SlotValue::SessionHandle(s)) => Ok(s),
            Some(_) => Err(format!("Slot {} is not a SessionHandle", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取 U64 类型的槽位值，类型不匹配时返回错误。
    pub fn get_u64(&self, slot: SlotId) -> Result<u64, String> {
        match self.get(slot) {
            Some(SlotValue::U64(v)) => Ok(*v),
            Some(_) => Err(format!("Slot {} is not a U64", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 读取错误类型的槽位值，类型不匹配时返回错误。
    pub fn get_error(&self, slot: SlotId) -> Result<&String, String> {
        match self.get(slot) {
            Some(SlotValue::Error(e)) => Ok(e),
            Some(_) => Err(format!("Slot {} is not an Error", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }

    /// 取出 IoHandle，类型不匹配时返回错误。
    /// IoHandle 不可 Clone，只能 take 一次。
    pub fn take_io_handle(&mut self, slot: SlotId) -> Result<llm_io::IoHandle, String> {
        match self.take(slot) {
            Some(SlotValue::IoHandle(h)) => Ok(h),
            Some(_) => Err(format!("Slot {} is not an IoHandle", slot.0)),
            None => Err(format!("Slot {} is empty", slot.0)),
        }
    }
}