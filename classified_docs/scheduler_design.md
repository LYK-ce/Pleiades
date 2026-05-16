# Scheduler 最优分配方案

## 1. 背景

当前 Scheduler 采用**均匀分配**策略：
- `total_gguf_layers / (1 + K)` 层给每个节点，余数扔给 Coordinator
- 设备选择仅看 `has_gpu` 布尔值
- 完全未使用 `compute_score`, `layer_time`, `memory_mb`, `bandwidth_mbps`, `latency_ms`

Profile 模块已采集并存储了每个节点的性能数据：

| 指标 | 存储字段 | 采集方式 |
|------|----------|----------|
| 单层推理耗时 | `PeerCapability.layer_time[model_id]` | 跑5层warmup再计时1层推理 |
| 剩余内存 | `PeerCapability.memory_mb` | sysinfo/CUDA NVML（加载模型前） |
| Ping延迟 | `PeerInfo.latency_ms` | libp2p Ping协议（每60s） |
| 带宽 | `PeerInfo.bandwidth_mbps` | 1M/10M/50M 发包测试取最大 |

这些数据已可用，但 Scheduler 完全未利用。

## 2. 目标

将 Scheduler 从均匀分配升级为**性能感知的加权分配**：

1. **内存约束优先** — 根据 `memory_mb` + 运行时内存模型（量化权重存储 + KV Cache + 激活值），确定每个节点最大可承载层数（硬上限）
2. **综合成本加权** — 每节点有效成本 = `layer_time` + 通信固定开销平摊（带宽/延迟）。成本越低的节点分越多层，自然实现算力+带宽+延迟的联合优化
3. **合理拓扑排列** — Worker 按综合有效成本升序排列，快的节点在前段

## 3. 前置改动：Model_Info 扩展

### 3.1 问题

`Model_Info` 当前仅包含架构元数据（`architecture`, `num_layers` 等），**缺少每层内存大小信息**。Scheduler 无法判断每个节点能装多少层。

`GGUF_Analyze()` 在 `gguf_model_manager.rs:285` 已计算出每层 `total_size_bytes`，但在 `session.rs:185-193` 转换为 `Model_Info` 时被丢弃。

### 3.2 方案

给 `Model_Info` 添加以下字段：

```rust
pub struct Model_Info {
    // 已有字段不变
    pub architecture: String,
    pub num_layers: usize,
    pub embedding_length: usize,
    pub has_input_head: bool,
    pub has_output_head: bool,
    pub has_tokenizer: bool,
    pub eos_token_id: u32,

    // 新增：内存预算相关
    /// 每层 GGUF 文件中的 tensor 字节数（量化存储大小）
    /// 索引规则（GGUF 层编号）：
    ///   layer_sizes[0]         = embedding 层 (token_embd.weight)
    ///   layer_sizes[1..=N]     = transformer blocks (blk.0 ~ blk.N-1)
    ///   layer_sizes[N+1]       = output 层 (output_norm + output)
    /// 数组长度 = num_layers + 2
    pub layer_sizes_bytes: Vec<usize>,

    // 新增：KV Cache 计算所需参数
    /// KV attention head 数量（通常 <= head_count，GQA/MQA 优化后更小）
    pub num_kv_heads: usize,
    /// 每个 attention head 的维度
    pub head_dim: usize,
    /// 最大上下文长度（暂不用于 KV Cache 估算，保留供后续使用）
    pub context_length: usize,
}
```

### 3.3 运行时内存模型

**GGUF 核心设计原则：磁盘多大，内存就多大。** `.gguf` 文件中的量化权重加载到内存后保持 QTensor 量化格式（`QMatMul` 在推理时按需反量化），不会整体膨胀到 F32。因此 `layer_sizes_bytes` 基本就是权重运行时的内存占用。

存在一个例外：embedding 层在加载时显式反量化（`gguf_model.rs:163`），因为 `candle_nn::Embedding` 要求 F32 Tensor。反量化后该层的运行时内存 = `vocab_size × embedding_length × sizeof(f32)`。

单节点加载 `[start, end]` 层后的运行时内存占用 = 以下三项之和：

#### (a) 权重内存（量化存储）

transformer block 权重保持 QTensor 量化格式，运行时内存 ≈ GGUF 文件中的存储大小。

```
weight_memory = Σ layer_sizes_bytes[l]     (l ∈ [start, end])
```

