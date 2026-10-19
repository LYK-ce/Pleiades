# Pleiades 项目总体介绍

> Presented by KeJi
> Date: 2026-05-21
> Modified Date: 2026-10-19

## 概述

Pleiades 是一个**边缘设备分布式推理运行时框架**。它让多台设备（笔记本、工作站、边缘服务器等）通过 P2P 网络协同完成大规模语言模型的推理，每台设备只负责模型的一部分层。

核心思想：**单机跑不动的大模型，拆成几片分给局域网里的几台机器一起跑。**

2026 下半年起，Pleiades 扩展出**机器人控制**方向：`pleiades-base` 内新增 Robot 子系统与 **ORION 统一通信协议**，工程按 5 个 crate 组织（`pleiades-base` / `ugv` / `uav` / `terminal` / `sim`，见「工程结构」）。

---

## 设计理念：Lua 虚拟机驱动一切

Pleiades 最核心的设计决策是**用 Lua 虚拟机替代传统硬编码编排**。所有调度策略、推理流程、节点选择逻辑全部写成 `.lua` 脚本，Rust 层只提供高性能的能力函数（网络收发、模型推理、文件存储等）。

<details>
<summary>为什么选 Lua？</summary>

- **语法简洁**：零学习成本，比 Rhai/Python 更适合嵌入
- **工业成熟度**：Lua 在游戏引擎、路由器、边缘设备中广泛使用（mlua + vendored Lua 5.4）
- **动态重载**：修改脚本无需重新编译，`reload` 命令即可热加载
- **天然沙箱**：禁用 os/io/require 后，脚本无法触碰系统，安全性由 Rust 保证
</details>

### 虚拟机架构

```
┌─────────────────────────────────────────────────────────────┐
│                    programs/*.lua 脚本                       │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────────────┐ │
│  │ pipeline.lua │ │  run.lua     │ │ 用户自定义脚本         │ │
│  │ (分布式推理)  │ │ (本地推理)    │ │ (programs/user/*.lua) │ │
│  └──────────────┘ └──────────────┘ └──────────────────────┘ │
├─────────────────────────────────────────────────────────────┤
│         Lua VM 层 (pleiades-base/src/VM/)                          │
│  ┌─────────────┐  ┌────────────────┐  ┌─────────────────┐  │
│  │ engine.rs   │  │ registry.rs    │  │capability_binding│  │
│  │ 沙箱创建     │  │ 脚本注册表      │  │ Rust→Lua 桥接    │  │
│  └─────────────┘  └────────────────┘  └─────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│         Rust 能力层 (纯 API，无业务逻辑)                       │
│  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌────────┐  │
│  │Net   │ │ML    │ │Store │ │Peer  │ │Event │ │Local   │  │
│  │P2P   │ │GGUF  │ │文件   │ │节点   │ │Bus   │ │Tensor  │  │
│  └──────┘ └──────┘ └──────┘ └──────┘ └──────┘ └────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### 虚拟机好处

| 对比维度 | 硬编码 Rust 编排 | Lua 虚拟机驱动 |
|----------|------------------|---------------|
| 新增推理策略 | 改 Rust → 重新编译 → 部署 | 写 `.lua` → `reload` 即生效 |
| 用户自定义 | 不可能 | `programs/user/` 目录下任意加脚本 |
| 代码量 | 每个策略 ~500 行 Rust | 每个策略 ~50 行 Lua |
| 安全边界 | Rust 编译器保证 | 沙箱禁用 os/io/require |
| 调试 | 需要 Rust 工具链 | `print()` 直接显示在 TUI 日志 |
| 部署 | 二进制替换 | 同步 `.lua` 文件即可 |

### 典型脚本示例：分布式流水线推理

```lua
-- programs/builtin/pipeline.lua
COMMAND = "pipeline"
DESCRIPTION = "分布式流水线推理"

function execute(params, caps)
    -- 1. 分析模型结构 → Rust: analyze_model()
    local arch = caps.analyze_model(params.model)

    -- 2. 发现可用节点 → Rust: Get_Peers()
    local peers = caps.get_available_peers()

    -- 3. 调度策略 → Rust: plan_uniform() / plan_weighted()
    local plan = caps.plan_uniform(arch, peers)

    -- 4. 分发模型分片 → Rust: split_model() + send_file()
    for _, w in ipairs(plan.workers) do
        caps.split_model(params.model, w.layer_start, w.layer_end)
        caps.send_file(w.peer_id, shard)
    end

    -- 5. 建立张量流 → Rust: open_tensor_stream()
    caps.establish_streams(plan)

    -- 6. 推理循环（纯 Lua 控制流）
    local sess = caps.create_session(params.model, "cuda", 0, 39)
    sess:forward(sess:tensorize(tokens), 0)

    for i = 1, 120 do
        local hidden = sess:get_output_tensor()
        caps.send_tensor(hidden, workers[1].peer_id)
        local result = caps.receive_tensor()
        sess:forward(result, nil)
        local tok = sess:sample(0.8)
        print(sess:decode(tok))
        if tok == sess:get_eos() then break end
    end
