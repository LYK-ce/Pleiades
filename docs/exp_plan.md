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

# 拆分 Qwen3-235B-A22B (假设 total_layers=60，根据实际调整)
> exec split model=Qwen3-235B-A22B.pgguf start=0 end=20
> exec split model=Qwen3-235B-A22B.pgguf start=21 end=40
> exec split model=Qwen3-235B-A22B.pgguf start=41 end=59
```

输出文件:
```
Qwen3-235B-A22B_split_0_20.pgguf
Qwen3-235B-A22B_split_21_40.pgguf
Qwen3-235B-A22B_split_41_59.pgguf
```

> **注意**: 每个分片共享同一个 `model_id`（xxhash32），可跨分片互相识别。

#### ② 模型分发

```bash
# 将分片 scp 到各节点的 Pleiades_Workspace 目录
scp Qwen3-235B-A22B_split_0_20.pgguf  haoxiang01:/data/.../Pleiades_Workspace/
scp Qwen3-235B-A22B_split_21_40.pgguf haoxiang02:/data/.../Pleiades_Workspace/
scp Qwen3-235B-A22B_split_41_59.pgguf yatao:/data/.../Pleiades_Workspace/
```

#### ③ 各节点 flush + 启动

```bash
# 【每个节点】
./Pleiades cli

> flush                    # 索引模型文件
[Storage] flush 完成: 新增 1 个文件

# 验证模型已被发现（Coordinator 节点可查）
> ls
Qwen3-235B-A22B_split_0_20.pgguf  135GB  model_id=12345678  [0-20]
```

#### ④ 网络验证

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
> exec pipeline model=Qwen3-235B-A22B_split_0_20.pgguf prompt=你好
```

> Coordinator 节点必须持有参数中指定的分片文件（用于获取 model_id + tokenizer）。

#### 预期输出

```
[coord] 模型: /data/.../Qwen3-235B-A22B_split_0_20.pgguf, id=12345678, 总层: 60
[coord] 链条: 3 个节点
  [1] haoxiang01: layers [0,20] file=Qwen3-235B-A22B_split_0_20.pgguf
  [2] haoxiang02: layers [21,40] file=Qwen3-235B-A22B_split_21_40.pgguf
  [3] yatao:      layers [41,59] file=Qwen3-235B-A22B_split_41_59.pgguf
[coord] rexec → haoxiang01: EXEC|pipe_worker|{"model":"Qwen3-235B-A22B_split_0_20.pgguf",...}
[coord] haoxiang01 响应: OK
[coord] rexec → haoxiang02: EXEC|pipe_worker|{"model":"Qwen3-235B-A22B_split_21_40.pgguf",...}
[coord] haoxiang02 响应: OK
[coord] rexec → yatao: EXEC|pipe_worker|{"model":"Qwen3-235B-A22B_split_41_59.pgguf",...}
[coord] yatao 响应: OK
[coord] 建立 tensor stream ...
[coord] fwd stream 已打开 (→ haoxiang01)
[coord] bwd stream 已建立 (← yatao)
[coord] tokenizer 已加载
[coord] Prompt: 你好
[coord] 编码: 2 tokens
[coord] hidden 已发送
[worker] layers=[0,20] up=nil down=12D3...haoxiang02 coord=12D3...haoxiang01 id=1717...
[worker] 模型: /data/.../Qwen3-235B-A22B_split_0_20.pgguf
[worker] cuda:0 ← [0,10]  cuda:1 ← [11,20]
[worker] GPU:0 模型加载完成
[worker] GPU:1 模型加载完成
[worker] 等待入站 stream (timeout=120s)...
[worker] 入站 stream 已建立
[worker] 打开出站 stream → 12D3...haoxiang02
[worker] 出站 stream 已打开
[worker] GPU:0 forward done (0.23s)
[worker] GPU:1 forward done (0.18s)
[worker] iter 1 完成
你好！我是 Qwen，一个由阿里巴巴开发的大语言模型...
<eos>
[coord] 完成, 共 42 tokens
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