注意：若包含 embedding 层（start == 0），该层需额外计算反量化后大小替换 `layer_sizes_bytes[0]`。

#### (b) KV Cache

```
kv_cache_bytes = 2                    // K + V
               × num_local_layers     // 本节点负责的 transformer block 层数
               × num_kv_heads         // KV head 数
               × head_dim             // 每个 head 维度
               × 1024                 // 固定预估 context 长度
               × sizeof(f32)          // 4 bytes
```

对于分布式流水线，每个节点只维护自己负责的层的 KV Cache。
注意：KV Cache 随 token 位置线性增长，预估 context 长度固定取 1024。

#### (c) 激活值 + 其他开销

包括 RotaryEmbedding 预计算、中间激活张量、RmsNorm 参数等。简化处理为固定比例：

```
activation_overhead = weight_memory × ACTIVATION_OVERHEAD_RATIO（默认 0.15）
```

#### 总公式

```
total_runtime_memory(start, end) =
    weight_memory(start..=end)
  + kv_cache_bytes
  + activation_overhead
```

### 3.4 数据来源

`Model_Arch_Info` 中已有所有必须数据：
- `layers[i].total_size_bytes` → `layer_sizes_bytes[i+1]`
- `non_layer_tensors` → `layer_sizes_bytes[0]` (embedding) 和 `layer_sizes_bytes[N+1]` (output)
- `head_count_kv` → `num_kv_heads`
- `head_dim` → `head_dim`
- `context_length` → `context_length`

需要修改：
- `session.rs:185-193` — 从 `Model_Arch_Info` 读出上述字段，组装 `Model_Info`
- `service.rs` — `Analyze_Model` 同样填充

### 3.5 修改清单

| 文件 | 改动 |
|------|------|
| `Src/ML_Engine/pipeline.rs` | `Model_Info` 新增 `layer_sizes_bytes`, `num_kv_heads`, `head_dim`, `context_length` |
| `Src/ML_Engine/session.rs` | `Session_Thread` 中从 `gguf_model.arch_info` 填充新字段 |
| `Src/ML_Engine/service.rs` | `Analyze_Model` 中填充新字段 |
| `Src/Scheduler/capability.rs` | 无需改动 |
| 测试代码 | `make_model_info` 增加新字段 |

## 4. Scheduler 分配算法

### 4.1 完整调度流程

#### Step 0 — 节点过滤

**规则：节点必须有当前模型的 Profile 数据，否则直接丢弃。**

从 `available_peers` 中筛选：
- `capability.layer_time` 必须包含 `model_id`（当前模型）
- `capability.memory_mb` 必须 > 0
- Coordinator 自身也同样检查

这不是"降权"，是硬性排除——没有性能数据的节点无法参与分配。

#### Step 1 — 内存容量检查（硬约束）

对每个通过筛选的节点（含 Coordinator），计算最大可装层数。

##### 运行时内存模型

给定范围 `[start, end]`，运行时内存（全部计算见 7 节）：
```
runtime_memory(start, end) = weight_memory + kv_cache_bytes + activation_overhead
```

其中：
- `weight_memory = Σ layer_sizes_bytes[l]`（GGUF 设计：磁盘多大内存多大，量化权重不膨胀）
- `kv_cache_bytes = 2 × num_transformer_blocks × num_kv_heads × head_dim × 1024 × 4`
  - 只有 transformer block 层（1..N）需要 KV Cache，embedding/output 层不需要
- `activation_overhead = weight_memory × 0.15`

##### 查找最大层数

从 `layer_start` 开始，累加 `[layer_start, layer_end]` 的运行时内存，线性扫描找到不超过 `memory_mb[i]` 的最大 `layer_end`：

```
max_layers[i] = max_layer_end（满足 runtime_memory(start, end) ≤ memory_mb[i] × 1024 × 1024）
```

若最大装载量 < 1 层，排除该节点。

**起始层差异**：
- Coordinator：从 layer 0（embedding）开始
- Worker：从 layer 1（第一个 transformer block）开始

#### Step 2 — 综合成本加权分配（软约束）

##### 每节点通信固定开销

先按某种初始排列估算每节点的通信成本（排列未定时可用默认顺序先算，后续 Step 3 确定拓扑后再精细排列）。

隐状态张量 `hidden_bytes = embedding_length × 4`。

