//Presented by KeJi
//Date ： 2026-04-28

//! PeerManagement 模块集成测试
//!
//! 为 PeerManagement 模块建立完整测试覆盖。
//! 使用 Peer_Management_Capability trait 作为统一入口，验证 CRUD、并发安全和生命周期管理。

#![allow(non_snake_case)]

mod common;

use pleiades::peer_management::{
    PeerInfo, PeerStatus, PeerCapability, Peer_Management_Error,
    PeerManager, PeerHandle, Peer_Management_Capability,
    create_peer_management,
};
use libp2p::PeerId;
use std::sync::Arc;

/// 辅助函数：创建带有默认地址的 PeerInfo
fn make_peer(peer_id: PeerId) -> PeerInfo {
    PeerInfo::new(peer_id, vec![])
}

/// TC-01: 批量节点 CRUD
///
/// add 20 → list = 20 → get_idle = Connected 数
/// → update_status Busy → get_idle 验证 → remove 5 → count = 15
#[tokio::test]
async fn tc01_bulk_peer_crud() {
    let (_manager, handle) = create_peer_management(PeerId::random());
    let mut peer_ids: Vec<PeerId> = Vec::new();

    // 1. add 20 个节点
    for _ in 0..20 {
        let peer_id = PeerId::random();
        peer_ids.push(peer_id);
        handle.Add_Peer(make_peer(peer_id)).await.unwrap();
    }

    // 2. list = 20
    let all_peers = handle.List_Peers().await.unwrap();
    assert_eq!(all_peers.len(), 20, "添加 20 个节点后 list 应返回 20");

    // 3. 所有节点初始状态为 Connected → Get_Idle_Peers = 20
    let idle_peers = handle.Get_Idle_Peers().await.unwrap();
    assert_eq!(
        idle_peers.len(),
        20,
        "初始状态全部为 Connected，get_idle 应返回 20"
    );

    // 4. 将前 8 个节点设为 Busy
    for i in 0..8 {
        handle
            .Update_Status(&peer_ids[i], PeerStatus::Busy)
            .await
            .unwrap();
    }

    // 5. 验证 idle 减少 → Get_Idle_Peers = 12 (20 - 8)
    let idle_peers = handle.Get_Idle_Peers().await.unwrap();
    assert_eq!(idle_peers.len(), 12, "设置 8 个为 Busy 后空闲节点应为 12");

    // 6. remove 5 个节点（取后 5 个）
    for i in 15..20 {
        handle.Remove_Peer(&peer_ids[i]).await.unwrap();
    }

    // 7. count = 15
    let count = handle.Count().await.unwrap();
    assert_eq!(count, 15, "移除 5 个后应剩 15 个节点");

    // 8. 已移除的节点 Get_Peer 应返回 PeerNotFound
    let result = handle.Get_Peer(&peer_ids[19]).await;
    assert!(
        result.is_err(),
        "已移除的节点 Get_Peer 应返回错误"
    );
}

