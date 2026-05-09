//Presented by KeJi
//Date ： 2026-04-29

//! Network 集成测试 — C 组
//!
//! 验证双节点真实网络通信和 DataType 入站预筛选。
//! 全部标记 `#[ignore]`，仅手动执行：
//!
//! ```bash
//! cargo test --test t07_network_integration -- --ignored --nocapture
//! ```
//!
//! ## 测试基础设施
//!
//! 每个测试启动两个 `Network_Service` 实例（各持独立 Keypair + PeerManager），
//! 通过 `handle.Dial()` 手动连接（避免 mDNS 发现延迟），监听随机端口。
//!
//! ## DataType 分流规则
//!
//! | DataType      | 处理方式                     |
//! |---------------|------------------------------|
//! | BandwidthTest | Network 内部直接回复         |
//! | Data          | Network 内部直接回复 OK      |
//! | Info          | Network 内部直接回复 OK      |
//! | Command       | 转发给 Orchestrator Core B5  |
//! | File          | 转发给 Orchestrator Core B5  |

#![allow(non_snake_case)]

use std::sync::Arc;
use std::time::Duration;

use libp2p::identity::Keypair;
use libp2p::{Multiaddr, PeerId};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use pleiades::network::{
    DataType, InboundRequest, Network_Inbound_Event, Network_Service,
    NetworkConfig, NodeHandle,
};
use pleiades::peer_management::{create_peer_management, PeerManager};
use pleiades::event_bus::EventBus;

// =========================================================================
// 辅助函数
// =========================================================================

/// 获取一个可用的 TCP 端口
///
/// 通过 bind 到 `127.0.0.1:0` 让 OS 分配端口，读取后释放。
/// 存在微小竞争窗口，在本地测试场景下可接受。
fn Get_Available_Port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// 启动一个轻量级 Network 节点
///
/// 返回元组：
/// - `NodeHandle`: 对外 API 句柄
/// - `PeerId`: 本地节点 ID
/// - `Receiver<InboundRequest>`: 入站请求接收端（仅 Command/File）
/// - `Receiver<Network_Inbound_Event>`: 复杂入站事件接收端（文件流/张量流）
/// - `Multiaddr`: 监听地址（用于对端 Dial）
/// - `JoinHandle<()>`: 后台任务句柄
/// - `Arc<PeerManager>`: 底层 PeerManager（用于验证节点管理操作）
async fn Spawn_Node() -> (
    NodeHandle,
    PeerId,
    mpsc::Receiver<InboundRequest>,
    mpsc::Receiver<Network_Inbound_Event>,
    Multiaddr,
    JoinHandle<()>,
    Arc<PeerManager>,
) {
    let port = Get_Available_Port();
    let keypair = Keypair::generate_ed25519();
    let peer_id = PeerId::from(keypair.public());
    let config = NetworkConfig {
        listen_port: port,
        ..Default::default()
    };
    let (manager, peer_capability) = create_peer_management();
    let event_bus = Arc::new(EventBus::New(1024));

    let (mut service, handle, inbound_rx, _capability, event_rx) =
        Network_Service::Init(config, keypair, peer_capability, event_bus)
            .await
            .expect("Network_Service::Init failed");

    let addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/{}", port)
        .parse()
        .expect("Failed to parse Multiaddr");

    let join = tokio::spawn(async move {
        if let Err(e) = service.Start().await {
            eprintln!("Network_Service error: {}", e);
        }
    });

    // 等待服务启动并开始监听
    tokio::time::sleep(Duration::from_millis(500)).await;

    (handle, peer_id, inbound_rx, event_rx, addr, join, manager)
}

/// 连接两个节点：A dial B，等待连接建立
async fn Connect_Nodes(handle_a: &NodeHandle, addr_b: &Multiaddr) {
    handle_a
        .Dial(addr_b.clone())
        .await
        .expect("Dial failed");
    // 等待连接建立（TCP 握手 + libp2p noise/yamux 协商）
    tokio::time::sleep(Duration::from_secs(1)).await;
}

/// 停止节点并等待后台任务结束
async fn Stop_Node(handle: &NodeHandle, join: JoinHandle<()>) {
    let _ = handle.Stop().await;
    let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
}

/// 验证 `inbound_rx` 为空（无消息转发给 Core）
///
/// 等待 200ms，如果收到消息则断言失败。
async fn Assert_Inbound_Empty(inbound_rx: &mut mpsc::Receiver<InboundRequest>) {
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        inbound_rx.recv(),
    ).await;
    assert!(
        result.is_err(),
        "Expected inbound_rx to be empty, but received a message"
    );
}