**对 Coordinator**（流水线前段，出站给 W1）：
```
comm_fixed[Coord] = hidden_bytes / min(bw[Coord], bw[W1]) × 8e-6 + lat_est(Coord, W1)
```
Coordinator 的入站来自流水线末端的回传，不产生额外前向延迟（回传与下一 token 的前向可并行），故简化只计出站开销。

**对中间 Worker**（入站 + 出站）：
```
comm_fixed[Wi] = recv_cost(上游, Wi) + send_cost(Wi, 下游)
```

**对末尾 Worker**（入站 + 回传 Coordinator）：
```
comm_fixed[Wk] = recv_cost(上游, Wk) + send_cost(Wk, Coord)
```

##### 有效成本与权重

```
estimated_layers = total_layers / node_count           // 初始估计
effective_cost[i] = layer_time[i] + comm_fixed[i] / estimated_layers
weight[i] = 1.0 / effective_cost[i]
```

**这是自平衡的**：
- Coordinator 性能差 → `layer_time` 大 → `effective_cost` 高 → `weight` 低 → **少分** ✓
- Coordinator 性能好 → `layer_time` 小 → `effective_cost` 低 → `weight` 高 → **多分** ✓
- Worker 低带宽/高延迟 → `comm_fixed` 大 → `weight` 低 → 少分 ✓

##### 分配层数

```
allocated_layers[i] = min(
    floor(total_layers × weight[i] / sum(weights)),
    max_layers[i]
)
```

余数按权重降序逐一分配，直到分完或所有节点达到内存上限。

#### Step 3 — 拓扑排列与最终 layer range

按 `effective_cost` 升序排列 Worker，确定最终拓扑：

```
Coordinator → W(cost最小) → W(次小) → ... → W(cost最大) → Coordinator
```

基于排列顺序和各节点分到的层数，计算最终的 GGUF 层范围：

| 节点 | layer_start | layer_end |
|------|-------------|-----------|
| Coordinator | 0 | allocated[Coord] - 1 |
| W1 | allocated[Coord] | allocated[Coord] + allocated[W1] - 1 |
| W2 | ... | ... |

末尾 Worker 若分到 output 层（layer_end == N+1），则其包含 `output_norm + lm_head`。

注：这是简化的一维切分策略。更精确的方案（如中间某层切给不同节点）需要模型分片支持，不在当前范围内。

### 4.2 退化情况

| 情况 | 行为 |
|------|------|
| 无可用 Worker | 单机退化（Coordinator 检查自身内存是否够全模型） |
| 某节点没有 `layer_time[model_id]` | Step 0 直接丢弃，不参与分配 |
| 某节点 bandwidth/latency 未知 | `comm_fixed` 中的对应项取 0（不惩罚） |
| 某节点 memory_mb 不足以装任何层（max_layers < 1） | 排除该节点 |
| 所有节点总内存 < 全模型运行时内存 | 报错 `Insufficient_Memory`，附带各节点可用内存明细 |
| Coordinator 自身内存不足以装全模型 | 报错 `Coordinator_Memory_Insufficient` |
| 所有节点被过滤掉 | 报错 `No_Eligible_Peers` |

### 4.3 device 选择

`device` 字段仍保留（模型加载必须指定设备），从 `capability.has_gpu` 读取的方式不变。
`has_gpu` 不再作为分配决策依据——`layer_time` 已反映真实算力。

## 5. 数据结构变更

### 5.1 Scheduler_Input（不变）

`scheduler_handler.rs` 的调用方式不变，但从 `PeerInfo.query_profile()` 已在 `Scheduler_Input.available_peers` 中包含所有性能数据：
- `capability.layer_time[model_id]`
- `capability.memory_mb`
- `latency_ms`
- `bandwidth_mbps`

### 5.2 Worker_Assignment（不变）

现有字段已够用：`layer_start`, `layer_end`, `peer_id`, `outbound_target`, `device`。

### 5.3 Scheduler_Error（扩展）

```rust
pub enum Scheduler_Error {
    Zero_Layers,
    /// 所有节点（含 Coordinator）内存不足以装下全模型
    Insufficient_Memory {
        total_required_bytes: u64,
        available: Vec<(PeerId, u64)>, // (节点ID, 可用字节数)
    },
    /// Coordinator 自身内存不足（单机退化失败）
    Coordinator_Memory_Insufficient {
        required_bytes: u64,
        available_bytes: u64,
    },
    /// 所有节点被排除（内存均不足 1 层）
    No_Eligible_Peers,
}
```

