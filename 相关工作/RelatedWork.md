#Presented by KeJi
#Date ： 2026-01-15

# 相关工作

## Petals: 协作式大模型推理与微调系统

**论文**:
- PETALS: Collaborative Inference and Fine-tuning of Large Models (EMNLP 2023)
- Distributed Inference and Fine-tuning of Large Language Models Over The Internet (NeurIPS 2023)

**项目地址**: https://petals.dev

**实现**: Python + PyTorch + hivemind库

### 核心问题

100B+规模LLM（如BLOOM-176B）需要350GB+显存，普通研究者难以使用。现有方案的局限：
- **RAM/SSD Offloading**: 延迟过高（BLOOM-176B单token生成需5.5-22秒）
- **云端API**: 不灵活，无法访问内部状态，成本高

### 核心架构：Client-Server模式

```
┌─────────────────────────────────────────────────────────┐
│                        Clients                          │
│  (持有embeddings层 + 可训练参数，约3%模型参数)            │
└───────────────────────┬─────────────────────────────────┘
                        │ hidden states
        ┌───────────────┼───────────────┐
        ▼               ▼               ▼
┌───────────────┐ ┌───────────────┐ ┌───────────────┐
│   Server 1    │ │   Server 2    │ │   Server 3    │
│ Blocks 0-22   │→│ Blocks 23-46  │→│ Blocks 47-69  │
│ (GPU + Cache) │ │ (GPU + Cache) │ │ (GPU + Cache) │
└───────────────┘ └───────────────┘ └───────────────┘
```

**关键设计原则**:
- Server持有**连续的**Transformer blocks
- Client通过pipeline连接多个Server完成完整推理
- Server存储attention KV cache，支持自回归生成

### 核心技术贡献

#### 1. 容错推理算法（NeurIPS 2023核心贡献）

**双缓存策略**:
| 缓存类型 | 存储位置 | 内容 | 用途 |
|----------|----------|------|------|
| Server-side cache | 服务器GPU | Attention KV | 正常推理 |
| Client-side cache | 客户端 | 每个pipeline stage的输入 | 故障恢复 |

**故障恢复流程**:
1. Server失败时，Client检测到连接断开
2. Client从已知Server中选择替代节点
3. 将cached inputs发送给新Server重建KV cache
4. 继续推理，无需重启

**通信复杂度**:
- 正常情况: O(n·t)，n为层数，t为序列长度
- 单Server失败: 额外O(t)数据传输（单轮）

#### 2. 动态路由与负载均衡

**Client路由策略**:
- 使用D*Lite算法寻找最优Server链
- 优化目标：最小化总推理时间
- 考虑因素：计算时间 + 网络延迟

**Server负载均衡**:
```
新Server选择blocks策略：
start = argmin_{i=1}^{L-K+1} sorted([t_i, t_{i+1}, ..., t_{i+K-1}])

- L: 模型总层数
- K: 该Server可持有的blocks数
- t_i: 第i个block的当前总吞吐量
```

- 新Server优先覆盖吞吐量最低的blocks（消除瓶颈）
- 定期检查是否需要rebalancing（阈值p=20%）
- 通过DHT（Kademlia）发布Server信息

#### 3. 量化优化

| 技术 | 效果 |
|------|------|
| **8-bit权重量化** | 内存减半，BLOOM从44节点降至22节点 |
| **动态分块量化** | hidden states传输带宽减半 |
| **4-bit NormalFloat** | Llama 2支持 |

8-bit量化对质量影响极小（HellaSwag/LAMBADA/WinoGrande平均<0.5%下降）

#### 4. Parameter-efficient Fine-tuning

**核心原则**: Client持有可训练参数，Server只执行forward/backward

```python
# Prompt tuning示例
model = AutoModelForSequenceClassification.from_pretrained(
    "bigscience/bloom-petals",
    tuning_mode="ptune", pre_seq_len=5)
    
opt = torch.optim.AdamW(model.parameters())
for input_ids, labels in data_loader:
    out = model.forward(input_ids)  # 分布式forward
    loss = cross_entropy(out.logits, labels)
    loss.backward()  # 分布式backward
    opt.step()  # 仅更新Client参数
```

支持方法: Prompt tuning, Prefix tuning, LoRA, Adapters

### 性能基准

