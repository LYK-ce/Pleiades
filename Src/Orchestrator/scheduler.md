# Scheduler 模块设计

## 概述

Scheduler 是一个**纯计算函数模块**（无状态、无副作用），负责：

给定**模型信息** + **可用节点列表** → 输出**分布式拓扑 plan**。

模块位置：`Src/Orchestrator/scheduler.rs`

## 数据结构

### 输入

```rust
/// Scheduler 输入（由 PlanPipeline handler 组装）
pub struct Scheduler_Input {
    /// 模型分析结果（层数、架构等）— 来自 AnalyzeModel 指令
    pub model_info: Model_Info,
    /// 可用节点列表（空闲 + 已连接的）— 来自 PeerManager.Get_Idle_Peers()
    pub available_peers: Vec<PeerInfo>,
    /// Coordinator 自己的 PeerId — 来自 Network.get_local_peer_id()
    pub local_peer_id: PeerId,
    /// 全局唯一推理 ID — 由 Core 在 route_user 中预生成
    pub inference_id: u64,
}
```

### 输出

```rust
/// Pipeline 拓扑规划（完整 plan）
#[derive(Debug, Clone)]
pub struct Pipeline_Plan {
    /// 全局唯一推理 ID
    pub inference_id: u64,
    /// Coordinator 负责的层范围（起始，含）
    pub coord_layer_start: usize,
    /// Coordinator 负责的层范围（结束，不含）
    pub coord_layer_end: usize,
    /// Worker 分配列表（按流水线顺序排列）
    pub workers: Vec<Worker_Assignment>,
}

/// 单个 Worker 的分配信息
#[derive(Debug, Clone)]
pub struct Worker_Assignment {
    /// Worker 节点 ID
    pub peer_id: PeerId,
    /// 负责的层范围（起始，含）
    pub layer_start: usize,
    /// 负责的层范围（结束，不含）
    pub layer_end: usize,
    /// 出站目标节点（该 Worker 需要向其打开张量流）
    pub outbound_target: PeerId,
    /// 模型分片 file_id（如果已分发；None 表示需要先分发）
    pub model_shard_id: Option<String>,
    /// 设备偏好
    pub device: String,
}

/// Scheduler 错误
#[derive(Debug, thiserror::Error)]
pub enum Scheduler_Error {
    #[error("模型层数为 0")]
    ZeroLayers,
    #[error("无可用节点且不允许单机退化")]
    NoPeersAvailable,
}
```

## 核心函数

```rust
/// 纯计算函数：根据输入规划 Pipeline 拓扑
///
/// - 无 async
/// - 无 trait object
/// - 输入确定 → 输出确定
/// - 易于单元测试
pub fn Plan_Pipeline(input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error>
```

## 分层策略

### 初版：均匀分配

```text
总节点数 = 1 (Coordinator) + K (Workers)
每节点分配 N / (K+1) 层
余数全给 Coordinator（它通常有最好的设备）

拓扑为环形：
  Coordinator → Worker_1 → Worker_2 → ... → Worker_K → Coordinator
```

示例（32 层模型，2 个 Worker）：
```text
Coordinator: 层 0..12  (10 + 2余数)
Worker_1:    层 12..22
Worker_2:    层 22..32
流向: Coord→W1→W2→Coord
```

### 退化情况

| 可用 Worker 数 | 行为 |
|---------------|------|
| 0 | 单机退化：全部层由 Coordinator 执行，无 EstablishStreams / JoinWorkers |
| 1 | 两节点流水线：Coordinator 前半 → Worker 后半 → 回传 |
| N | 标准环形流水线 |

### 未来扩展：计算感知分配

```rust
/// 按 compute_score 加权分配层数
fn weighted_allocation(
    total_layers: usize,
    scores: &[f32],  // 各节点的 compute_score
) -> Vec<usize>     // 各节点分配的层数
```

考虑因素：
- `PeerCapability.compute_score` → 计算能力强的节点分更多层
- `PeerCapability.memory_mb` → 内存决定单节点可承载的最大层数
- `PeerInfo.bandwidth_mbps` → 带宽影响拓扑排列（高带宽节点相邻）
- `PeerInfo.latency_ms` → 延迟影响整体吞吐量

