# Pleiades — 去中心化 AI 推理网络

[![Rust](https://img.shields.io/badge/Rust-2021%20edition-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Pleiades** 是一个去中心化的 P2P AI 推理网络。节点之间通过 libp2p 协议发现彼此、共享模型分片，并协同完成分布式大语言模型推理。每个节点可以持有模型的一部分层，通过张量流（Tensor Stream）传递中间隐藏状态，实现流水线并行推理。

## 核心特性

- **P2P 网络**: 基于 libp2p (TCP + Noise + Yamux)，支持 mDNS 局域网发现和 Kademlia DHT
- **分布式推理**: 模型按层切分，多个节点流水线并行执行推理
- **PGGUF 模型格式**: 在标准 GGUF 基础上追加模型 ID 和层位图元数据
- **Lua 脚本引擎**: 沙箱化的 Lua 运行时，用户可编写脚本来编排推理任务
- **统一存储管理**: 文件访问锁定、惰性发现、完整性校验（BLAKE3/SHA256）
- **终端 UI**: 基于 ratatui 的实时监控面板

## 系统要求

- **Rust** 1.80+
- **CUDA** (可选，默认启用): 用于 GPU 加速推理
- **操作系统**: Linux (Arch, Ubuntu), macOS

## 快速开始

### 构建

```bash
# CPU 模式 (无 GPU)
cargo build --release --no-default-features

# CUDA 模式 (默认)
cargo build --release

# 针对本机 CPU 优化
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

### 运行

```bash
# 将模型文件放入工作目录
mkdir -p Pleiades_Workspace
cp /path/to/your-model.gguf Pleiades_Workspace/

# 启动节点
cargo run --release

# 在 TUI 中输入命令:
#   flush       — 扫描并注册工作目录中的模型
#   ls          — 列出可用模型
#   exec run    — 执行内置 run 脚本（单机推理）
#   help        — 查看所有命令
```

### 配置文件

首次运行自动生成 `.config/config.toml`（共享段由 `pleiades-base` 的 `BaseConfig` 定义；设备段如 `[chassis]` / `[lidar]` / `[flight_ctrl]` / `[um960]` / `[lg290p]` / `[camera]` 由各设备端 crate 启动时补全）：

```toml
[Log]       level = "info", log_file_path = "Log/"
[Network]   LAN = true, WAN = false, Transport_Protocol = "TCP",
            listen_port = 0, dht_namespace = "pleiades-nodes", bootstrap_peers = []
[Storage]   workspace_dir = "Pleiades_Workspace", quota_gb = 0, kvcache_dir = ".kvcache"
[Identity]  peer_name = "new_peer", node_type = "car"   # car=车 / uav=机 / ground_station=地面站
```

## 项目结构

Cargo workspace（5 个 crate）：

```
Orion/
├── Cargo.toml              ← workspace 清单
├── pleiades-base/          ← 核心库 + [[bin]] Pleiades：Config/EventBus/ML_Engine/Network/
│                              Orchestrator/PeerManagement/Robot/Session_Manager/Storage/TUI/API/VM
├── pleiades-terminal/      ← GDExtension 地面站桥（PleiadesKernel）+ RTK 基站 UM960
├── pleiades-uav/           ← 无人机设备端：mavlink 飞控 / camera USB 摄像头
├── pleiades-ugv/           ← 无人车设备端：stm32 / lidar / lg290p(RTK 流动站) / slam / planning
├── pleiades-sim/           ← 仿真节点（无硬件，跑完整决策链，用于测试）
├── programs/               ← Lua 脚本
│   ├── builtin/            ← 内置脚本
│   └── user/               ← 用户脚本
├── Architecture/           ← 架构设计文档
├── docs/                   ← 设计文档
└── pleiades-base/tests/    ← 集成测试
```

机器人子系统：共享中枢在 `pleiades-base/src/Robot/`（状态 / 命令 / 集群 / 世界模型 / ORION 协议），设备端实现分布在 `pleiades-ugv/`、`pleiades-uav/`、`pleiades-terminal/`，另有 `pleiades-sim/` 仿真节点；节点间通过 **ORION 二进制协议**（帧头 + 校验和，POSE / MAP_FULL / MAP_DELTA / MANUAL_CONTROL / TASK_SET / RTCM 等消息）在 libp2p 上通信。详见 [Architecture/robot_arch.md](Architecture/robot_arch.md)。

## 架构概览

```
┌────────────┐    ┌────────────┐    ┌────────────┐
│  Node A    │    │  Node B    │    │  Node C    │
│ layers 0-3 │◄──►│ layers 4-7 │◄──►│ layers 8-11│
└────────────┘    └────────────┘    └────────────┘
      │                 │                 │
      └─────────── P2P Network ───────────┘
                (libp2p + Noise)
```

每个节点持有模型的若干层，通过 Tensor Stream 协议传递中间隐藏状态，协同完成推理。

## 文档

- [架构文档](Architecture/Pleiades_Architecture.md)
- [设计文档](docs/)
- [Agent 开发规范](.github/instructions.md)

## 许可证

MIT