/// TC-02: 心跳与超时清理
///
/// add 5 → sleep → 仅 Update_Heartbeat 3 个 → Cleanup_Timeout_Peers(1) → 清除 2 → count = 3
#[tokio::test]
async fn tc02_heartbeat_and_timeout_cleanup() {
    let (_manager, handle) = create_peer_management(PeerId::random());
    let mut peer_ids: Vec<PeerId> = Vec::new();

    // 1. add 5 个节点
    for _ in 0..5 {
        let peer_id = PeerId::random();
        peer_ids.push(peer_id);
        handle.Add_Peer(make_peer(peer_id)).await.unwrap();
    }
    assert_eq!(handle.Count().await.unwrap(), 5);

    // 2. 等待足够时间使所有节点"超时"（> 1 秒）
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    // 3. 仅对前 3 个更新心跳（重置 last_active）
    for i in 0..3 {
        handle
            .Update_Heartbeat(&peer_ids[i], Some(10))
            .await
            .unwrap();
    }

    // 4. Cleanup_Timeout_Peers(1) → 应清除 elapsed >= 1s 的节点（后 2 个）
    let removed = handle.Cleanup_Timeout_Peers(1).await.unwrap();
    assert_eq!(removed, 2, "应清除 2 个超时节点");

    // 5. count = 3
    let count = handle.Count().await.unwrap();
    assert_eq!(count, 3, "清理后应剩 3 个节点");

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
/// spawn 10 任务同时 add/update/list → 无 panic，最终 count 一致
#[tokio::test]
async fn tc03_concurrent_safety() {
    // 使用 PeerHandle 直接构造，因为需要 Clone + Arc 来在多任务间共享
    let manager = Arc::new(PeerManager::new(PeerId::random()));
    let handle = PeerHandle::new(manager.clone());
    let handle = Arc::new(handle);
    let task_count = 10usize;
    let peers_per_task = 5usize;

    // 每个任务添加 5 个节点，同时穿插 Update_Status 和 List_Peers 操作
    let mut join_handles = Vec::new();
    for _task_id in 0..task_count {
        let h = Arc::clone(&handle);
        let jh = tokio::spawn(async move {
            let mut task_peers = Vec::new();
            for _ in 0..peers_per_task {
                let peer_id = PeerId::random();
                task_peers.push(peer_id);
                h.Add_Peer(make_peer(peer_id)).await.unwrap();
            }
            // 穿插读操作
            let _list = h.List_Peers().await.unwrap();
            let _count = h.Count().await.unwrap();

            // 穿插 Update_Status
            if !task_peers.is_empty() {
                h.Update_Status(&task_peers[0], PeerStatus::Busy)
                    .await
                    .unwrap();
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

    // 最终 count 应等于所有任务添加的总和
    let expected_total = task_count * peers_per_task;
    let final_count = handle.Count().await.unwrap();
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

/// TC-04: 能力与带宽更新
///
/// add peer → Update_Capability → Get_Peer → 验证 capability 字段
/// → Update_Bandwidth → 验证
#[tokio::test]
async fn tc04_capability_and_bandwidth_update() {
    let (_manager, handle) = create_peer_management(PeerId::random());
    let peer_id = PeerId::random();
    handle.Add_Peer(make_peer(peer_id)).await.unwrap();

    // 1. 初始状态：capability = None, bandwidth = None
    let info = handle.Get_Peer(&peer_id).await.unwrap();
    assert!(info.capability.is_none(), "初始 capability 应为 None");
    assert!(info.bandwidth_mbps.is_none(), "初始 bandwidth 应为 None");

    // 2. Update_Capability
    let cap = PeerCapability::with_gpu(8192, 95.5);
    handle
        .Update_Capability(&peer_id, Some(cap.clone()))
        .await
        .unwrap();

    let info = handle.Get_Peer(&peer_id).await.unwrap();
    let actual_cap = info.capability.unwrap();
    assert!(actual_cap.has_gpu, "GPU 标志应为 true");
    assert_eq!(actual_cap.memory_mb, 8192, "内存应为 8192 MB");
    assert!((actual_cap.compute_score - 95.5).abs() < f32::EPSILON, "计算评分应为 95.5");

    // 3. Update_Bandwidth
    handle
        .Update_Bandwidth(&peer_id, Some(1000))
        .await
        .unwrap();

    let info = handle.Get_Peer(&peer_id).await.unwrap();
    assert_eq!(
        info.bandwidth_mbps,
        Some(1000),
        "带宽应为 1000 Mbps"
    );

    // 4. 更新心跳并验证延迟
    handle
        .Update_Heartbeat(&peer_id, Some(25))
        .await
        .unwrap();

    let info = handle.Get_Peer(&peer_id).await.unwrap();
    assert_eq!(info.latency_ms, Some(25), "延迟应为 25ms");
}

/// TC-05: 空管理器操作
///
/// 空 manager → Get_Peer = Err → list = empty → Remove_Peer = Err → Count = 0
#[tokio::test]
async fn tc05_empty_manager_operations() {
    let (_manager, handle) = create_peer_management(PeerId::random());
    let fake_peer_id = PeerId::random();

    // 1. Count = 0 (替代 is_empty)
    assert_eq!(handle.Count().await.unwrap(), 0, "空管理器 count 应为 0");

    // 2. list = empty
    let peers = handle.List_Peers().await.unwrap();
    assert!(peers.is_empty(), "空管理器 list 应返回空 Vec");

    // 3. Get_Peer = Err(PeerNotFound)
    let result = handle.Get_Peer(&fake_peer_id).await;
    assert!(result.is_err(), "空管理器 Get_Peer 应返回错误");
    match result.unwrap_err() {
        Peer_Management_Error::PeerNotFound(_) => {
            // 正确的错误类型
        }
        other => panic!("期望 PeerNotFound，实际: {:?}", other),
    }

    // 4. Remove_Peer = Err(PeerNotFound)
    let result = handle.Remove_Peer(&fake_peer_id).await;
    assert!(result.is_err(), "空管理器 Remove_Peer 应返回错误");

    // 5. Get_Idle_Peers = empty
    let idle = handle.Get_Idle_Peers().await.unwrap();
    assert!(idle.is_empty(), "空管理器 Get_Idle_Peers 应返回空 Vec");

    // 6. Cleanup_Timeout_Peers = 0 removed
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
    handle_1.Add_Peer(make_peer(peer_a)).await.unwrap();

    // 2. handle_2 能立即看到 peer_a
    assert!(
        handle_2.Contains_Peer(&peer_a).await.unwrap(),
        "另一个 handle 应看到第一个 handle 添加的节点"
    );

    // 3. handle_2 添加 peer_b
    handle_2.Add_Peer(make_peer(peer_b)).await.unwrap();

    // 4. handle_1 能看到 peer_b
    assert!(
        handle_1.Contains_Peer(&peer_b).await.unwrap(),
        "第一个 handle 应看到第二个 handle 添加的节点"
    );

    // 5. 两个 handle 的 count 一致
    let count_1 = handle_1.Count().await.unwrap();
    let count_2 = handle_2.Count().await.unwrap();
    assert_eq!(count_1, 2, "handle_1 count 应为 2");
    assert_eq!(count_2, 2, "handle_2 count 应为 2");

    // 6. handle_1 更新状态，handle_2 能看到
    handle_1
        .Update_Status(&peer_a, PeerStatus::Busy)
        .await
        .unwrap();

    let info_from_h2 = handle_2.Get_Peer(&peer_a).await.unwrap();
    assert_eq!(
        info_from_h2.status,
        PeerStatus::Busy,
        "handle_2 应看到 handle_1 的状态更新"
    );

    // 7. handle_2 删除 peer_b，handle_1 同步感知
    handle_2.Remove_Peer(&peer_b).await.unwrap();
    assert_eq!(
        handle_1.Count().await.unwrap(),
        1,
        "删除后 handle_1 count 应为 1"
    );
    assert!(
        !handle_1.Contains_Peer(&peer_b).await.unwrap(),
        "handle_2 删除的节点在 handle_1 中也不应存在"
    );
}