**BLOOM-176B推理性能** (NeurIPS 2023):

| 配置 | 带宽 | RTT | 推理速度(steps/s) |
|------|------|-----|-------------------|
| 3×A100 | 1 Gbit/s | <5ms | 1.71 |
| 3×A100 | 100 Mbit/s | 100ms | 1.23 |
| 10×RTX 3090 | 1 Gbit/s | <5ms | 1.17 |
| 14×异构GPU（实际跨洲部署） | Real-world | - | 0.83 |
| Offloading（理论最优） | PCIe 4.0 | - | 0.18 |

**关键结论**: 分布式推理比Offloading快**10x以上**

**多Client并发**: 8个Client同时推理时，每个Client仅有约20%性能下降

### 系统要求

| 角色 | RAM | GPU | 带宽 |
|------|-----|-----|------|
| Client | ≥12GB | 可选（高级采样需要） | ≥25 Mbit/s |
| Server | ≥16GB | ≥8GB | ≥100 Mbit/s |

### 局限性与未来方向

1. **隐私问题**: 第一层Server可恢复Client输入tokens
   - 缓解：使用可信Server或隔离网络
   - 未来：安全多方计算(MPC)或隐私硬件

2. **供需平衡**: Client不强制运行Server
   - 解决方案：积分激励系统

3. **恶意节点**: Server可能返回错误结果
   - 解决方案：Validator验证 + 质押惩罚机制

### Pleiades可借鉴点

| Petals特性 | Pleiades借鉴点 |
|------------|----------------|
| Client-Server分离 | 计算节点与协调节点职责分离 |
| 双缓存容错 | 分布式推理的容错策略 |
| DHT负载均衡 | libp2p Kademlia复用 |
| D*Lite路由 | 最优Server链选择 |
| 8-bit量化 | ort支持INT8推理 |

### 与Pleiades设计差异

| 方面 | Petals | Pleiades |
|------|--------|----------|
| **运行时** | PyTorch + hivemind | ONNX Runtime (ort) |
| **模型格式** | HuggingFace格式 | ONNX（支持物理切分） |
| **传输层** | hivemind (gRPC) | libp2p (Rust原生) |
| **语言** | Python | Rust |
| **目标场景** | 研究者协作 | 边缘设备联合推理 |

---

## exo: 家庭设备AI集群

**项目地址**: https://github.com/exo-explore/exo

**实现**: Python (61.8%) + Svelte (16.6%) + Swift (9.7%) + Rust (6.3%)

**许可**: Apache-2.0

**社区热度**: 40k+ stars

### 核心定位

将家庭日常设备（Mac Studio、MacBook、iPhone等）连接成统一的AI集群，运行超大规模模型（如DeepSeek v3.1 671B、Qwen3-235B）。

### 核心特性

#### 1. 自动设备发现（Automatic Device Discovery）

设备运行exo后自动发现彼此，无需手动配置网络。

#### 2. RDMA over Thunderbolt 5

- macOS 26.2新增功能
- 支持设备：M4 Pro Mac Mini, M4 Max Mac Studio, M4 Max MacBook Pro, M3 Ultra Mac Studio
- **延迟减少99%**
- 启用方式：Recovery Mode下执行`rdma_ctl enable`

#### 3. 拓扑感知自动并行（Topology-Aware Auto Parallel）

exo根据实时设备拓扑自动决定最优模型切分方案，考虑因素：
- 设备计算资源
- 网络延迟
- 各链路带宽

#### 4. Tensor Parallelism

支持张量并行（非仅Pipeline并行）：
- 2设备：1.8x加速
- 4设备：3.2x加速

### 技术架构

```
┌─────────────────────────────────────────────────┐
│              exo Dashboard (Svelte)             │
│         http://localhost:52415                  │
└───────────────────────┬─────────────────────────┘
                        │ REST API (OpenAI兼容)
┌───────────────────────▼─────────────────────────┐
│                exo Master Node                  │
│  - 设备发现 (mDNS/Bonjour)                      │
│  - 拓扑分析                                      │
│  - 模型分片调度                                  │
└───────────────────────┬─────────────────────────┘
        ┌───────────────┼───────────────┐
        ▼               ▼               ▼
┌───────────────┐ ┌───────────────┐ ┌───────────────┐
│   Node 1      │ │   Node 2      │ │   Node 3      │
│ M3 Ultra      │←→│ M4 Max        │←→│ MacBook Pro   │
│ 192GB RAM     │   │ 128GB RAM     │   │ 48GB RAM      │
│ (MLX Runtime) │   │ (MLX Runtime) │   │ (MLX Runtime) │
└───────────────┘ └───────────────┘ └───────────────┘
       ↑_______________|_______________|
               RDMA over Thunderbolt 5
```

