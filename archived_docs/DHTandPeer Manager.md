# PeerManager 模块设计文档

## 概述

PeerManager 是一个独立的节点信息管理模块，用于管理Pleiades网络中已连接节点的信息。该模块使用读写锁（RwLock）提供并发安全的访问，供网络层和Control层调用。

## 设计目标

1. **解耦**：将节点信息管理从Network Service中分离出来
2. **并发安全**：支持多读单写的并发访问模式
3. **高性能**：减少消息传递延迟，提高访问性能
4. **可扩展**：为未来功能（节点能力发现、负载均衡）提供基础

## 文件结构

```
Src/PeerManagement/
├── mod.rs          # 模块导出和工具函数
├── peer_info.rs    # 节点信息数据结构定义
├── peer_manager.rs # 核心管理组件（读写锁实现）
└── peer_handle.rs  # 对外调用接口（类似C头文件作用）
```

## 各文件功能详述

### 1. `peer_info.rs` - 节点信息数据结构

#### 主要类型定义

```rust
/// 节点状态枚举
pub enum PeerStatus {
    Connected,     // 已连接，空闲
    Busy,          // 已连接，忙碌（执行推理任务）
    Connecting,    // 连接建立中
    Disconnected,  // 已断开连接
}

/// 节点能力描述
pub struct PeerCapability {
    pub has_gpu: bool,           // 是否有GPU
    pub memory_mb: u64,          // 内存大小（MB）
    pub compute_score: f32,      // 计算能力评分
    pub supported_models: Vec<String>, // 支持的模型类型
}

/// 节点详细信息
pub struct PeerInfo {
    pub peer_id: PeerId,                 // 节点ID
    pub addresses: Vec<Multiaddr>,       // 地址列表
    pub latency_ms: Option<u64>,         // 最后一次ping延迟
    pub connected_at: Instant,           // 连接建立时间
    pub last_active: Instant,            // 最后活跃时间
    pub status: PeerStatus,              // 节点状态
    pub capability: Option<PeerCapability>, // 节点能力（可选）
    pub metadata: HashMap<String, String>, // 元数据（可扩展）
}
```

#### 主要功能
- 定义节点相关的数据结构
- 提供数据验证和辅助方法
- 支持序列化/反序列化（用于网络传输）
- 超时检查等工具函数

### 2. `peer_manager.rs` - 核心管理组件

#### 主要类型定义

```rust
/// 节点管理器（使用读写锁保护）
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>, // 核心存储
    cleanup_interval: Duration, // 清理间隔
}
```

#### 核心功能

**CRUD操作：**
- `upsert_peer()` - 添加或更新节点信息
- `remove_peer()` - 移除节点
- `get_peer()` - 获取单个节点信息
- `get_all_peers()` - 获取所有节点信息

**状态管理：**
- `update_status()` - 更新节点状态
- `update_latency()` - 更新节点延迟
- `update_active()` - 更新活跃时间

**查询功能：**
- `get_idle_peers()` - 获取空闲节点列表
- `get_busy_peers()` - 获取忙碌节点列表
- `count()` - 获取节点数量
- `is_empty()` - 检查是否为空

**维护功能：**
- `start_cleanup_task()` - 启动后台清理任务
- `cleanup_timeout_peers()` - 清理超时节点

#### 并发设计
- 使用 `tokio::sync::RwLock` 支持异步上下文
- 读多写少的访问模式优化
- 自动清理超时节点，防止内存泄漏

### 3. `peer_handle.rs` - 对外调用接口

#### 主要类型定义

```rust
/// 节点管理句柄（类似C头文件接口）
#[derive(Clone)]
pub struct PeerHandle {
    inner: Arc<PeerManager>, // 内部管理器引用
}

/// 节点事件（用于通知）
pub enum PeerEvent {
    PeerAdded(PeerInfo),
    PeerRemoved(PeerId),
    PeerStatusChanged(PeerId, PeerStatus),
    PeerLatencyUpdated(PeerId, u64),
}

/// 节点管理错误
pub enum PeerError {
    PeerNotFound(PeerId),
    Timeout,
    Internal(String),
}
```

#### 接口功能

**基本操作：**
- `add_peer()` - 添加节点
- `remove_peer()` - 移除节点
- `list_peers()` - 列出所有节点
- `get_peer()` - 获取节点信息

**状态管理：**
- `update_status()` - 更新节点状态
- `get_idle_peers()` - 获取空闲节点
- `get_best_peer()` - 获取最佳节点（基于延迟和能力）

**事件通知：**
- 可选的事件通知机制
- 支持TUI实时更新节点状态

#### 设计特点
- 统一的错误处理（`Result<T, PeerError>`）
- 线程安全的共享（`Clone` + `Send` + `Sync`）
- 可选的性能监控和缓存层

### 4. `mod.rs` - 模块导出

#### 主要功能

```rust
// 导出所有公共类型
pub use peer_info::{PeerInfo, PeerStatus, PeerCapability};
pub use peer_manager::PeerManager;
pub use peer_handle::{PeerHandle, PeerEvent, PeerError};

// 工具函数
pub fn create_peer_management() -> (Arc<PeerManager>, PeerHandle) {
    let manager = Arc::new(PeerManager::new());
    let handle = PeerHandle::new(manager.clone());
    manager.start_cleanup_task();
    (manager, handle)
}
```

