//Presented by KeJi
//Date ： 2026-05-17

//! Session 模块集成测试
//!
//! 验证 SessionManager 在多会话并发场景下的通道隔离和生命周期。
//! 每个测试使用独立的 SessionManager 实例，测试间互不干扰。
//!
//! 注意：IoHandle（ML 侧）和 IoFrontend（前端侧）是两条独立通道，
//! 跨层桥接逻辑待 batching 实现。当前测试分别验证各层通道存活。

#![allow(non_snake_case)]

mod common;

use pleiades::session::{SessionManager, Session_Capability, Session_Error};
use std::sync::Arc;

/// TC-01: 多会话并发创建，验证 IoHandle 通道存活
///
/// 并发 create_session 10 个 → 每个 IoHandle 的 input_rx 应存活（对端未 drop）
#[tokio::test]
async fn tc01_multi_session_io_handle_alive() {
    let mgr = SessionManager::new(20);
    let session_count = 10usize;

    let mut io_handles = Vec::new();
    for i in 0..session_count {
        let (session_id, io_handle) = mgr.create_session(format!("model_{}", i)).await.unwrap();
        assert!(!session_id.is_empty(), "session_id 不应为空");
        io_handles.push(io_handle);
    }

    // 每个 IoHandle 的 input_rx 应存活（对端 ml_input_tx 在 Session 中）
    for io_handle in &mut io_handles {
        let result = io_handle.input_rx.try_recv();
        assert!(
            !matches!(result, Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)),
            "IoHandle.input_rx 应存活（对端存在于 Session 中）"
        );
    }

    // 验证 list_sessions
    let sessions = mgr.list_sessions();
    assert_eq!(sessions.len(), session_count, "应创建 10 个会话");
}

/// TC-02: connect 创建的 IoFrontend 通道存活
///
/// create_session → connect → IoFrontend 通道应存活 → drop IoFrontend → 对端关闭
#[tokio::test]
async fn tc02_frontend_channel_alive() {
    let mgr = SessionManager::new(4);

    let (session_id, _io_handle) = mgr.create_session("test_model".to_string()).await.unwrap();
    let (slot_id, frontend) = mgr.connect(&session_id).await.unwrap();
    assert_eq!(slot_id, 0);

    // IoFrontend 应能发送而不会立即报 SendError
    let result = frontend.input_tx.try_send("ping".to_string());
    assert!(
        result.is_ok(),
        "IoFrontend.input_tx 发送应成功（对端在 Session.frontend_pairs 中）"
    );

    // drop IoFrontend 后，对端的 input_rx 应收不到更多消息（内部 rx 存活但 tx 已断）
    drop(frontend);
    // 通过 destroy 验证 Session 被释放时对端也被清理
    mgr.destroy_session(&session_id).await.unwrap();
    assert!(mgr.list_sessions().is_empty());
}

/// TC-03: 高速吞吐 — session 创建与销毁压力
///
/// 快速创建+销毁 200 个 session，验证无泄漏/panic
#[tokio::test]
async fn tc03_high_speed_create_destroy() {
    let mgr = SessionManager::new(4);
    let count = 200usize;

    for i in 0..count {
        let (session_id, _io_handle) = mgr.create_session(format!("model_{}", i)).await.unwrap();
        mgr.destroy_session(&session_id).await.unwrap();
    }

    assert!(mgr.list_sessions().is_empty(), "全部释放后应无残留会话");
}

/// TC-04: 并发创建会话无竞争
///
/// spawn 20 个任务同时 create_session 不同 model_id → 全部成功 → list_sessions 验证
#[tokio::test]
async fn tc04_concurrent_create_no_contention() {
    let mgr = Arc::new(SessionManager::new(50));
    let task_count = 20u64;

    let mut handles = Vec::new();
    for i in 0..task_count {
        let mgr_clone = Arc::clone(&mgr);
        let handle = tokio::spawn(async move {
            let result = mgr_clone.create_session(format!("model_{}", i)).await;
            (i, result)
        });
        handles.push(handle);
    }

    for handle in handles {
        let (i, result) = handle.await.unwrap();
        assert!(
            result.is_ok(),
            "model_{} 并发 create_session 应成功",
            i
        );
    }

    let sessions = mgr.list_sessions();
    assert_eq!(sessions.len(), task_count as usize, "应创建 20 个会话");
}

/// TC-05: 槽位耗尽错误
///
/// create_session(max_slots=2) → connect 3 次 → 第 3 次返回 SlotExhausted
#[tokio::test]
async fn tc05_slot_exhausted_error() {
    let mgr = SessionManager::new(2);

    let (session_id, _io) = mgr.create_session("test_model".to_string()).await.unwrap();

    mgr.connect(&session_id).await.unwrap();
    mgr.connect(&session_id).await.unwrap();

    let result = mgr.connect(&session_id).await;
    assert!(result.is_err(), "第 3 次 connect 应返回错误");

    match result.unwrap_err() {
        Session_Error::SlotExhausted(msg) => {
            assert!(
                msg.contains(&session_id),
                "错误信息应包含 session_id，实际: {}",
                msg
            );
        }
        other => panic!("期望 SlotExhausted，实际: {:?}", other),
    }
}
