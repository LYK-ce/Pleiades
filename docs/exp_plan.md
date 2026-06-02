# 分布式推理实验计划

> 日期: 2026-06-02
> 集群: 8×2 RTX 4090 (48G), Pleiades demo 分支
> 模型: Qwen3-235B-A22B Q4_K_M + DeepSeek V3.2 Q4_K_M

---

## 一、演示操作

### 1.1 准备工作

#### ① 模型拆分（在任一节点执行）

```bash
# 进入 Pleiades CLI
./Pleiades cli

# 按份数均分 PGGUF（自动计算每份层数，余数分给前几份）
> exec split path=Qwen3-235B-A22B.pgguf num=3
```

输出文件:
```
Qwen3-235B-A22B_split_0_19.pgguf
Qwen3-235B-A22B_split_20_39.pgguf
Qwen3-235B-A22B_split_40_59.pgguf
```

> **注意**: 每个分片共享同一个 `model_id`（xxhash32），可跨分片互相识别。第一份保留 tokenizer。

#### ② 部署 Lua 脚本到各节点

```bash
# pipe_worker 脚本需预先存在于各节点的 programs/user/ 目录
# 方式1: git pull (如果各节点从同一仓库运行)
# 方式2: scp 脚本文件
scp programs/user/pipe_worker.lua haoxiang01:/path/to/Pleiades/programs/user/
scp programs/user/pipe_worker.lua haoxiang02:/path/to/Pleiades/programs/user/
```

> `rexec` 只触发远程节点执行本地已有的脚本，不传输脚本文件。

#### ③ 模型分发

```bash
# 将分片 scp 到各节点的 Pleiades_Workspace 目录
scp Qwen3-235B-A22B_split_0_20.pgguf  haoxiang01:/data/.../Pleiades_Workspace/
scp Qwen3-235B-A22B_split_21_40.pgguf haoxiang02:/data/.../Pleiades_Workspace/
scp Qwen3-235B-A22B_split_41_59.pgguf yatao:/data/.../Pleiades_Workspace/
```

#### ④ 各节点 flush + 启动

```bash
# 【每个节点】
./Pleiades cli

> flush                    # 索引模型文件
[Storage] flush 完成: 新增 1 个文件

# Coordinator 节点额外执行: 创建推理会话
> session create Qwen3-235B-A22B_split_0_19.pgguf
Session 1 created
```

#### ⑤ 网络验证

```bash
# 【Coordinator 节点】
> dp                       # 查看已发现 peer
[本地] haoxiang01  models: Qwen3-235B-A22B_split_0_20.pgguf
[远程] haoxiang02  models: Qwen3-235B-A22B_split_21_40.pgguf
[远程] yatao       models: Qwen3-235B-A22B_split_41_59.pgguf
```

### 1.2 执行演示

#### 命令

```bash
# Coordinator 节点执行
> session inference pipeline 1 Qwen3-235B-A22B_split_0_19.pgguf
```

> - `pipeline` = 双卡 Coordinator（COMMAND="pipeline"）
> - `1` = session_id（由 `session create` 返回）
> - `Qwen3-235B-A22B_split_0_19.pgguf` = coordinator 本地模型文件（用于获取 model_id）

> 单卡模式: `session inference pipeline_coord_single 1 model.pgguf`

#### 预期输出

