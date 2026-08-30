//Presented by KeJi
//Date ： 2026-05-17

//! PeerManagement 模块集成测试
//!
//! 使用 Peer_Management_Capability trait 作为统一入口，验证 CRUD、并发安全和生命周期管理。

#![allow(non_snake_case)]

mod common;

use pleiades_base::peer_management::{
    PeerInfo, Peer_Management_Error,
    PeerManager, PeerHandle, Peer_Management_Capability,
    PeerProfile, create_peer_management,
};
use libp2p::PeerId;
use std::sync::Arc;

/// 辅助函数：创建带有默认地址的 PeerInfo
fn make_peer(peer_id: PeerId) -> PeerInfo {
    PeerInfo::new(peer_id, vec![])
}

/// TC-01: 批量节点 CRUD
///
/// upsert 20 → get_peers = 20 → update 8 个心跳 → remove 5 → count = 15
#[tokio::test]
async fn tc01_bulk_peer_crud() {
    let (_manager, handle) = create_peer_management(PeerId::random());
    let mut peer_ids: Vec<PeerId> = Vec::new();

    // 1. upsert 20 个节点
    for _ in 0..20 {
        let peer_id = PeerId::random();
        peer_ids.push(peer_id);
        handle.Upsert_Peer(make_peer(peer_id)).await;
    }

    // 2. Get_Peers = 20
    let all_peers = handle.Get_Peers().await.unwrap();
    assert_eq!(all_peers.len(), 20, "添加 20 个节点后 Get_Peers 应返回 20");

    // 3. 对前 8 个节点更新心跳（设置延迟）
    for i in 0..8 {
        let profile = PeerProfile { latency_ms: Some(10), ..PeerProfile::default() };
        handle.Update_Profile(&peer_ids[i], profile).await.unwrap();
    }

    // 4. remove 5 个节点（取后 5 个）
    for i in 15..20 {
        handle.Remove_Peer(&peer_ids[i]).await.unwrap();
    }

    // 5. Get_Peers().len() = 15 (排除本地节点)
    let count = handle.Get_Peers().await.unwrap().len();
    assert_eq!(count, 15, "移除 5 个后应剩 15 个远程节点");

    // 6. 已移除的节点 Get_Peer 应返回 PeerNotFound
    let result = handle.Get_Peer(&peer_ids[19]).await;
    assert!(
        result.is_err(),
        "已移除的节点 Get_Peer 应返回错误"
    );
}

/// TC-02: 心跳与超时清理
///
/// upsert 5 → sleep → 仅 Update_Profile 3 个 → Cleanup_Timeout_Peers(1) → 清除 2 → count = 3
#[tokio::test]
async fn tc02_heartbeat_and_timeout_cleanup() {
    let (_manager, handle) = create_peer_management(PeerId::random());
    let mut peer_ids: Vec<PeerId> = Vec::new();

    // 1. upsert 5 个节点
    for _ in 0..5 {
        let peer_id = PeerId::random();
        peer_ids.push(peer_id);
        handle.Upsert_Peer(make_peer(peer_id)).await;
    }
    assert_eq!(handle.Get_Peers().await.unwrap().len(), 5);

    // 2. 等待足够时间使所有节点"超时"（> 1 秒）
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    // 3. 仅对前 3 个更新心跳（重置 last_active）
    for i in 0..3 {
        let profile = PeerProfile { latency_ms: Some(10), ..PeerProfile::default() };
        handle.Update_Profile(&peer_ids[i], profile).await.unwrap();
    }

    // 4. Cleanup_Timeout_Peers(1) → 应清除 elapsed >= 1s 的节点（后 2 个）
    let removed = handle.Cleanup_Timeout_Peers(1).await.unwrap();
    assert_eq!(removed, 2, "应清除 2 个超时节点");

    // 5. Get_Peers().len() = 3 (排除本地节点)
    let count = handle.Get_Peers().await.unwrap().len();
    assert_eq!(count, 3, "清理后应剩 3 个远程节点");

    // 6. 验证存活的是前 3 个
    for i in 0..3 {
        assert!(
            handle.Contains_Peer(&peer_ids[i]).await.unwrap(),
            "心跳更新过的节点 {} 应存活",
            i
        );
    }
    for i in 3..5 {
        assert!(
            !handle.Contains_Peer(&peer_ids[i]).await.unwrap(),
            "未更新心跳的节点 {} 应被清除",
            i
        );
    }
}

