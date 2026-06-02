# Fleet Workstations 实验环境总结

> 更新时间: 2026-06-02
> 用于 Task 16 动态流水线编排的集群部署参考

---

## 1. 设备清单

| 机器 | IP | GPU | RAM | 磁盘 (可用) | OS |
|------|-----|-----|-----|-------------|-----|
| **yatao** | 10.100.6.104 | 2× RTX 4090 (48G) | 128G | 1.67T (1.65T) | Arch Linux |
| **haoxiang01** | 192.168.31.22 | 2× RTX 4090 (48G) | 128G | 939G (775G) | Ubuntu 22.04.5 |
| **haoxiang02** | 192.168.31.240 | 2× RTX 4090 (48G) | 128G | 939G (902G) | Ubuntu 22.04.2 |

- **GPU 合计**：6× RTX 4090，全部空闲
- **状态**：三台在线，sudo 可用

---

## 2. 网络拓扑

```
         10.100.6.0/24                    192.168.31.0/24
             yatao                          haoxiang01    haoxiang02
            (.104)                            (.22)        (.240)
               │                                 │             │
               │          1G                      │     1G      │
               └──── [10.100.6.102 网关] ─────────┘             │
                         (NAT/路由)            └─── 100G直连 ───┘
                                                  (未配IP, RDMA可用)
```

### 链路详情

| 链路 | 带宽 | 连通性 | 备注 |
|------|------|--------|------|
| haoxiang01 ↔ haoxiang02 (eno1) | 1G | ✅ 双向 | 已验证 NCCL: 1.78 Gbps |
| haoxiang01 ↔ haoxiang02 (enp2s0np0) | 100G | ✅ 双向 | 物理UP, 无IP, ConnectX-5 RDMA |
| haoxiang → yatao | 1G | ✅ 单向 | haoxiang 可 ping yatao |
| yatao → haoxiang | 1G | ❌ | NAT 阻断，需隧道/端口转发 |

### 关键限制

- **yatao 无法直接访问 haoxiang 的 192.168.31.x 地址**
- 三机 NCCL 需额外配置：SSH 反向隧道 或 网关转发 或 100G 口配 IP + 路由
- haoxiang01/02 之间 100G RDMA 链路物理就绪但未配 IP，当前仅 1G 可用

---

## 3. 软件环境

三台统一，路径 `/data/dist-infer-exp/miniforge/envs/dist-infer`：

| 组件 | 版本 | 备注 |
|------|------|------|
| Python | 3.12 | |
| PyTorch | 2.5.1 | |
| CUDA | 12.4 | 受 haoxiang 550 驱动约束 (≤12.4) |
| NCCL | 2.21.5 | |
| GPU Driver | yatao: 595 / haoxiang: 550 | |

### 激活环境

```bash
/data/dist-infer-exp/miniforge/bin/conda activate dist-infer
```

---

## 4. 运行分布式任务

### 4.1 双机 NCCL（haoxiang01 + haoxiang02）✅ 已验证

```bash
# haoxiang01 (node_rank=0)
NCCL_SOCKET_IFNAME=eno1 torchrun \
  --nnodes=2 --nproc_per_node=2 --node_rank=0 \
  --master_addr=192.168.31.22 --master_port=29500 \
  your_script.py

# haoxiang02 (node_rank=1)
NCCL_SOCKET_IFNAME=eno1 torchrun \
  --nnodes=2 --nproc_per_node=2 --node_rank=1 \
  --master_addr=192.168.31.22 --master_port=29500 \
  your_script.py
```

- **必须** 设 `NCCL_SOCKET_IFNAME=eno1` 确保走 1G 管理口

### 4.2 三机 NCCL（待配置）

yatao 需能访问 haoxiang。可选方案：

| 方案 | 描述 | 优缺点 |
|------|------|--------|
| SSH 反向隧道 | haoxiang 各开 `ssh -R` 到 yatao | 简单、无需改网络；额外延迟 |
| 网关端口转发 | 在 10.100.6.102 上做 DNAT | 需网关权限 |
| 100G 口配 IP + 路由 | 给 100G 口配同一子网IP，yatao 路由可达 | 最优性能；需配路由 |

---

## 5. 基准性能

| 场景 | 带宽 |
|------|------|
| haoxiang 机内 (2 GPU, NVLink) | 271 Gbps |
| haoxiang 跨机 (4 GPU, 1G eno1) | 1.78 Gbps |

---

## 6. 对 Task 16 的影响分析

### 6.1 Pleiades 网络层适配

Pleiades 使用 libp2p 进行 peer discovery 和 tensor stream，底层依赖 TCP。当前网络存在以下问题：

- **yatao ↔ haoxiang 链路单向阻断**：libp2p 的 `kad` (Kademlia DHT) 和 `mdns` 都需要双向可达。NAT 环境下 yatao 无法被 haoxiang 主动连接。
- **解决方案**：在 yatao 上部署 relay 或使用 libp2p 的 ` autonat` + `relay` 机制。或者最简单：haoxiang01/02 主动 dial yatao，维持长连接。

### 6.2 流水线带宽瓶颈

| 跳类型 | 当前带宽 | 影响 |
|--------|----------|------|
| 机内 GPU:0→GPU:1 | ~271 Gbps | 无瓶颈 |
| 跨机 (1G eno1) | ~1.78 Gbps | **严重瓶颈** |
| 跨机 (100G RDMA) | 待测 | 可期望 50+ Gbps |

- **双机流水线（haoxiang01 + haoxiang02）**：1G 链路对于 hidden state 传输（典型 ~8MB/token）理论饱和吞吐 ~27 tokens/s，实际打折扣后可能只有 ~10-15 tokens/s。
- **三机流水线**：若 yatao 加入且走 1G 链路，两跳跨机将吞吐再对半。

### 6.3 建议优先级

1. **短期**：打通 yatao ↔ haoxiang 的 SSH 反向隧道，先跑通三机流水线（即使慢）
2. **中期**：给 enp2s0np0 (100G) 配 IP，验证 Pleiades tensor stream 在 100G 上的吞吐
3. **长期**：如果需要 Pleiades Native 模式，将 100G 口纳入 libp2p 的 transport 层（需 RDMA 支持或 TCP/IP over 100G）

---

## 7. 讨论要点

- [ ] Task 16 先在双机 (haoxiang01+02) 验证，还是直接上三机？
- [ ] 100G 链路是否需要立刻配置 IP？当前 RDMA 对 Pleiades 的 tensor stream 帮助不大（走 TCP），需要配 IP 走 TCP/IP
- [ ] yatao NAT 问题的优先级：SSH 隧道（最快） vs 网关转发？
- [ ] 模型如何分片到三机？考虑 4090 24G 单卡显存约束
