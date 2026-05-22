//Presented by KeJi
//Date ： 2026-05-22

//! Session 模块集成测试
//!
//! 验证 SessionManager 在多会话并发场景下的槽位分配和生命周期。

#![allow(non_snake_case)]

mod common;

use pleiades::session::{SessionManager, Session_Error};

/// TC-01: 多会话创建 + 销毁
#[test]
fn tc01_multi_session_create_destroy() {
    let mut mgr = SessionManager::new(20);
    let count = 10usize;

    let mut ids = Vec::new();
    for i in 0..count {
        let id = mgr.create_session(&format!("model_{}", i));
        assert!(!id.is_empty(), "session_id 不应为空");
        ids.push(id);
    }

    assert_eq!(mgr.list_sessions().len(), count);

    for id in &ids {
        mgr.destroy_session(id).unwrap();
    }
    assert!(mgr.list_sessions().is_empty());
}

/// TC-02: Slot 分配 + 释放
#[test]
fn tc02_slot_allocate_and_release() {
    let mut mgr = SessionManager::new(4);
    let sess_id = mgr.create_session("test_model");

    let _handle = mgr.allocate_slot(&sess_id).expect("slot 0");
    let _handle2 = mgr.allocate_slot(&sess_id).expect("slot 1");

    assert_eq!(mgr.list_sessions()[0].occupied_slots, 2);

    mgr.close_slot(&sess_id, 0);
    assert_eq!(mgr.list_sessions()[0].occupied_slots, 1);

    // slot 0 重新可用
    assert!(mgr.allocate_slot(&sess_id).is_ok());
}

/// TC-03: 高速创建+销毁压力
#[test]
fn tc03_high_speed_create_destroy() {
    let mut mgr = SessionManager::new(4);
    let count = 200usize;

    for i in 0..count {
        let id = mgr.create_session(&format!("model_{}", i));
        mgr.destroy_session(&id).unwrap();
    }

    assert!(mgr.list_sessions().is_empty(), "全部释放后应无残留会话");
}

/// TC-04: 槽位耗尽错误
#[test]
fn tc04_slot_exhausted() {
    let mut mgr = SessionManager::new(2);
    let sess_id = mgr.create_session("test_model");

    mgr.allocate_slot(&sess_id).unwrap();
    mgr.allocate_slot(&sess_id).unwrap();

    let result = mgr.allocate_slot(&sess_id);
    assert!(matches!(result, Err(Session_Error::SlotExhausted(_))));
}

/// TC-05: SlotHandle submit → prompt 直通 shared_prompt_rx
#[tokio::test]
async fn tc05_slot_handle_submit_to_channel() {
    let mut mgr = SessionManager::new(4);
    let sess_id = mgr.create_session("test_model");

    // 直接调用 allocate_slot 获取 handle
    let handle = mgr.allocate_slot(&sess_id).expect("allocate");

    // submit 应该把数据放到 shared_prompt_rx 里
    handle.submit(&sess_id, 0, "hello".into());

    // 验证 shared_prompt_rx 收到了
    let (sid, slot, text) = mgr.shared_prompt_rx.recv().await.unwrap();
    assert_eq!(sid, sess_id);
    assert_eq!(slot, 0);
    assert_eq!(text, "hello");
}

/// TC-06: 无法连接不存在的 session
#[test]
fn tc06_session_not_found() {
    let mut mgr = SessionManager::new(4);
    assert!(matches!(
        mgr.allocate_slot("no-such"),
        Err(Session_Error::SessionNotFound(_))
    ));
}