## 使用示例

### 初始化

```rust
use crate::peer_management::{create_peer_management, PeerHandle};

// 创建管理器和句柄
let (peer_manager, peer_handle) = create_peer_management();

// 或者单独创建
let peer_manager = Arc::new(PeerManager::new());
let peer_handle = PeerHandle::new(peer_manager.clone());
```

### Network层使用

```rust
// 连接建立时
async fn handle_connection_established(peer_handle: &PeerHandle, peer_id: PeerId, addr: Multiaddr) {
    let peer_info = PeerInfo::new(peer_id, vec![addr]);
    peer_handle.add_peer(peer_info).await.unwrap();
}

// 连接断开时
async fn handle_connection_closed(peer_handle: &PeerHandle, peer_id: &PeerId) {
    peer_handle.remove_peer(peer_id).await.unwrap();
}
```

### Control层使用

```rust
// 查询节点列表进行模型分配
async fn distribute_model_layers(peer_handle: &PeerHandle) -> Result<(), Box<dyn Error>> {
    let peers = peer_handle.list_peers().await?;
    let total_devices = peers.len() + 1; // 包含自己
    
    // 计算分配方案
    for (i, peer) in peers.iter().enumerate() {
        // 分配逻辑...
    }
    Ok(())
}

// 获取最佳节点
async fn find_best_peer(peer_handle: &PeerHandle) -> Option<PeerInfo> {
    peer_handle.get_best_peer().await.ok().flatten()
}
```

## 与现有架构的集成

### 替换Network Service中的节点管理

**当前：**
```rust
// network_service.rs
pub struct Network_Service {
    connected_peers: HashMap<PeerId, PeerInfo>, // 需要替换
    // ...
}
```

**新架构：**
```rust
// network_service.rs
pub struct Network_Service {
    peer_handle: PeerHandle, // 替换为PeerHandle
    // ...
}

impl Network_Service {
    pub async fn handle_swarm_event(&mut self, event: SwarmEvent) {
        match event {
            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                let peer_info = PeerInfo::new(peer_id, vec![endpoint.get_remote_address().clone()]);
                self.peer_handle.add_peer(peer_info).await.ok();
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                self.peer_handle.remove_peer(&peer_id).await.ok();
            }
            // ...
        }
    }
}
```

### 更新NodeHandle接口

**保持向后兼容：**
```rust
// node_handle.rs
impl NodeHandle {
    // 旧方式：通过消息传递
    pub async fn Get_Peers(&self) -> Result<Vec<PeerId>, Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::GetPeers { reply: tx }).await?;
        Ok(rx.await?)
    }
    
    // 新方式：直接访问（高性能）
    pub async fn Get_Peers_Direct(&self) -> Result<Vec<PeerId>, Box<dyn Error + Send + Sync>> {
        let peers = self.peer_handle.list_peers().await?;
        Ok(peers.into_iter().map(|p| p.peer_id).collect())
    }
}
```

## 性能考虑

### 读写锁性能
- **读操作**：多个线程可以同时读取，无阻塞
- **写操作**：独占访问，会阻塞其他读写操作
- **优化**：读多写少的场景下性能良好

### 内存使用
- `PeerInfo` 结构体大小约 200-300 字节
- 假设 1000 个节点，内存占用约 200-300 KB
- `Arc` 引用计数有额外开销，但可接受

### 异步兼容性
- 使用 `tokio::sync::RwLock` 而非 `std::sync::RwLock`
- 支持在异步上下文中使用
- 避免阻塞事件循环

## 迁移策略

### 第一阶段：实现独立模块
1. 创建 `Src/PeerManagement/` 目录和文件
2. 实现基本功能，编写单元测试
3. 不修改现有代码，独立测试

### 第二阶段：并行运行
1. 在 `Network_Service` 中同时维护新旧两种方式
2. 添加日志对比数据一致性
3. 验证新模块功能正确性

### 第三阶段：逐步切换
1. 修改 `Control` 层使用新接口
2. 更新 `TUI` 显示使用新数据源
3. 逐步替换所有使用节点信息的地方

### 第四阶段：清理旧代码
1. 移除 `Network_Service` 中的 `connected_peers` 字段
2. 删除旧的 `GetPeers` 消息处理逻辑
3. 清理不再使用的代码

## 扩展性考虑

### 未来功能扩展
1. **节点能力发现**：自动探测节点硬件能力
2. **负载均衡**：基于节点负载动态分配任务
3. **健康检查**：定期ping节点检测连通性
4. **地理位置**：基于延迟优化节点选择
5. **权限管理**：节点访问控制和认证

### 接口扩展点
1. `PeerHandle` 可以添加缓存层提高性能
2. 可以添加监控指标（查询次数、平均延迟等）
3. 支持插件式的能力发现机制
4. 可配置的清理策略和超时设置

## 总结

PeerManager 模块通过读写锁实现了并发安全的节点信息管理，解决了当前架构中节点信息管理与网络服务紧耦合的问题。该设计提供了高性能的直接访问接口，同时保持了良好的扩展性，为Pleiades项目的分布式推理场景提供了坚实的基础。

关键优势：
1. **解耦**：单一职责，提高代码可维护性
2. **性能**：减少消息传递延迟，支持高并发读取
3. **安全**：线程安全的并发访问
4. **扩展**：为未来功能提供良好基础