```
╔══════════════════════════════════════════════╗
║   Pleiades 动态流水线 — 分布式推理演示       ║
╚══════════════════════════════════════════════╝

┌─ 阶段 1/4: 模型识别 ─────────────────────────┐
│ 模型文件 : Qwen3-235B-A22B_split_0_19.pgguf
│ 架构     : qwen3moe
│ 总层数   : 60 (embedding + 58 blocks + output)
│ Model ID : 12345678 (xxhash32, 跨分片唯一)
└──────────────────────────────────────────────┘

┌─ 阶段 2/4: 集群发现 ─────────────────────────┐
│ 搜索 Model ID = 12345678 的节点...
│ ✓ 发现 3 个节点持有该模型:
│   [1] haoxiang01           层 [ 0 - 19]  Qwen3-235B-A22B_split_0_19.pgguf
│   [2] haoxiang02           层 [20 - 39]  Qwen3-235B-A22B_split_20_39.pgguf
│   [3] yatao                层 [40 - 59]  Qwen3-235B-A22B_split_40_59.pgguf
└──────────────────────────────────────────────┘

┌─ 阶段 3/4: 构建推理链条 ─────────────────────┐
│ ✓ 链条验证通过 — 层覆盖完整无间隙
│
│ 流水线拓扑 (3 节点, 每节点 2×GPU):
│
│   ┌─ [1] haoxiang01
│   │   层 0→19  GPU:0→CPU→GPU:1 ← 本机(协调者)
│   ├─ [2] haoxiang02
│   │   层 20→39  GPU:0→CPU→GPU:1
│   └─ [3] yatao
│       层 40→59  GPU:0→CPU→GPU:1
│   Session ←→ 节点[1] ←→ 节点[2] ←→ 节点[3] ←→ Session
└──────────────────────────────────────────────┘

┌─ 阶段 4/4: 分发 + 推理 ──────────────────────┐
│ 推理会话 ID: 1
│
│ 正在向各节点启动 pipe_worker ...
│
│   [1/3] → haoxiang01  rexec pipe_worker
│         响应: OK
│   [2/3] → haoxiang02  rexec pipe_worker
│         响应: OK
│   [3/3] → yatao  rexec pipe_worker
│         响应: OK
│
│ 建立网络张量流 ...
│   fwd: coord → haoxiang01  ✓
│   bwd: yatao → coord  ✓
│
│ 连接 Session ...
│   local_tensor: session ↔ coord  ✓
│
│ ╔══════════════════════════════════╗
│ ║  ✓ 流水线就绪，开始推理          ║
│ ╚══════════════════════════════════╝
└──────────────────────────────────────────────┘

[worker] ═══ 收到分片任务: 层 [0 → 19] (共 20 层) ═══
[worker] GPU:0 ← 层 [0,9]  |  GPU:1 ← 层 [10,19]
[worker] GPU:0 加载完成 (3.12s)
[worker] GPU:1 加载完成 (2.98s)
[worker] 等待上游张量流...
[worker] ✓ 入站流已建立 (← coordinator)
[worker] ✓ 出站流已打开 (→ haoxiang02)
[worker] ═══ Worker 就绪，等待推理任务 ═══

[worker] ═══ 收到分片任务: 层 [20 → 39] (共 20 层) ═══
... (类似)

[worker] ═══ 收到分片任务: 层 [40 → 59] (共 20 层) ═══
... (类似)

[coord] Prefill 完成 (1.85s)
你好！我是 Qwen，一个由阿里巴巴开发的大语言模型...
... (Session 输出生成的文本) ...
[coord] Session 流结束
[coord] 流水线演示结束
```

### 1.3 演示要点（给观众看）

| 时刻 | 现象 | 说明 |
|------|------|------|
| `list_model_peers` 返回 | 3 个节点、各自的层范围 | **自动发现**，无人干预 |
| `rexec` 三次 | 三个节点收到 EXEC 命令 | **自动分发** worker 脚本 |
| tensor stream | 首尾流建立 | **自动拓扑**，跨节点链 |
| 推理输出 | 正常生成中文 | **端到端跑通** |

---

## 二、实验矩阵

### 2.1 实验总览

| # | 实验 | 节点 | 模型 | 指标 |
|---|------|------|------|------|
| E1 | Pipeline Scaling | 2→3→4→6→8 | Qwen3-235B + DS V3.2 | tok/s, 扩展效率 |
| E2 | 机内桥接开销 | 2→4 | 两者 | 双卡 vs 单卡 per-token 延迟 |
| E3 | 网络带宽影响 | 2 (haoxiang 对) | 两者 | 1G vs 100G tok/s |
| E4 | 架构对比 | 同节点数 | 两者 | pipeline bubble, 扩展效率差异 |
| E5 | 单节点极限 | 1 | 两者 | 能否单节点装下？offload 兜底？ |
| E6 | Chunked Offloading | 2 | DS V3.2 | chunks=1/2/4/8 延迟分布 |

### 2.2 E1: Pipeline Scaling 曲线

**目标**: 量化增加节点数对吞吐的影响，回答"加节点值不值"。

**配置**:

```
模型: Qwen3-235B + DeepSeek V3.2
节点: 2 / 3 / 4 / 6 / 8
每配置: prefill 512 tokens + decode 128 tokens, temperature=0.8
```