## 实现伪代码

```rust
pub fn Plan_Pipeline(input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error> {
    let total_layers = input.model_info.num_layers;
    if total_layers == 0 {
        return Err(Scheduler_Error::ZeroLayers);
    }

    let workers = &input.available_peers;

    // 无可用 Worker → 退化为单机
    if workers.is_empty() {
        return Ok(Pipeline_Plan {
            inference_id: input.inference_id,
            coord_layer_start: 0,
            coord_layer_end: total_layers,
            workers: vec![],
        });
    }

    let node_count = workers.len() + 1; // +1 for Coordinator
    let layers_per_node = total_layers / node_count;
    let remainder = total_layers % node_count;

    // Coordinator 取前段 + 余数
    let coord_end = layers_per_node + remainder;

    // Workers 按顺序分配后续层
    let mut assignments = Vec::new();
    let mut layer_cursor = coord_end;

    for (i, peer) in workers.iter().enumerate() {
        let start = layer_cursor;
        let end = start + layers_per_node;

        // 下一跳：最后一个 Worker 出站回 Coordinator
        let outbound_target = if i + 1 < workers.len() {
            workers[i + 1].peer_id
        } else {
            input.local_peer_id
        };

        let device = if peer.capability.as_ref().map_or(false, |c| c.has_gpu) {
            "cuda".to_string()
        } else {
            "cpu".to_string()
        };

        assignments.push(Worker_Assignment {
            peer_id: peer.peer_id,
            layer_start: start,
            layer_end: end,
            outbound_target,
            model_shard_id: None, // 后续由模型分发逻辑填充
            device,
        });

        layer_cursor = end;
    }

    Ok(Pipeline_Plan {
        inference_id: input.inference_id,
        coord_layer_start: 0,
        coord_layer_end: coord_end,
        workers: assignments,
    })
}
```

## 与 TaskInstruction 的集成

### PlanPipeline 指令

```rust
// instruction.rs 新增
PlanPipeline {
    /// 输入：模型分析结果槽位
    model_info: SlotId,
    /// 输入：inference_id 槽位
    inference_id: SlotId,
    /// 输出：Pipeline_Plan 写入此槽位
    result: SlotId,
}
```

### compile_pipeline 生成的完整指令序列

```text
正向序列：
  Const(model_path) → SLOT_MODEL
  Const(inference_id) → SLOT_INFERENCE_ID
  AnalyzeModel { model: SLOT_MODEL, result: SLOT_MODEL_INFO }
  PlanPipeline { model_info: SLOT_MODEL_INFO, inference_id: SLOT_INFERENCE_ID, result: SLOT_PLAN }
  EstablishStreams { plan: SLOT_PLAN, result: SLOT_STREAMS_RESULT }
  JoinWorkers { plan: SLOT_PLAN, result: SLOT_WORKERS_RESULT }
  Const("coordinator") → SLOT_ML_PROGRAM_MODE
  Const(device) → SLOT_DEVICE
  CreateSession { model, device, start(从plan), end(从plan), io, tensor_io, result }
  RunProgram { session, result }

补偿序列：
  TeardownPipeline { plan: SLOT_PLAN }
  ShutdownSession { session: SLOT_SESSION }
```

### PlanPipeline handler 实现