### 支持模型

| 模型 | 规模 | 配置示例 |
|------|------|----------|
| DeepSeek v3.1 | 671B | 4×M3 Ultra Mac Studio |
| Qwen3 | 235B (8-bit) | 4×M3 Ultra Mac Studio |
| Kimi K2 Thinking | native 4-bit | 4×M3 Ultra Mac Studio |
| Llama 3.2 | 1B-70B | 单设备或多设备 |

### 推理引擎

- **macOS**: MLX（Apple Silicon优化），GPU加速
- **Linux**: 当前仅CPU支持，GPU支持开发中

### API接口

OpenAI兼容API：

```bash
# 列出可用模型
curl http://localhost:52415/models

# 创建实例
curl -X POST http://localhost:52415/instance \
  -H 'Content-Type: application/json' \
  -d '{"instance": {...}}'

# Chat Completions (OpenAI格式)
curl http://localhost:52415/v1/chat/completions \
  -d '{
    "model": "llama-3.2-1b",
    "messages": [{"role": "user", "content": "Hello"}],
    "stream": true
  }'

# 查看集群状态
curl http://localhost:52415/state
```

### 硬件要求

| 平台 | GPU支持 | 最低系统版本 |
|------|---------|--------------|
| macOS | ✅ Metal | macOS Tahoe 26.2+ |
| Linux | ❌ (开发中) | - |
| Windows | ❌ | - |

### 与Petals对比

| 方面 | exo | Petals |
|------|-----|--------|
| **目标场景** | 家庭设备集群 | 互联网志愿者协作 |
| **网络** | 局域网/Thunderbolt | 互联网 |
| **延迟优化** | RDMA (μs级) | 网络容错 (ms级) |
| **并行策略** | Tensor + Pipeline | Pipeline |
| **推理引擎** | MLX (Apple) | PyTorch |
| **容错** | 设备离线重连 | 双缓存故障恢复 |
| **微调支持** | ❌ | ✅ (LoRA/Prompt tuning) |

### Pleiades可借鉴点

| exo特性 | Pleiades借鉴点 |
|---------|----------------|
| 自动设备发现 | libp2p mDNS |
| 拓扑感知调度 | 异构设备层分配策略 |
| Tensor Parallelism | 考虑张量切分（当前仅Pipeline） |
| OpenAI兼容API | HTTP API设计参考 |
| Dashboard | 可视化监控界面 |

### 局限性

1. **平台限制**: 主要面向Apple Silicon，Linux/Windows支持薄弱
2. **网络依赖**: 最佳性能需要Thunderbolt直连
3. **无微调支持**: 仅推理，不支持分布式微调
4. **闭源核心**: 部分RDMA优化可能依赖macOS私有API

---

## PRIMA.cpp: 低资源家庭集群上的70B规模LLM推理加速

**论文**: PRIMA.CPP: Speeding Up 70B-Scale LLM Inference on Low-Resource Everyday Home Clusters

**项目地址**: https://github.com/lookastarik/LOcalprima.cpp

### 核心问题

在低资源家庭设备集群（笔记本、台式机、手机、平板）上高效运行70B规模LLM，现有系统存在以下限制：
- 需要GPU集群、大内存、高带宽
- 端侧方案仅支持小模型（<10B）
- 内存不足时频繁OOM

### 核心贡献

#### 1. Piped-Ring Parallelism（管道环形并行）

设备连接成环形结构，每个设备处理固定层数后传递hidden state给下一个设备。与传统Pipeline区别：
- 可多轮次预测一个token
- Layer Window Size可变（强设备窗口更大）
- 使用mmap + WILLNEED实现prefetch

**注**: 此技术与DeepSpeed的虚拟流水线技术(Virtual Pipeline)原理相同，并非全新贡献。

#### 2. LDA问题建模（Layer-to-Device Assignment）