**数据收集**:

| 配置 | 每节点层数 | 跨机跳数 | tok/s | 扩展效率 |
|------|-----------|---------|-------|---------|
| 2 节点 | 30 | 1 | T₂ | 1.00 |
| 3 节点 | 20 | 2 | T₃ | T₃/(T₂×3/2) |
| 4 节点 | 15 | 3 | T₄ | T₄/(T₂×2) |
| 6 节点 | 10 | 5 | T₆ | T₆/(T₂×3) |
| 8 节点 | 7~8 | 7 | T₈ | T₈/(T₂×4) |

**预期**: 随节点增加，扩展效率递减。跨机 tensor stream 带宽（1G）成为瓶颈。

### 2.3 E2: 机内 GPU→CPU→GPU 桥接开销

**目标**: 量化 `to_device("cpu")` + `to_device("cuda:1")` 的开销。

**配置**:

```
每个节点数 (2/3/4) × 两种 worker (pipe_worker / pipe_worker_single)
→ 对比双卡 vs 单卡的 per-token 延迟
```

**数据**:

| 节点 | Worker | 机内跳 | 跨机跳 | tok/s |
|------|--------|--------|--------|-------|
| 2 | 双卡 | 2 | 1 | |
| 2 | 单卡 | 0 | 1 | |
| 3 | 双卡 | 2/节点 | 2 | |
| 3 | 单卡 | 0 | 2 | |

**预期**: 双卡比单卡多 10-30% 延迟（取决于 hidden state 大小），但双卡能用更大的层范围 → 需要更少节点 → 更少跨机跳。

### 2.4 E3: 网络带宽影响

**目标**: 量化 1G vs 100G 对流水线吞吐的实际影响。

**配置**:

```
2 节点 (haoxiang01 + haoxiang02)
× eno1(1G) vs enp2s0np0(100G, 需配 IP)
× Qwen3-235B + DS V3.2
```

**数据**: tok/s 对比。hidden state ~8MB/token (Y×X×F32)，1G 理论极限 ~27 tok/s。

### 2.5 E4: 模型架构对比

**目标**: 对比 Qwen3 MoE vs DeepSeek V3.2 MLA 在流水线下的行为差异。

**分析维度**:
- 每层 forward 耗时分布（Qwen3 MoE 层间差异大 vs DS V3.2 MLA 更均匀）
- Pipeline bubble 占比
- 扩展效率曲线形状

### 2.6 E5: 单节点极限

**目标**: 回答"最低几台机器能跑"。

| 模型 | 大小 | 单节点 96G | 结果 |
|------|------|-----------|------|
| Qwen3-235B | ~135GB Q4 | ❌ | 最低 2 节点 |
| DS V3.2 | ~170GB Q4 | ❌ | 最低 2 节点 |

> 可尝试 chunked offloading 让单节点慢速跑通，作为 baseline。

### 2.7 E6: Chunked Offloading

**目标**: 突破单卡显存限制时的降级方案。

**配置**: DS V3.2, 2 节点, chunks=1/2/4/8 per GPU。

**数据**: load / forward / offload 时间比，端到端 tok/s。

---

## 三、论文叙事线

```
1. 问题: 大模型推理需要昂贵的专用集群 (A100/H100)
2. 方案: Pleiades — 消费级 GPU 集群上的动态流水线编排
3. 演示: 3 节点自动发现 → 建链 → 推理（现场）
4. E1: Scaling 曲线（核心结果）
5. E4: Qwen3 vs DS V3.2 架构差异对流水线效率的影响
6. E2+E3: 瓶颈分析（机内桥接 vs 跨机网络）
7. E5+E6: 极端场景（最小节点 + Offloading 兜底）
8. 结论: 8×4090 可替代 1×A100 用于百亿 MoE 推理，成本降低 X%
```

---

## 四、待办清单

- [ ] 模型拆分 + 分发到 8 节点
- [ ] 100G 链路配 IP (haoxiang enp2s0np0)
- [ ] 打通 yatao ↔ haoxiang 双向网络
- [ ] 冒烟测试: 2 节点 pipeline
- [ ] pipe_worker 加 per-layer 计时（用于 E4 分析）
- [ ] 编写自动化实验脚本（一键跑 E1~E6）
- [ ] 实验数据收集 + 绘图