// =========================================================================
// TC-01: BandwidthTest 分流 — Network 内部回复
// =========================================================================

/// A→B 发送 `DataType::BandwidthTest`（size=1000）→ B 直接回复 1000 字节包
/// → 验证 B 的 `inbound_rx` 为空（未转发给 Core）
#[tokio::test]
#[ignore]
async fn tc_01_bandwidth_test_shunt_normal() {
    let (handle_a, _peer_a, mut _inbound_rx_a, _event_rx_a, _addr_a, join_a, _mgr_a) =
        Spawn_Node().await;
    let (handle_b, peer_b, mut inbound_rx_b, _event_rx_b, addr_b, join_b, _mgr_b) =
        Spawn_Node().await;

    // A → B 连接
    Connect_Nodes(&handle_a, &addr_b).await;

    // A 发送 BandwidthTest(size=1000)
    let size: u64 = 1000;
    let payload = size.to_le_bytes().to_vec();
    let response = handle_a
        .Send_Data(&peer_b, DataType::BandwidthTest, payload)
        .await
        .expect("Send_Data failed");

    // 验证响应：类型为 BandwidthTest，长度 = 1000
    assert_eq!(response.data_type, DataType::BandwidthTest);
    assert_eq!(
        response.payload.len(),
        1000,
        "Response payload should be 1000 bytes, got {}",
        response.payload.len()
    );

    // 验证 B 的 inbound_rx 为空（BandwidthTest 不转发给 Core）
    Assert_Inbound_Empty(&mut inbound_rx_b).await;

    // 清理
    Stop_Node(&handle_a, join_a).await;
    Stop_Node(&handle_b, join_b).await;
}

// =========================================================================
// TC-02: BandwidthTest 50MB 上限防护
// =========================================================================

/// A→B 发送 `BandwidthTest`（size=100_000_000）→ B 回复包大小 = 50_000_000
/// （截断到上限）→ 验证 `inbound_rx` 为空
#[tokio::test]
#[ignore]
async fn tc_02_bandwidth_test_cap_50mb() {
    let (handle_a, _peer_a, mut _inbound_rx_a, _event_rx_a, _addr_a, join_a, _mgr_a) =
        Spawn_Node().await;
    let (handle_b, peer_b, mut inbound_rx_b, _event_rx_b, addr_b, join_b, _mgr_b) =
        Spawn_Node().await;

    Connect_Nodes(&handle_a, &addr_b).await;

    // 请求 100MB（超过 50MB 上限）
    let size: u64 = 100_000_000;
    let payload = size.to_le_bytes().to_vec();
    let response = handle_a
        .Send_Data(&peer_b, DataType::BandwidthTest, payload)
        .await
        .expect("Send_Data failed");

    // 验证响应被截断到 50MB
    assert_eq!(response.data_type, DataType::BandwidthTest);
    assert_eq!(
        response.payload.len(),
        50_000_000,
        "Response should be capped at 50MB, got {} bytes",
        response.payload.len()
    );

    Assert_Inbound_Empty(&mut inbound_rx_b).await;

    Stop_Node(&handle_a, join_a).await;
    Stop_Node(&handle_b, join_b).await;
}

// =========================================================================
// TC-03: BandwidthTest payload 不足 8 字节
// =========================================================================

/// A→B 发送 `BandwidthTest`（payload=3字节）→ B 回复 8 字节包
/// → 验证 `inbound_rx` 为空
#[tokio::test]
#[ignore]
async fn tc_03_bandwidth_test_short_payload() {
    let (handle_a, _peer_a, mut _inbound_rx_a, _event_rx_a, _addr_a, join_a, _mgr_a) =
        Spawn_Node().await;
    let (handle_b, peer_b, mut inbound_rx_b, _event_rx_b, addr_b, join_b, _mgr_b) =
        Spawn_Node().await;

    Connect_Nodes(&handle_a, &addr_b).await;

    // 发送不足 8 字节的 payload（无法解析为 u64 size）
    let payload = vec![0u8; 3];
    let response = handle_a
        .Send_Data(&peer_b, DataType::BandwidthTest, payload)
        .await
        .expect("Send_Data failed");

    // 验证响应为默认 8 字节（短 payload 回退值）
    assert_eq!(response.data_type, DataType::BandwidthTest);
    assert_eq!(
        response.payload.len(),
        8,
        "Response should be 8 bytes for short payload, got {}",
        response.payload.len()
    );

    Assert_Inbound_Empty(&mut inbound_rx_b).await;

    Stop_Node(&handle_a, join_a).await;
    Stop_Node(&handle_b, join_b).await;
}