end
```

### 能力函数分级

Lua 虚拟机通过 `caps.*` / `network.*` / `ml.*` / `local_tensor.*` 暴露 Rust 能力：

| 级别 | 命名空间 | 函数数 | 用途 |
|------|---------|--------|------|
| A 级 | `caps.*` | ~8 | 无需模型的基础操作（节点发现、事件发布、IO 分配） |
| B 级 | `ml.*` | ~3 | 模型文件操作（分析结构、拆分模型、加载模型） |
| C 级 | `caps.*` | ~3 | 会话管理（创建/销毁推理会话） |
| D 级 | `sess:*` | ~10 | ML 推理指令（forward / sample / encode / decode ...） |
| 网络 | `network.*` | ~10 | P2P 通信（send_data / dial / send_file / open_stream ...） |
| 本地 | `local_tensor.*` | ~5 | 线程间张量流（open / accept / send / recv / eof） |

---

## 系统架构总览

```
┌──────────────────────────────────────────────────────────┐
│                        TUI 终端界面                       │
├──────────────────────────────────────────────────────────┤
│                  Orchestrator Core 编排器                 │
│  ┌──────────────┐ ┌──────────────┐ ┌─────────────────┐  │
│  │ branch_user  │ │ branch_cmd   │ │ branch_stream   │  │
│  │ (用户命令)    │ │ (网络命令)    │ │ (流事件)         │  │
│  └──────────────┘ └──────────────┘ └─────────────────┘  │
├──────────────────────────────────────────────────────────┤
│   ProgramRegistry (脚本注册) → Lua VM (沙箱) → 能力函数   │
├──────────┬──────────┬──────────┬───────────┬────────────┤
│ Network  │ ML Engine│ Storage  │ PeerMgr   │ EventBus   │
│ (P2P)    │ (GGUF)   │ (文件)    │ (节点)     │ (事件总线)  │
├──────────┴──────────┴──────────┴───────────┴────────────┤
│                操作系统 + GPU Driver                      │
└──────────────────────────────────────────────────────────┘
```

## 典型工作流

```
1. 多台设备启动 Pleiades → mDNS 互相发现 → 连接建立
2. 用户在 TUI 输入: exec pipeline model=llama3.gguf
3. Orchestrator Core 从 ProgramRegistry 找到 pipeline.lua
4. 创建沙箱 Lua VM → 注入所有能力函数 → 执行脚本
5. Lua 脚本驱动整个流程:
   a. ml.analyze_model() → 分析 GGUF 结构
   b. caps.get_available_peers() → 查找可用节点
   c. caps.plan_uniform() → 计算各节点层分配
   d. ml.split_model() → 按层拆分 + network.send_file() → 分发
   e. network.open_tensor_stream() → 建立各节点间张量流
   f. 推理循环: forward → send_tensor → recv_tensor → sample → decode