将层到设备分配问题形式化为**整数线性分数规划模型**：

**目标函数**:
$$\min_{w,n} L \cdot \frac{a^T \cdot w + b^T \cdot n + e^T \cdot c}{e^T \cdot w} + \kappa$$

**决策变量**:
- $w_m$: 设备m每轮处理的层数（layer window size）
- $n_m$: 设备m中由GPU处理的层数

**数学本质**: "模型总层数L与总窗口大小$e^Tw$之比"乘以"各设备计算、内存访问与通信延迟的线性组合"

**约束条件**:
- GPU层数不超过总窗口
- 所有设备执行相同推理轮次
- 针对macOS/Linux/Android等不同OS及Metal/CUDA等硬件后端，差异化设置RAM/VRAM上限

#### 3. Halda求解算法

**核心技巧**: 枚举所有可能的k值（轮次数，为L的因子），问题退化为整数线性规划(ILP)问题，然后对每个k值求解，最终选择最优解。

复杂度: $O(M^2 \cdot \log L \cdot M^{3.5})$，多项式时间可解。

### 实验结果

在4节点家庭集群（Mac M1 + Linux笔记本 + Linux台式机 + Android手机）上：
- 比llama.cpp快17×（70B模型）
- 内存压力<6%
- Token延迟约567ms（70B模型）

---

### 个人评价与思考

#### LDA建模问题

论文的目标函数采用**所有设备延迟之和**：
$$T = \sum_{m=1}^{M}(T_{comp}^m + T_{mem}^m + T_{disk}^m + T_{comm}^m)$$

这存在严重问题：**完全忽略了流水线中设备间的并行时间重叠**。

实际流水线执行中，当设备1在预取下一轮数据时，设备2、3可以同时在计算。真正的token延迟应该是：
$$T_{real} \approx k \times \max_m(T_m) + \text{bubble}$$

而非简单的$\sum_m T_m$。这导致优化目标与实际延迟存在数倍误差。

#### 替代方案思考：P2P流式权重传输

**设想**: 一台设备运行时，其他设备通过P2P网络向其"灌"权重，用过的权重直接丢弃。

**理论分析**:

| 参数 | 值 |
|------|-----|
| 每层权重大小(70B Q4) | ~500MB |
| 单层计算时间(GPU) | ~7ms |
| 单层计算时间(CPU) | ~100ms |
| P2P传输时间(3×千兆) | ~1.7秒 |
| P2P传输时间(3×万兆) | ~167ms |

**结论**: 
- GPU场景：P2P传输比计算慢100-500倍，方案不可行
- CPU+万兆：接近平衡，有可行性
- 该方案本质是用网络带宽换设备存储/磁盘带宽，适用于弱设备+高速网络场景

#### GGUF格式的质疑

PRIMA.cpp基于llama.cpp构建，使用GGUF模型格式。论文未详细说明如何实现模型分层，这里存在关键问题：

**GGUF格式的局限性**：
| 问题 | 说明 |
|------|------|
| 单文件设计 | GGUF设计为整体存储，元数据+张量连续排列 |
| 无原生切分支持 | 没有官方工具按层切分模型 |
| mmap整体映射 | 即使只用几层，也需要映射整个文件 |
| 物理分发困难 | 每个设备都需要完整模型文件，只是访问不同偏移量 |

**推测其实现方式**：
- mmap映射整个GGUF文件
- 通过张量名称前缀（如`blk.{layer_id}`）定位层边界
- 每个设备只**访问**指定层的内存偏移，而非**物理切分**

**问题**：这意味着在P2P场景下，无法实现"设备A只下载layer 0-10，设备B只下载layer 11-20"，每个设备都需要获取完整模型文件。

#### Pleiades选择ONNX的原因

针对GGUF的上述问题，Pleiades项目选择ONNX格式作为模型承载方案：

| 特性 | ONNX | GGUF |
|------|------|------|
| **层/子图切分** | ✅ `onnx.utils.extract_model` | ❌ 无原生支持 |
| 单文件自包含 | ✅ 计算图+权重+IO定义 | ✅ 元数据+张量 |
| 物理分发 | ✅ 各设备只需下载自己的子图 | ❌ 需要完整文件 |
| Rust集成 | ✅ ort crate成熟 | ⚠️ ggml-rs不成熟 |
| 后端支持 | ✅ CUDA/Metal/CPU自动切换 | ⚠️ 需要编译时指定 |