// =========================================================================
// TC-04: Data 分流 — Network 内部回复 OK
// =========================================================================

/// A→B 发送 `DataType::Data` → B 直接回复 `DataType::Data` + `b"OK"`
/// → 验证 `inbound_rx` 为空
#[tokio::test]
#[ignore]
async fn tc_04_data_shunt_ok() {
    let (handle_a, _peer_a, mut _inbound_rx_a, _event_rx_a, _addr_a, join_a, _mgr_a) =
        Spawn_Node().await;
    let (handle_b, peer_b, mut inbound_rx_b, _event_rx_b, addr_b, join_b, _mgr_b) =
        Spawn_Node().await;

    Connect_Nodes(&handle_a, &addr_b).await;

    let response = handle_a
        .Send_Data(&peer_b, DataType::Data, b"test_data".to_vec())
        .await
        .expect("Send_Data failed");

    // 验证 Network 内部直接回复 Data + "OK"
    assert_eq!(response.data_type, DataType::Data);
    assert_eq!(response.payload, b"OK");

    // 验证 B 的 inbound_rx 为空（Data 不转发给 Core）
    Assert_Inbound_Empty(&mut inbound_rx_b).await;

    Stop_Node(&handle_a, join_a).await;
    Stop_Node(&handle_b, join_b).await;
}

// =========================================================================
// TC-05: Info 分流 — Network 内部回复 OK
// =========================================================================

/// A→B 发送 `DataType::Info` → B 直接回复 `DataType::Info` + `b"OK"`
/// → 验证 `inbound_rx` 为空
#[tokio::test]
#[ignore]
async fn tc_05_info_shunt_ok() {
    let (handle_a, _peer_a, mut _inbound_rx_a, _event_rx_a, _addr_a, join_a, _mgr_a) =
        Spawn_Node().await;
    let (handle_b, peer_b, mut inbound_rx_b, _event_rx_b, addr_b, join_b, _mgr_b) =
        Spawn_Node().await;

    Connect_Nodes(&handle_a, &addr_b).await;

    let response = handle_a
        .Send_Data(&peer_b, DataType::Info, b"node_info".to_vec())
        .await
        .expect("Send_Data failed");

    // 验证 Network 内部直接回复 Info + "OK"
    assert_eq!(response.data_type, DataType::Info);
    assert_eq!(response.payload, b"OK");

    // 验证 B 的 inbound_rx 为空（Info 不转发给 Core）
    Assert_Inbound_Empty(&mut inbound_rx_b).await;

    Stop_Node(&handle_a, join_a).await;
    Stop_Node(&handle_b, join_b).await;
}

// =========================================================================
// TC-06: Command 转发到 inbound_rx
// =========================================================================

/// A→B 发送 `DataType::Command`（payload=`b"TEST_CMD"`）→ 在 B 的 `inbound_rx`
/// 接收到 `InboundRequest`（验证 `data_type=Command`, `payload` 一致, `peer=A`）
/// → B 调用 `Send_Response` 回复 → A 收到 response
#[tokio::test]
#[ignore]
async fn tc_06_command_forwarded() {
    let (handle_a, peer_a, mut _inbound_rx_a, _event_rx_a, _addr_a, join_a, _mgr_a) =
        Spawn_Node().await;
    let (handle_b, peer_b, mut inbound_rx_b, _event_rx_b, addr_b, join_b, _mgr_b) =
        Spawn_Node().await;

    Connect_Nodes(&handle_a, &addr_b).await;

    // B 端：spawn 任务接收入站请求并回复
    let handle_b_clone = handle_b.clone();
    let responder = tokio::spawn(async move {
        let req = tokio::time::timeout(
            Duration::from_secs(5),
            inbound_rx_b.recv(),
        )
            .await
            .expect("Timeout waiting for inbound Command request")
            .expect("inbound_rx closed unexpectedly");

        // 验证请求字段
        assert_eq!(req.data_type, DataType::Command, "Expected Command type");
        assert_eq!(req.payload, b"TEST_CMD", "Payload mismatch");
        assert_eq!(req.peer, peer_a, "Peer ID should be node A");

        // 回复
        handle_b_clone
            .Send_Response(req.request_id, DataType::Command, b"REPLY_OK".to_vec())
            .await
            .expect("Send_Response failed");
    });

    // A 端：发送 Command 并等待响应
    let response = handle_a
        .Send_Data(&peer_b, DataType::Command, b"TEST_CMD".to_vec())
        .await
        .expect("Send_Data failed");

    assert_eq!(response.data_type, DataType::Command);
    assert_eq!(response.payload, b"REPLY_OK");

    responder.await.expect("Responder task panicked");

    Stop_Node(&handle_a, join_a).await;
    Stop_Node(&handle_b, join_b).await;
}