6. 推理结果通过 TUI Command 面板实时显示
```

## 模块清单

| 模块 | 目录 | 职责 |
|------|------|------|
| Config | `pleiades-base/src/Config/` | TOML 配置读写 |
| Network | `pleiades-base/src/Network/` | P2P 网络、Gossipsub、Tensor/File/Bandwidth Stream、Rendezvous |
| ML Engine | `pleiades-base/src/ML_Engine/` | GGUF 模型加载/推理/拆分/分析 |
| Orchestrator | `pleiades-base/src/Orchestrator/` | Core 编排、命令解析、本地张量流 |
| Peer Mgmt | `pleiades-base/src/PeerManagement/` | 节点状态表、节点名称、Profile |
| Storage | `pleiades-base/src/Storage/` | 文件追踪、并发读写锁、校验 |
| Session | `pleiades-base/src/Session_Manager/` | LLM 会话管理 |
| EventBus | `pleiades-base/src/EventBus/` | 全局事件广播（Notify/State/Stream/StreamRaw/Output） |
| TUI | `pleiades-base/src/TUI/` | 终端图形界面（双输入框、4面板） |
| VM | `pleiades-base/src/VM/` | Lua 沙箱引擎、能力函数桥接、脚本注册表 |
| API | `pleiades-base/src/API/` | OpenAI 兼容 API Server、YOLO WebUI |
| CLI | `pleiades-base/src/CLI/` | 命令行入口 |
| Robot | `pleiades-base/src/Robot/` | 机器人统一核心：ORION 协议、状态/栅格/任务/集群、串口 |
| Programs | `programs/` | Lua 脚本（builtin 系统 + user 用户） |

### 工程结构（5 个 crate）

| crate | 目录 | 职责 |
|-------|------|------|
| `pleiades-base` | `pleiades-base/` | 共享运行时：网络 / ML / 存储 / 编排 / Lua VM / TUI / API + **Robot 子系统**（含 ORION 协议） |
| `pleiades-ugv` | `pleiades-ugv/` | 无人车：STM32 底盘、LiDAR、SLAM、规划/执行、LG290P RTK 流动站 |
| `pleiades-uav` | `pleiades-uav/` | 无人机：MAVLink、相机、规划/执行 |
| `pleiades-terminal` | `pleiades-terminal/` | 地面站/控制终端：UM960 RTK 基站、Pictor（Godot）GDExtension 桥 |
| `pleiades-sim` | `pleiades-sim/` | 仿真车端：规划/执行仿真 |

## 当前完成度

> 更新至 2026-10-19（原 2026-05 快照已过时）。✅ = 代码已落地；待确认 = 无法在不构建的前提下验证行为；❌ = 全库无对应实现。

| 领域 | 状态 | 依据 |
|------|------|------|
| Lua 基础设施（沙箱 + 注册表 + 能力函数） | ✅ | `pleiades-base/src/VM/` |
| TOML 模板 → Lua 脚本迁移 | ✅ | `programs/builtin/` + `programs/user/` |
| 遗留 VM 代码清理（Vm_Base/ML_VM/Orch_VM/Scheduler） | ✅ | 全库无 `Scheduler_Capability` 等引用 |
| TUI 命令体系（run/send/reload/exec/pipeline/...） | ✅ | `pleiades-base/src/TUI/` |
| 独立本地流（Local Tensor Stream） | ✅ | `Orchestrator/local_tensor_stream/` |
| 5 crate 拆分（base/ugv/uav/terminal/sim） | ✅ | `Cargo.toml` workspace members |
| Robot 子系统（状态/栅格/任务/集群/串口） | ✅ | `pleiades-base/src/Robot/` |
| ORION 统一协议（帧 + msgid 1~6 + 命令解码） | ✅ | `Robot/core/protocol/` |
| 多车集群（gossipsub 广播 + 任务群发 + 集群地图合并） | ✅ | `Robot/core/cluster/` + `Network/Gossipsub/` |
| RTK 接入（UM960 基站 + LG290P 流动站，msgid=6） | ✅ | `pleiades-terminal/src/device/um960/`、`pleiades-ugv/src/device/lg290p/` |
| YOLO 检测 + WebUI 展示 | ✅ | `API/webui.rs` + `programs/user/yolo_*.lua` |
| Pipeline 端到端联调（Coordinator + Worker） | 待确认 | `programs/user/pipeline_coord*.lua`、`llm*.lua` 已存在 |
| Relay Job 指令完善 | 待确认 | — |
| GGUF 模型加载优化（From_Extracted → New） | 待确认 | `Layer_Weights` 与 `From_Extracted` 并存 |
| 心跳故障处理（滑动窗口） | 待确认 | `Network/network_service.rs` 有 heartbeat 相关代码 |
| PeerStore 共享组件 | ❌ | 全库 0 命中 `PeerStore` |
| pending_responses 超时清理 | 待确认 | `Network/Request_Response/outbound.rs` |
| ML Engine 推理路径优化（零拷贝） | 待确认 | — |
| mDNS 服务名自定义 | 待确认 | `Network/` 有 mDNS 代码 |
| TUI 命令补全动态化 | 待确认 | — |

## 开发方向

1. **Pipeline 端到端联调** — Coordinator + Worker 完整推理流程验证，Relay Job 指令实现
2. **模型加载优化** — `Layer_Weights::New()` 替代 `From_Extracted()`，单次打开文件顺序加载
3. **可靠性增强** — 心跳滑动窗口、超时自动重连、推理任务中断通知
4. **性能优化** — ML Engine 数据平面/控制平面分离、零拷贝张量传输、预分配缓冲区
5. **线程间通信** — 本地张量流完善 + 线程生命周期管理
6. **TUI 增强** — 命令补全动态化、Prompt 推理面板通路打通
7. **广域网支持** — mDNS 服务名自定义、DHT 引导节点、NAT 穿透

## 技术栈

| 层次 | 技术 |
|------|------|
| 语言 | Rust (edition 2021) |
| 异步 | Tokio |
| P2P | libp2p (mDNS / Kademlia / Noise / Yamux) |
| ML | Candle (GGUF 推理) |
| 脚本引擎 | Lua 5.4 (mlua, vendored) |
| 序列化 | bincode / serde / serde_json |
| 终端 | ratatui + crossterm |
| GPU | CUDA (candle-core/cuda, nvml-wrapper) |
| 存储校验 | BLAKE3 / SHA256 |
| 机器人 | UGV/UAV/仿真端 + ORION 协议 + RTK（UM960 基站 / LG290P 流动站） |
