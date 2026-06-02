# 分布式推理实验计划

> 日期: 2026-06-02
> 集群: 8x2 RTX 4090 (48G), Pleiades demo 分支
> 模型: Qwen3-235B-A22B (~135GB Q4) + DeepSeek V3.2 (~400GB Q4)

---

## 一、演示操作

> 演示分两场：**Qwen3-235B (3节点, 轻量)** → **DeepSeek V3.2 (5节点, 重量级)**
>
> 显存: Qwen3 3节点 22.5 GB/GPU | DS V3.2 5节点 40 GB/GPU ✅

### 1.1 准备工作

#### 1 模型拆分

```bash
./Pleiades cli

# Qwen3: 拆 3 份
> exec split path=Qwen3-235B-A22B.pgguf num=3

# DeepSeek V3.2: 拆 5 份
> exec split path=DeepSeek-V3.2.pgguf num=5
```

每份共享同一 `model_id`，第一份保留 tokenizer。

#### 2 部署 Lua 脚本

```bash
# pipe_worker.lua 需预先存在于各节点 programs/user/
# 各节点 git pull 或 scp
```

#### 3 模型分发

```bash
# Qwen3 分片 -> 3 节点
scp Qwen3-235B-A22B_split_0_*.pgguf   node1:/path/Pleiades_Workspace/
scp Qwen3-235B-A22B_split_*_*.pgguf   node2:/path/Pleiades_Workspace/
scp Qwen3-235B-A22B_split_*_Z.pgguf   node3:/path/Pleiades_Workspace/

# DS V3.2 分片 -> 5 节点
scp DeepSeek-V3.2_split_*.pgguf       node1..5:/path/Pleiades_Workspace/
```

#### 4 各节点 flush

```bash
./Pleiades cli
> flush
```

#### 5 Coordinator 创建 Session

```bash
> session create Qwen3-235B-A22B_split_0_*.pgguf    # -> Session 1
> session create DeepSeek-V3.2_split_0_*.pgguf      # -> Session 2
```

### 1.2 演示一：Qwen3-235B (3节点, 135GB)

```bash
> session inference pipeline 1 Qwen3-235B-A22B_split_0_*.pgguf
```

**看点**: 自动发现 3 节点 -> 构建链条 -> 启动 worker -> tensor stream 桥接 -> 推理输出

预期输出:

```
╔══════════════════════════════════════════════╗
║   Pleiades 动态流水线 - 分布式推理演示       ║
╚══════════════════════════════════════════════╝

+- 阶段 1/4: 模型识别 ---------------------------+
| 模型文件: Qwen3-235B-A22B_split_0_XX.pgguf
| 架构: qwen3moe  总层数: XX  Model ID: XXXXXXXX
+------------------------------------------------+

+- 阶段 2/4: 集群发现 ---------------------------+
| ✓ 发现 3 个节点持有该模型:
|   [1] node1  层 [ 0 - XX]  ...split_0_XX.pgguf
|   [2] node2  层 [XX - YY]  ...split_XX_YY.pgguf
|   [3] node3  层 [YY - ZZ]  ...split_YY_ZZ.pgguf
+------------------------------------------------+

+- 阶段 3/4: 构建推理链条 -----------------------+
| ✓ 链条验证通过 - 层覆盖完整无间隙
| 流水线拓扑 (3 节点, 每节点 2xGPU):
|   +- [1] node1  层 0->XX   GPU:0->CPU->GPU:1
|   +- [2] node2  层 XX->YY  GPU:0->CPU->GPU:1
|   +- [3] node3  层 YY->ZZ  GPU:0->CPU->GPU:1
|   Session <-> [1] <-> [2] <-> [3] <-> Session
+------------------------------------------------+

+- 阶段 4/4: 分发 + 推理 ------------------------+
| 正在向各节点启动 pipe_worker ...
|   [1/3] -> node1  响应: OK
|   [2/3] -> node2  响应: OK
|   [3/3] -> node3  响应: OK
| fwd stream ✓  bwd stream ✓  Session ✓
| ║  ✓ 流水线就绪，开始推理  ║
+------------------------------------------------+

[worker] === 收到分片任务 ... ===  (x3)
[coord] Prefill 完成 (X.XXs)
你好！我是 Qwen ... (Session 输出生成的文本)
```