/// TC-03: 并发安全
///
/// spawn 10 任务同时 upsert/update/list → 无 panic，最终 count 一致
#[tokio::test]
async fn tc03_concurrent_safety() {
    let manager = Arc::new(PeerManager::new(PeerId::random()));
    let handle = Arc::new(PeerHandle::new(manager.clone()));
    let task_count = 10usize;
    let peers_per_task = 5usize;

    let mut join_handles = Vec::new();
    for _task_id in 0..task_count {
        let h = Arc::clone(&handle);
        let jh = tokio::spawn(async move {
            let mut task_peers = Vec::new();
            for _ in 0..peers_per_task {
                let peer_id = PeerId::random();
                task_peers.push(peer_id);
                h.Upsert_Peer(make_peer(peer_id)).await;
            }
            // 穿插读操作
            let _list = h.Get_Peers().await.unwrap();
            let _count = h.Count().await.unwrap();

            // 穿插 Contains_Peer 检查
            if !task_peers.is_empty() {
                assert!(h.Contains_Peer(&task_peers[0]).await.unwrap());
            }

            task_peers
        });
        join_handles.push(jh);
    }

    // 等待全部完成 — 不应 panic
    let mut all_peers = Vec::new();
    for jh in join_handles {
        let peers = jh.await.unwrap();
        all_peers.extend(peers);
    }

    // 最终 Get_Peers().len() 应等于所有任务添加的总和 (排除本地节点)
    let expected_total = task_count * peers_per_task;
    let final_count = handle.Get_Peers().await.unwrap().len();
    assert_eq!(
        final_count, expected_total,
        "并发添加后 count 应为 {} ({}×{})",
        expected_total, task_count, peers_per_task
    );

    // 验证每个 peer_id 都可查到
    for peer_id in &all_peers {
        assert!(
            handle.Contains_Peer(peer_id).await.unwrap(),
            "并发添加的节点应全部可查到"
        );
    }
}

/// TC-04: Profile 更新与查询
///
/// upsert peer → Update_Profile → Get_Peer → 验证 profile 字段
/// → Update_Profile → 验证延迟
#[tokio::test]
async fn tc04_profile_update_and_query() {
    let (_manager, handle) = create_peer_management(PeerId::random());
    let peer_id = PeerId::random();
    handle.Upsert_Peer(make_peer(peer_id)).await;

    // 1. 初始状态：profile 字段均为 None
    let info = handle.Get_Peer(&peer_id).await.unwrap();
    assert!(info.profile.bandwidth_mbps.is_none(), "初始 bandwidth 应为 None");
    assert!(info.profile.latency_ms.is_none(), "初始 latency 应为 None");

    // 2. Update_Profile: 设置带宽和内存
    let profile = PeerProfile {
        bandwidth_mbps: Some(1000),
        memory_mb: Some(8192),
        ..PeerProfile::default()
    };
    handle.Update_Profile(&peer_id, profile).await.unwrap();

    let info = handle.Get_Peer(&peer_id).await.unwrap();
    assert_eq!(info.profile.bandwidth_mbps, Some(1000), "带宽应为 1000 Mbps");
    assert_eq!(info.profile.memory_mb, Some(8192), "内存应为 8192 MB");

    // 3. Update_Profile: 设置延迟
    let profile = PeerProfile { latency_ms: Some(25), ..PeerProfile::default() };
    handle.Update_Profile(&peer_id, profile).await.unwrap();

    let info = handle.Get_Peer(&peer_id).await.unwrap();
    assert_eq!(info.profile.latency_ms, Some(25), "延迟应为 25ms");
}