**ONNX切分示例**：
```python
from onnx.utils import extract_model

# 设备A负责的子图
extract_model("full.onnx", "part_0.onnx",
              input_names=["input"],
              output_names=["layer_7_output"])

# 设备B负责的子图
extract_model("full.onnx", "part_1.onnx",
              input_names=["layer_7_output"],
              output_names=["output"])
```

这实现了**真正的物理切分**，每个设备只需通过P2P下载自己负责的子图文件。

#### 有价值的参考点

1. **LDA问题的形式化**: 将异构设备层分配问题形式化为优化问题的思路值得借鉴
2. **枚举k值简化求解**: 通过枚举有限的k值将ILFP转化为ILP的技巧实用
3. **跨平台OS行为建模**: 考虑macOS/Linux/Android内存管理差异的思路有参考价值
4. **mmap懒加载**: 使用mmap管理模型权重避免OOM的方案可直接复用

---

## Parallax: 去中心化环境下的高效LLM推理服务

**论文**: Parallax: Efficient LLM Inference Service over Decentralized Environment

**项目地址**: https://github.com/GradientHQ/parallax

**实现**: Python + vLLM runtime

### 核心问题

在去中心化志愿者GPU池上实现高效LLM推理服务，主要挑战包括：
- GPU**异构性**（算力、内存、带宽差异大）
- **网络带宽低**（跨区域链路仅数百MB/s）
- 节点**动态可用性**

### 核心贡献：两阶段调度算法

#### 1. 离线模型分配（Phase-1）

**目标**: 将L层模型分配到N个异构GPU上，形成多个流水线副本，最小化请求延迟，最大化系统吞吐量。

**问题本质**: 装箱问题，最优分配为NP-hard（与PRIMA.cpp类似）。

**关键观察**:
- 跨地域GPU连接有超低带宽
- 通信时间远大于计算时间

**启发式策略**:
| 策略 | 说明 |
|------|------|
| **区域优先** | 层分配必须在单一地理区域，不允许跨区域切分 |
| **延迟主导** | 优先选择流水线阶段数少的方案 |

**调度目标**:
1. 最小化单条流水线的阶段数
2. 最大化流水线副本数量k

**求解方法**:
- **动态规划**求解最佳分配方案
- **Water-Filling算法**平衡各stage层数，使执行时间均衡

**目标函数**: $Z(k) = k^α / (T_{comp} + (s^*(k)/k) \cdot r_{RTT})$

#### 2. 在线GPU链选择（Phase-2）

**目标**: 为每个推理请求动态构建端到端执行路径。

**DHT存储内容**:
- 节点标识
- RAM容量
- 周期性发布的延迟信息
- RTT等实时性能指标

DHT实时反映集群状态，通过**DAG寻找从Layer1到LayerL的最短路径**。

**复杂度**: $O(L\bar{R}^2)$时间，$O(L\bar{R})$空间

**负载均衡机制**: 高负载GPU的延迟值$τ$增大，后续请求自动绕行。

#### 3. 成员动态管理

基于DHT的标准机制：
- GPU加入：贪心分配至瓶颈层，更新DHT
- GPU离开：释放层分配，DHT条目过期清除
- 全局重平衡触发条件：①无完整流水线 ②负载变异系数超阈值

### 实验结果

**测试环境**: 5×RTX 5090 + 2×RTX 4090，跨数据中心，平均网络延迟10ms

**对比基准**: HexGen（异构LLM推理引擎）

| 指标 | 提升幅度 |
|------|----------|
| **吞吐量** | 最高3.6×，平均1.58× |
| **延迟** | 最高降低3.2×，平均1.66× |

### 相关工作参考价值

论文对分布式集群和数据中心区别的描述非常好，可供借鉴。

相关工作部分，被充分认可的去中心化LLM推理系统主要是**Petals**。

### Pleiades可借鉴点

| Parallax特性 | Pleiades可借鉴点 |
|--------------|------------------|
| 区域感知的层分配 | P2P节点分组策略 |
| DHT实时性能发布 | libp2p Kademlia DHT复用 |
| Water-Filling负载均衡 | 异构设备层数自适应 |
| DAG最短路径选择 | 请求路由优化 |
