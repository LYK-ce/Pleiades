//Presented by KeJi
//Date ： 2026-05-22

//! Session 模块集成测试

#![allow(non_snake_case)]

mod common;

use pleiades::session::{SessionManager, Session_Error};

#[test]
fn tc01_multi_session_create_destroy() {
    let mut mgr = SessionManager::new(20);
    let count = 10usize;

    let mut ids = Vec::new();
    for i in 0..count {
        let id = mgr.create_session(&format!("model_{}", i));
        assert!(id > 0, "session_id should be positive");
        ids.push(id);
    }

    assert_eq!(mgr.list_sessions().len(), count);

    for id in &ids {
        mgr.destroy_session(*id).unwrap();
    }
    assert!(mgr.list_sessions().is_empty());
}

#[test]
fn tc02_slot_allocate_and_release() {
    let mut mgr = SessionManager::new(4);
    let sess_id = mgr.create_session("test_model");

    let _handle = mgr.allocate_slot(sess_id).expect("slot 0");
    let _handle2 = mgr.allocate_slot(sess_id).expect("slot 1");

    assert_eq!(mgr.list_sessions()[0].occupied_slots, 2);

    mgr.close_slot(sess_id, 0);
    assert_eq!(mgr.list_sessions()[0].occupied_slots, 1);

    assert!(mgr.allocate_slot(sess_id).is_ok());
}

#[test]
fn tc03_high_speed_create_destroy() {
    let mut mgr = SessionManager::new(4);
    let count = 200usize;

    for i in 0..count {
        let id = mgr.create_session(&format!("model_{}", i));
        mgr.destroy_session(id).unwrap();
    }

    assert!(mgr.list_sessions().is_empty());
}

#[test]
fn tc04_slot_exhausted() {
    let mut mgr = SessionManager::new(2);
    let sess_id = mgr.create_session("test_model");

    mgr.allocate_slot(sess_id).unwrap();
    mgr.allocate_slot(sess_id).unwrap();

    let result = mgr.allocate_slot(sess_id);
    assert!(matches!(result, Err(Session_Error::SlotExhausted(_))));
}

#[tokio::test]
async fn tc05_slot_handle_submit_to_channel() {
    let mut mgr = SessionManager::new(4);
    let sess_id = mgr.create_session("test_model");

    let handle = mgr.allocate_slot(sess_id).expect("allocate");
    handle.submit("hello".into());

    let (sid, slot, text) = mgr.shared_prompt_rx.recv().await.unwrap();
    assert_eq!(sid, sess_id);
    assert_eq!(slot, 0);
    assert_eq!(text, "hello");
}

#[test]
fn tc06_session_not_found() {
    let mut mgr = SessionManager::new(4);
    assert!(matches!(
        mgr.allocate_slot(999),
        Err(Session_Error::SessionNotFound(_))
    ));
}