```rust
// handler_network.rs 或新增 handler_scheduler.rs
pub(super) async fn handle_plan_pipeline(
    &mut self,
    model_info_slot: SlotId,
    inference_id_slot: SlotId,
    result: SlotId,
) -> StepResult {
    // 1. 从槽位读取 Model_Info
    let model_info = match self.slots.take_model_info(model_info_slot) {
        Ok(info) => info,
        Err(e) => return StepResult::Abort(format!("PlanPipeline: model_info error: {}", e)),
    };

    // 2. 从槽位读取 inference_id
    let inference_id = match self.slots.get_u64(inference_id_slot) {
        Ok(id) => id,
        Err(e) => return StepResult::Abort(format!("PlanPipeline: inference_id error: {}", e)),
    };

    // 3. 查询可用节点
    let available_peers = match self.capabilities.peer_manager.Get_Idle_Peers().await {
        Ok(peers) => peers,
        Err(e) => return StepResult::Abort(format!("PlanPipeline: get peers error: {}", e)),
    };

    // 4. 获取本地 PeerId
    let local_peer_id = self.capabilities.network.get_local_peer_id();

    // 5. 调用 Scheduler 纯函数
    let input = Scheduler_Input {
        model_info,
        available_peers,
        local_peer_id,
        inference_id,
    };
    let plan = match Plan_Pipeline(input) {
        Ok(p) => p,
        Err(e) => return StepResult::Abort(format!("PlanPipeline: scheduler error: {}", e)),
    };

    // 6. 写入结果槽位
    self.slots.set(result, SlotValue::PipelinePlan(plan));
    StepResult::Continue
}
```

## SlotValue 扩展

```rust
// slot.rs 新增
SlotValue::PipelinePlan(Pipeline_Plan),
```

需要为 `SlotFile` 添加便捷访问方法：

```rust
impl SlotFile {
    pub fn get_pipeline_plan(&self, slot: SlotId) -> Result<&Pipeline_Plan, String> { ... }
    pub fn take_pipeline_plan(&mut self, slot: SlotId) -> Result<Pipeline_Plan, String> { ... }
}
```

## 代码变更清单

| # | 文件 | 变更 |
|---|------|------|
| 1 | `Src/Orchestrator/scheduler.rs` (新增) | `Pipeline_Plan`, `Worker_Assignment`, `Scheduler_Input`, `Scheduler_Error`, `Plan_Pipeline` |
| 2 | `Src/Orchestrator/mod.rs` | `pub mod scheduler;` |
| 3 | `Src/Orchestrator/slot.rs` | 新增 `SlotValue::PipelinePlan(Pipeline_Plan)` + 访问方法 |
| 4 | `Src/Orchestrator/instruction.rs` | 新增 `PlanPipeline { model_info, inference_id, result }` |
| 5 | `Src/Orchestrator/executor/task_engine.rs` | `step()` 新增 `PlanPipeline` 分支 |
| 6 | `Src/Orchestrator/executor/handler_network.rs` | 新增 `handle_plan_pipeline` |
| 7 | `Src/Orchestrator/compiler.rs` | `compile_pipeline` 修改：在 `EstablishStreams` 前插入 `AnalyzeModel + PlanPipeline` |

## 设计原则

1. **Scheduler 是纯函数**：不需要 async，不依赖 trait object，输入确定输出确定，易于单元测试
2. **Handler 负责数据获取**：`handle_plan_pipeline` 从 PeerManager 和 Network 获取运行时数据，然后调用纯函数
3. **模型分发是前置步骤**：用户先执行 `distribute` 命令分发模型分片，再执行 `pipeline` 启动推理；`model_shard_id` 的填充逻辑后续通过 Storage 查询实现
4. **层分配算法可插拔**：初版均分，后续可按节点能力加权

## 测试策略

由于 `Plan_Pipeline` 是纯函数，测试极为简单：

```rust
#[test]
fn test_plan_2_workers_32_layers() {
    let input = Scheduler_Input { /* 构造 */ };
    let plan = Plan_Pipeline(input).unwrap();
    assert_eq!(plan.coord_layer_end, 12);      // 10 + 2余数
    assert_eq!(plan.workers[0].layer_start, 12);
    assert_eq!(plan.workers[0].layer_end, 22);
    assert_eq!(plan.workers[1].layer_start, 22);
    assert_eq!(plan.workers[1].layer_end, 32);
}

#[test]
fn test_plan_no_workers_degrades_to_single() {
    let input = Scheduler_Input { available_peers: vec![], .. };
    let plan = Plan_Pipeline(input).unwrap();
    assert_eq!(plan.workers.len(), 0);
    assert_eq!(plan.coord_layer_end, total_layers);
}
```
