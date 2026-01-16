#Presented by KeJi
#Date: 2026-01-14

# Rust 管理层实现方案

## 一、功能需求

| 功能 | 描述 |
|------|------|
| 被动模式 | 后台挂起，监听命令，根据命令执行操作 |
| 主动模式 | 接收用户指令，选择节点，分发具体命令 |
| 命令收发 | 通过P2P网络收发命令 |
| 节点选择 | 根据规则选择合适的执行节点 |

## 二、依赖分析

### 2.1 已有依赖（复用）

| 库 | 用途 | 来源 |
|----|------|------|
| `tokio` | 异步运行时 | libp2p依赖 |
| `serde` + `serde_json` | 命令序列化 | libp2p依赖 |
| `libp2p` | P2P通信 | 传输层 |

### 2.2 需要新增的外部库

**结论：无需新增外部库**

原因：
1. **事件循环** - `tokio::select!` 可处理多事件源
2. **状态管理** - Rust enum + match 足够
3. **命令模式** - 自定义 Command/Response 枚举
4. **节点选择** - 自行实现算法（基于DHT信息）

## 三、架构设计

### 3.1 核心结构

```rust
// 节点角色
pub enum NodeRole {
    Passive,  // 被动节点：等待命令
    Active,   // 主动节点：发起任务
}

// 命令定义
#[derive(Serialize, Deserialize)]
pub enum ManagerCommand {
    // 模型相关
    LoadModel { model_id: String, layers: Range<usize> },
    Inference { request_id: u64, input: Vec<u8> },
    
    // 节点发现
    QueryCapabilities,
    ReportCapabilities(NodeCapabilities),
    
    // 任务协调
    AssignTask { task_id: u64, subtask: Subtask },
    TaskResult { task_id: u64, result: Vec<u8> },
}

// 节点能力
#[derive(Serialize, Deserialize)]
pub struct NodeCapabilities {
    pub memory_mb: u64,
    pub has_gpu: bool,
    pub loaded_models: Vec<String>,
}
```

### 3.2 事件驱动主循环

```rust
pub async fn run_manager(
    mut swarm: Swarm<MyBehaviour>,
    mut user_rx: mpsc::Receiver<UserInput>,
    role: NodeRole,
) {
    loop {
        tokio::select! {
            // P2P事件
            event = swarm.select_next_some() => {
                handle_swarm_event(event).await;
            }
            // 用户输入（仅Active模式）
            Some(input) = user_rx.recv(), if matches!(role, NodeRole::Active) => {
                handle_user_input(&mut swarm, input).await;
            }
        }
    }
}
```

### 3.3 节点选择算法

```rust
pub struct NodeSelector {
    known_nodes: HashMap<PeerId, NodeCapabilities>,
}

impl NodeSelector {
    /// 选择执行推理的节点
    pub fn select_for_inference(
        &self,
        model_id: &str,
        layer_count: usize,
    ) -> Vec<(PeerId, Range<usize>)> {
        let mut candidates: Vec<_> = self.known_nodes
            .iter()
            .filter(|(_, cap)| cap.has_gpu || cap.memory_mb > 4096)
            .collect();
        
        // 按内存排序，贪心分配层
        candidates.sort_by_key(|(_, cap)| std::cmp::Reverse(cap.memory_mb));
        
        let mut assignments = Vec::new();
        let mut assigned = 0;
        let layers_per_node = layer_count / candidates.len().max(1);
        
        for (peer_id, _) in candidates {
            let start = assigned;
            let end = (assigned + layers_per_node).min(layer_count);
            assignments.push((*peer_id, start..end));
            assigned = end;
            if assigned >= layer_count { break; }
        }
        
        assignments
    }
}
```

## 四、模块划分

```
src/
├── manager/
│   ├── mod.rs          // 模块导出
│   ├── command.rs      // 命令定义
│   ├── handler.rs      // 命令处理器
│   ├── selector.rs     // 节点选择
│   └── state.rs        // 状态管理
```

## 五、与其他层的交互

```
┌─────────────┐
│  用户接口   │  (CLI/API)
└──────┬──────┘
       │ UserInput
┌──────▼──────┐
│  管理层     │  ManagerCommand
└──────┬──────┘
       │ Request/Response
┌──────▼──────┐
│  传输层     │  libp2p
└──────┬──────┘
       │ Inference
┌──────▼──────┐
│  运行时层   │  ort (ONNX)
└─────────────┘
```

## 六、总结

| 项目 | 结论 |
|------|------|
| 外部库 | **无需新增**，复用tokio/serde/libp2p |
| 实现方式 | Rust标准库 + 异步事件驱动 |
| 核心模式 | enum命令 + tokio::select! 事件循环 |
| 节点选择 | 自实现贪心算法（可扩展） |