// =========================================================================
// TC-07: File 转发到 inbound_rx
// =========================================================================

/// A→B 发送 `DataType::File`（payload=`b"model.gguf|1024|abc123"`）→ 在 B 的
/// `inbound_rx` 接收到 `InboundRequest`（验证 `data_type=File`）
/// → B 调用 `Send_Response(Command, b"ACCEPT")` → A 收到 ACCEPT
#[tokio::test]
#[ignore]
async fn tc_07_file_forwarded() {
    let (handle_a, peer_a, mut _inbound_rx_a, _event_rx_a, _addr_a, join_a, _mgr_a) =
        Spawn_Node().await;
    let (handle_b, peer_b, mut inbound_rx_b, _event_rx_b, addr_b, join_b, _mgr_b) =
        Spawn_Node().await;

    Connect_Nodes(&handle_a, &addr_b).await;

    let file_metadata = b"model.gguf|1024|abc123".to_vec();

    // B 端：spawn 任务接收入站请求并回复 ACCEPT
    let handle_b_clone = handle_b.clone();
    let responder = tokio::spawn(async move {
        let req = tokio::time::timeout(
            Duration::from_secs(5),
            inbound_rx_b.recv(),
        )
            .await
            .expect("Timeout waiting for inbound File request")
            .expect("inbound_rx closed unexpectedly");

        // 验证请求字段
        assert_eq!(req.data_type, DataType::File, "Expected File type");
        assert_eq!(req.payload, b"model.gguf|1024|abc123", "File metadata mismatch");
        assert_eq!(req.peer, peer_a, "Peer ID should be node A");

        // 回复 ACCEPT
        handle_b_clone
            .Send_Response(req.request_id, DataType::Command, b"ACCEPT".to_vec())
            .await
            .expect("Send_Response failed");
    });

    // A 端：发送 File 并等待响应
    let response = handle_a
        .Send_Data(&peer_b, DataType::File, file_metadata)
        .await
        .expect("Send_Data failed");

    assert_eq!(response.payload, b"ACCEPT");

    responder.await.expect("Responder task panicked");

    Stop_Node(&handle_a, join_a).await;
    Stop_Node(&handle_b, join_b).await;
}

// =========================================================================
// TC-08: PeerManagement 联动
// =========================================================================

/// 两个节点 Dial 建立连接 → 验证 `Add_Peer` 被调用（对端存在于 PeerManager）
/// → `Disconnect` → 验证 `Remove_Peer` 被调用（对端不再存在于 PeerManager）
///
/// 注意：mDNS 可能导致其他并行测试的节点自动连入，因此
/// 使用 **特定 PeerId 查询** 而非全局 count 验证，避免跨测试干扰。
#[tokio::test]
#[ignore]
async fn tc_08_peer_management_integration() {
    let (handle_a, peer_a, _inbound_rx_a, _event_rx_a, _addr_a, join_a, mgr_a) =
        Spawn_Node().await;
    let (handle_b, peer_b, _inbound_rx_b, _event_rx_b, addr_b, join_b, mgr_b) =
        Spawn_Node().await;

    // A → B 连接（mDNS 可能已发现其他节点，Dial 仍确保双方有对端记录）
    Connect_Nodes(&handle_a, &addr_b).await;

    // 验证连接后 PeerManager 包含特定对端
    // ConnectionEstablished 事件触发 Add_Peer
    assert!(
        mgr_a.get_peer(&peer_b).await.is_some(),
        "A: PeerManager should contain peer B after connection"
    );
    assert!(
        mgr_b.get_peer(&peer_a).await.is_some(),
        "B: PeerManager should contain peer A after connection"
    );

    // A 断开与 B 的连接
    // Disconnect 命令：swarm.disconnect_peer_id + peer_handle.Remove_Peer
    // ConnectionClosed 事件：B 端也触发 peer_handle.Remove_Peer
    handle_a.Disconnect(&peer_b).await.expect("Disconnect failed");
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 验证断开后 PeerManager 不再包含对端
    assert!(
        mgr_a.get_peer(&peer_b).await.is_none(),
        "A: PeerManager should NOT contain peer B after disconnect"
    );
    assert!(
        mgr_b.get_peer(&peer_a).await.is_none(),
        "B: PeerManager should NOT contain peer A after disconnect"
    );

    Stop_Node(&handle_a, join_a).await;
    Stop_Node(&handle_b, join_b).await;
}