### 1.3 演示二：DeepSeek V3.2 (5节点, 400GB)

```bash
> session inference pipeline 2 DeepSeek-V3.2_split_0_*.pgguf
```

| | Qwen3 | DeepSeek V3.2 |
|---|---|---|
| 节点数 | 3 | 5 |
| 模型大小 | 135GB | **400GB** |
| 跨机跳 | 2 | 4 |
| 链条拓扑 | [1]<->[2]<->[3] | [1]<->[2]<->[3]<->[4]<->[5] |

输出格式与 Qwen3 相同，唯链条更长。核心看点：**400GB 模型在 5 台消费级 4090 上跑起来了**。

### 1.4 演示要点

| 时刻 | 现象 | 说明 |
|------|------|------|
| list_model_peers | 3/5 节点 + 各自层范围 | **自动发现**，无人干预 |
| rexec | 逐节点响应 OK | **自动启动** worker |
| tensor stream | 首尾流建立 | **自动拓扑** |
| 推理输出 | 正常生成中文 | **端到端跑通** |

---

## 二、实验矩阵

> 实验仅用 Qwen3-235B (3/4/5 节点, 无需 offloading)。DeepSeek V3.2 仅演示。

### 实验总览

| # | 脚本 | 节点 | 每 GPU 载荷 | 说明 |
|---|------|------|-----------|------|
| E1 | `exp_qwen_3.lua` | 3 | ~22.5GB | 三机流水线，刚好装下 |
| E2 | `exp_qwen_4.lua` | 4 | ~16.9GB | 四机流水线 |
| E3 | `exp_qwen_5.lua` | 5 | ~13.5GB | 五机流水线 |

每个脚本是独立的自包含流水线，`exec exp_qwen_3 model=xxx.pgguf tokens=128` 直接跑。

输出格式:
```
==== EXP_QWEN_3 RESULT ====
NODES:3  TOKENS:87  TOTAL_S:28.5  ENCODE_S:0.31
PREFILL_S:1.82  DECODE_S:26.4  TOK_S:3.30  TOK_S_E2E:3.05
==== EXP_QWEN_3 END ====
```

### 分析维度

| 指标 | 含义 |
|------|------|
| `TOK_S_E2E` vs 节点数 | 扩展效率曲线 |
| `PREFILL_S` vs 节点数 | prefill 是否随节点增加而加速 |
| `DECODE_S` 占比 | 网络通信在总延迟中的比重 |

---

## 三、论文叙事线

```
1. 问题: 大模型推理需要昂贵的专用集群 (A100/H100)
2. 方案: Pleiades - 消费级 GPU 集群上的动态流水线编排
3. 演示: 3/5 节点自动发现 -> 建链 -> 推理（Qwen3 + DS V3.2）
4. E1: Scaling 曲线（核心结果）
5. E2+E3: 瓶颈分析（机内桥接 vs 跨机网络）
6. E4: Pipeline bubble 分析
7. E5+E6: 极端场景（单节点 + Offloading 兜底）
8. 结论: 8x4090 可替代 1xA100 用于百亿 MoE 推理
```

---

## 四、待办

- [ ] Qwen3 split num=5 + 分发到各节点
- [ ] DS V3.2 split num=5 + 分发到 5 节点
- [ ] 100G 链路配 IP
- [ ] 打通 yatao <-> haoxiang 双向网络
- [ ] 冒烟测试: 3 节点 Qwen3 pipeline
- [ ] 实现 exp_qwen_3.lua / exp_qwen_4.lua / exp_qwen_5.lua
- [ ] 实验数据收集 + 绘图
