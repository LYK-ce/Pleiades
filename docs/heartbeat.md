#Presented by KeJi
#Date ： 2026-04-16

# 心跳功能实现方案

## 概述

本方案使用 libp2p 内置的 ping 协议实现节点心跳检测和延迟测量。ping 协议是 libp2p 的标准协议，提供自动的往返时间（RTT）计算和连接保持功能。

## 协议特性

### 协议标识符
- `/ipfs/ping/1.0.0`

### 工作原理
```
客户端发送: Ping { seq: u32 }
服务器响应: Pong { seq: u32 }
```

### 自动功能
1. **RTT 计算**：libp2p 自动测量往返时间
2. **连接保持**：定期发送 ping 保持连接活跃
3. **协议协商**：自动协商 ping 协议支持
4. **超时检测**：内置超时机制

## 实现步骤

### 1. 修改依赖配置

在 `Cargo.toml` 中为 libp2p 添加 `"ping"` feature：

```toml
libp2p = { version = "0.56", features = [
    "tokio",
    "tcp", 
    "noise",
    "yamux",
    "mdns",
    "kad", 
    "request-response",
    "macros",
    "ping"  # 新增
]}
```

### 2. 更新 NetworkBehaviour

在 [`network_service.rs`](Src/Network/network_service.rs) 中修改 `PleiadesNetworkBehaviour`：

```rust
use libp2p::ping;

#[derive(NetworkBehaviour)]
pub struct PleiadesNetworkBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub request_response: request_response::Behaviour<PleiadesCodec>,
    pub stream: stream::Behaviour,
    pub ping: ping::Behaviour,  // 新增
}
```

### 3. 配置 Ping 参数

在 `Network_Service` 初始化时配置 ping：

```rust
let ping_config = ping::Config::new()
    .with_interval(Duration::from_secs(30))  // 每30秒发送一次
    .with_timeout(Duration::from_secs(10))   // 10秒超时
    .with_keep_alive(true);                  // 保持连接活跃

let ping_behaviour = ping::Behaviour::new(ping_config);
```

### 4. 处理 Ping 事件

在 `Network_Service` 的事件循环中添加 ping 事件处理：

```rust
match swarm_event {
    SwarmEvent::Behaviour(PleiadesNetworkBehaviourEvent::Ping(ping_event)) => {
        match ping_event.result {
            Ok(ping::Success::Pong { rtt, .. }) => {
                // 成功收到 Pong，更新延迟
                let latency_ms = rtt.as_millis() as u64;
                if let Some(peer_id) = ping_event.peer {
                    peer_manager.update_heartbeat(&peer_id, Some(latency_ms)).await;
                    
                    // 发送 UI 事件
                    event_sender.send(NetworkEvent::PeerHeartbeatUpdated {
                        peer_id: peer_id.to_string(),
                        latency_ms,
                    }).ok();
                }
            }
            Err(ping::Failure::Timeout) => {
                // Ping 超时
                if let Some(peer_id) = ping_event.peer {
                    peer_manager.update_status(&peer_id, PeerStatus::Disconnected).await;
                    
                    // 发送 UI 事件
                    event_sender.send(NetworkEvent::PeerTimeout {
                        peer_id: peer_id.to_string(),
                    }).ok();
                }
            }
            Err(ping::Failure::Unsupported) => {
                // 对方不支持 ping 协议
                debug!("Peer does not support ping protocol");
            }
            _ => {}
        }
    }
}
```

### 5. 更新 NetworkEvent 枚举

在 [`network_service.rs`](Src/Network/network_service.rs) 的 `NetworkEvent` 中添加新事件：

```rust
pub enum NetworkEvent {
    // ... 现有事件
    PeerHeartbeatUpdated {
        peer_id: String,
        latency_ms: u64,
    },
    PeerTimeout {
        peer_id: String,
    },
}
```

## 配置参数

### 心跳间隔
- **推荐值**: 30 秒
- **范围**: 15-60 秒
- **考虑因素**: 
  - 局域网可设置较短间隔（15-30秒）
  - 广域网应设置较长间隔（30-60秒）

### 超时时间
- **推荐值**: 心跳间隔的 2-3 倍
- **示例**: 30秒间隔 → 60-90秒超时
- **容错机制**: 连续 3 次超时才标记为断开

### 延迟测量
- **精度**: 毫秒级
- **用途**: 
  - 节点选择（优先选择低延迟节点）
  - 网络质量监控
  - 故障诊断

## 与现有系统集成

### 1. PeerManager 集成
- 使用现有的 [`update_heartbeat()`](Src/PeerManagement/peer_manager.rs:68) 方法
- 更新 `last_active` 时间戳
- 存储 `latency_ms` 用于节点选择

### 2. Control 层集成
- 通过 `NetworkEvent` 接收心跳事件
- 更新节点状态
- 触发重连机制（如需要）

### 3. TUI 显示
- 在 Network 面板显示节点延迟
- 使用颜色标识连接质量：
  - 绿色: < 50ms
  - 黄色: 50-200ms  
  - 红色: > 200ms 或超时

## 故障处理

### 1. 临时网络波动
- **策略**: 允许偶尔超时，不立即断开
- **实现**: 使用滑动窗口计数（如 3/5 次失败才断开）

### 2. 协议不支持
- **处理**: 记录日志，不视为错误
- **回退**: 可考虑使用自定义心跳作为备选

### 3. 连接恢复
- **机制**: 超时后尝试重新建立连接
- **间隔**: 指数退避重试（1s, 2s, 4s, 8s...）



## 监控指标

### 1. 关键指标
- 心跳成功率
- 平均延迟
- 超时率
- 连接保持时间

### 2. 告警规则
- 连续 3 次心跳失败
- 平均延迟 > 500ms
- 心跳成功率 < 90%

## 扩展性考虑

### 1. 未来扩展
- 添加自定义心跳包携带能力信息
- 支持不同网络环境的心跳策略
- 集成更复杂的健康检查

### 2. 配置化
- 支持运行时调整心跳参数
- 不同节点类型使用不同策略
- 基于网络条件的自适应调整

## 总结

使用 libp2p ping 协议实现心跳功能具有以下优势：

1. **标准化**: 使用业界标准协议
2. **简单**: 无需实现复杂的协议逻辑
3. **高效**: libp2p 优化了协议实现
4. **可靠**: 经过广泛测试和验证

此方案能够满足基本的节点存活检测和延迟测量需求，为后续的负载均衡和故障转移提供基础数据。