/// TC-05: 空管理器操作
///
/// 空 manager → Get_Peer = Err → Get_Peers = empty → Remove_Peer = Err → Count = 0
#[tokio::test]
async fn tc05_empty_manager_operations() {
    let (_manager, handle) = create_peer_management(PeerId::random());
    let fake_peer_id = PeerId::random();

    // 1. Get_Peers().len() = 0 (排除本地节点)
    assert_eq!(handle.Get_Peers().await.unwrap().len(), 0, "空管理器 Get_Peers 应为 0");

    // 2. Get_Peers = empty
    let peers = handle.Get_Peers().await.unwrap();
    assert!(peers.is_empty(), "空管理器 Get_Peers 应返回空 Vec");

    // 3. Get_Peer = Err(PeerNotFound)
    let result = handle.Get_Peer(&fake_peer_id).await;
    assert!(result.is_err(), "空管理器 Get_Peer 应返回错误");
    match result.unwrap_err() {
        Peer_Management_Error::PeerNotFound(_) => {}
        other => panic!("期望 PeerNotFound，实际: {:?}", other),
    }

    // 4. Remove_Peer = Err(PeerNotFound)
    let result = handle.Remove_Peer(&fake_peer_id).await;
    assert!(result.is_err(), "空管理器 Remove_Peer 应返回错误");

    // 5. Cleanup_Timeout_Peers = 0 removed
    let removed = handle.Cleanup_Timeout_Peers(60).await.unwrap();
    assert_eq!(removed, 0, "空管理器 Cleanup 应移除 0 个");
}

/// TC-06: PeerHandle 共享（通过同一 PeerManager 创建两个 PeerHandle）
///
/// 两个 handle 操作同一 manager → 数据一致
#[tokio::test]
async fn tc06_peer_handle_shared() {
    let manager = Arc::new(PeerManager::new(PeerId::random()));
    let handle_1 = PeerHandle::new(manager.clone());
    let handle_2 = PeerHandle::new(manager.clone());

    let peer_a = PeerId::random();
    let peer_b = PeerId::random();

    // 1. handle_1 添加 peer_a
    handle_1.Upsert_Peer(make_peer(peer_a)).await;

    // 2. handle_2 能立即看到 peer_a
    assert!(
        handle_2.Contains_Peer(&peer_a).await.unwrap(),
        "另一个 handle 应看到第一个 handle 添加的节点"
    );

    // 3. handle_2 添加 peer_b
    handle_2.Upsert_Peer(make_peer(peer_b)).await;

    // 4. handle_1 能看到 peer_b
    assert!(
        handle_1.Contains_Peer(&peer_b).await.unwrap(),
        "第一个 handle 应看到第二个 handle 添加的节点"
    );

    // 5. 两个 handle 的 Get_Peers().len() 一致 (排除本地节点)
    let count_1 = handle_1.Get_Peers().await.unwrap().len();
    let count_2 = handle_2.Get_Peers().await.unwrap().len();
    assert_eq!(count_1, 2, "handle_1 Get_Peers 应为 2");
    assert_eq!(count_2, 2, "handle_2 Get_Peers 应为 2");

    // 6. handle_1 更新 Profile，handle_2 能看到
    let profile = PeerProfile {
        bandwidth_mbps: Some(500),
        ..PeerProfile::default()
    };
    handle_1.Update_Profile(&peer_a, profile).await.unwrap();

    let info_from_h2 = handle_2.Get_Peer(&peer_a).await.unwrap();
    assert_eq!(
        info_from_h2.profile.bandwidth_mbps,
        Some(500),
        "handle_2 应看到 handle_1 的 profile 更新"
    );

    // 7. handle_2 删除 peer_b，handle_1 同步感知
    handle_2.Remove_Peer(&peer_b).await.unwrap();
    assert_eq!(
        handle_1.Get_Peers().await.unwrap().len(),
        1,
        "删除后 handle_1 Get_Peers 应为 1"
    );
    assert!(
        !handle_1.Contains_Peer(&peer_b).await.unwrap(),
        "handle_2 删除的节点在 handle_1 中也不应存在"
    );
}