## 6. 实施步骤

### Step 1 — Model_Info 扩展
1. `pipeline.rs` — `Model_Info` 添加 `layer_sizes_bytes`, `num_kv_heads`, `head_dim`, `context_length`
2. `session.rs` — `Session_Thread` 中从 `arch_info` 填充新字段
3. `service.rs` — `Analyze_Model` 填充新字段
4. 测试适配

### Step 2 — Scheduler 加权分配
5. `service.rs` — 实现运行时内存估算函数 `Estimate_Runtime_Memory()`
6. `service.rs` — 实现内存约束查找 `Find_Max_Layers()`
7. `service.rs` — 实现通信开销计算函数 `Compute_Comm_Fixed_Cost()`
8. `service.rs` — 实现综合成本公式 `Effective_Cost(layer_time, comm_fixed_cost)`
9. `service.rs` — `Plan_Pipeline` 替换为：内存约束 + 综合成本加权 + 环拓扑排列
10. `capability.rs` — `Scheduler_Error` 新增 `Insufficient_Memory`, `Coordinator_Memory_Insufficient`, `No_Eligible_Peers`
11. 单元测试

### Step 3 — 拓扑优化
12. `service.rs` — Workers 按 `effective_cost` 升序排列（算力+通信综合）

### Step 4 — 验证
13. `cargo test --lib scheduler` 验证
14. `cargo test` 全量回归

## 7. 运行时内存估算器详细设计

### 7.1 接口

```rust
/// 估算加载 [start, end] 层（GGUF 编号）所需的运行时内存字节数
fn Estimate_Runtime_Memory(
    model_info: &Model_Info,
    start: usize,
    end: usize,
) -> u64
```

### 7.2 计算逻辑

```rust
fn Estimate_Runtime_Memory(model_info: &Model_Info, start: usize, end: usize) -> u64 {
    // (a) 权重内存（量化存储，GGUF 设计：磁盘多大内存多大）
    let mut weight_bytes: u64 = 0;
    for l in start..=end {
        if l < model_info.layer_sizes_bytes.len() {
            weight_bytes += model_info.layer_sizes_bytes[l] as u64;
        }
    }

    // (b) KV Cache
    //     仅 transformer blocks 需要 KV Cache（layer 1..=num_layers）
    let block_start = start.max(1);
    let block_end = end.min(model_info.num_layers);
    let num_blocks = if block_start <= block_end { block_end - block_start + 1 } else { 0 };
    let kv_cache_bytes = 2u64
        * num_blocks as u64
        * model_info.num_kv_heads as u64
        * model_info.head_dim as u64
        * 1024u64  // 固定预估 context 长度
        * 4u64; // sizeof(f32)

    // (c) 激活值开销（权重内存的 15%）
    let activation_overhead = (weight_bytes as f64 * ACTIVATION_OVERHEAD_RATIO) as u64;

    weight_bytes + kv_cache_bytes + activation_overhead
}
```

### 7.3 常量

| 常量 | 值 | 说明 |
|------|-----|------|
| `ACTIVATION_OVERHEAD_RATIO` | 0.15 | 激活值/中间张量相对于权重内存的比例 |

### 7.4 线性查找最大层数

```rust
/// 从 layer_start 开始，找到满足内存约束的最大 layer_end
fn Find_Max_Layers(
    model_info: &Model_Info,
    memory_mb: u64,
    layer_start: usize,  // 起始层（含）
) -> usize  // 最大结束层（含），若一层都装不下则返回 < layer_start
```

由于 `Estimate_Runtime_Memory` 单调递增，可用二分查找快速定位。但实际层数很少（通常 20-50 层），线性扫描也完全可行。优先采用线性扫描，简单可靠。

## 8. 不影响的部分

| 组件 | 说明 |
|------|------|
| `PeerManagement` 全部 | 数据模型不变 |
| `Profiler` | 数据采集不变 |
| `scheduler_handler.rs` | 调用接口不变 |
| `Scheduler_Input` / `Scheduler_Capability` trait | 接口不变 |
| `Orchestrator_VM` | 指令不变 |
| `Network` | 不变 |
| `TUI` | 不变 |
| `gguf_model_manager.rs` | 不变（数据已在 `Model_Arch_Info` 中） |
| `gguf_model.rs` | 不变 |
