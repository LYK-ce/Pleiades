// Presented by KeJi
// Date ： 2026-05-14

//! Session 模块 — 推理会话管理器
//!
//! 负责管理推理会话的生命周期、槽位分配和文本 IO 通道。
//! - `create_session` — 创建会话 + channel pair
//! - `connect` — 申请槽位，获取 IoFrontend
//! - `destroy_session` — 销毁会话，回收资源

pub mod capability;
pub mod manager;

pub mod slot;
pub mod session;

// ─── 聚合导出 ───────────────────────────────────────────────

pub use capability::{Session_Capability, Session_Error, IoFrontend, IoHandle};
pub use manager::SessionManager;
pub use session::SessionInfo;
pub use slot::{Slot, SlotState};

// ─── 模块级集成测试 ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_imports_compile() {
        let _: fn(usize) -> SessionManager = SessionManager::new;
        fn _assert_trait_object(_: &dyn Session_Capability) {}
        let _ = Session_Error::SessionNotFound("test".to_string());
        let _ = Session_Error::SlotExhausted("test".to_string());
    }

    #[test]
    fn test_session_info_fields() {
        let info = SessionInfo {
            session_id: "sess-1".to_string(),
            model_id: "qwen3".to_string(),
            total_slots: 4,
            occupied_slots: 1,
        };
        assert_eq!(info.session_id, "sess-1");
        assert_eq!(info.occupied_slots, 1);
    }

    #[test]
    fn test_slot_create() {
        let slot = Slot::new(0);
        assert_eq!(slot.slot_id, 0);
    }
}
