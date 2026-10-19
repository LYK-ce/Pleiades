# archived_docs 归档索引

> Presented by KeJi
> Created Date ： 2026-10-19
> Modified Date ： 2026-10-19

---

## 归档策略

- **归档 = 保留历史快照**：本目录中的文档仅作为历史记录留存，用于追溯设计演进与决策过程，**不再维护、不再更新**，也不作为当前实现的依据。
- **现行文档**位于 [`docs/`](../docs/) 与 [`docs/design_doc/`](../docs/design_doc/)，请以这两处的文档为准。
- 已被新版取代或确认无价值的旧文档已在本次清理中从本目录删除；其余保留项见下方清单。
- 本索引按 `ls archived_docs/` 的实际顺序（拉丁字母在前、中文在后）列出全部归档文件。

---

## 归档文件清单（共 21 篇）

| 文件 | 说明 |
|------|------|
| [P2P LLM可行性探究.md](P2P%20LLM可行性探究.md) | 从通信延迟组成、带宽与算力约束出发，论证 P2P 分布式部署 LLM 的可行性（2026-01-08）。 |
| [P2PLLM设计探究.md](P2PLLM设计探究.md) | P2PLLM 去中心化推理系统设计探究：DHT 组网、模型分片、动态节点管理与流水线并行推理（2026-01-12）。 |
| [Rust_Candle.md](Rust_Candle.md) | Hugging Face Candle 深度学习框架调研与运行时用法说明，面向边缘部署与 P2P 场景（2026-01-14）。 |
| [Rust_Installation.md](Rust_Installation.md) | Rust 开发环境配置指南，涵盖 Windows / Linux / macOS 安装步骤（2026-01-13）。 |
| [Rust_Libp2p.md](Rust_Libp2p.md) | Rust libp2p 模块化 P2P 网络协议栈的功能与依赖配置使用指南（2026-01-14）。 |
| [Rust_Manager.md](Rust_Manager.md) | Rust 管理层实现方案：被动/主动模式、命令收发与节点选择的依赖分析（2026-01-14）。 |
| [Rust_Onnx.md](Rust_Onnx.md) | Rust ONNX Runtime 选型与集成文档，对比性能、模型通用性、子图切分与 Rust 集成（2026-01-14）。 |
| [Rust_log.md](Rust_log.md) | Rust 日志模块实现方案，推荐 `tracing` 生态并给出依赖与配置（2026-01-14）。 |
| [code_review_lua_integration.md](code_review_lua_integration.md) | Lua 集成 + MlSession 重构的代码审查记录，覆盖 VM 引擎、能力绑定与 Orchestrator 改动（2026-05-17，分支 machine-learning-reforge）。 |
| [demo_modification.md](demo_modification.md) | demo 分支修改文档：修复 `list_model_peers` 等问题，使分布式流水线推理端到端可运行（2026-06-02）。 |
| [event_bus_reforge.md](event_bus_reforge.md) | EventBus 重构实施计划：将 `Bus_Event` 由 14 个领域耦合变体收敛为 4 个通用类型，payload 改为 JSON 字符串。 |
| [heartbeat.md](heartbeat.md) | 基于 libp2p 内置 ping 协议实现节点心跳检测与 RTT 延迟测量的方案（2026-04-16）。 |
| [lua_integration_plan.md](lua_integration_plan.md) | Lua（mlua）集成为 Pleiades 策略脚本引擎的方案，替代已删除的 VM 体系，采用渐进式集成（2026-05-17，分支 alpha）。 |
| [pictor_websocket_protocol.md](pictor_websocket_protocol.md) | 从 Pictor 项目复制的 WebSocket 通信协议参考，用于 Orion 接入可视化（2026-07-21）。 |
| [problem.md](problem.md) | 根目录归档：Lua 选型决策日志（Rhai vs Lua、Lua 替代范围、Capabilities 职责、命令约定等 5 项决策，均已落地）（2026-05-13）。 |
| [request_response_reforge.md](request_response_reforge.md) | Request_Response 传输子系统提取方案，使 Network 三种传输机制结构对称（2026-05-16）。 |
| [session_manager_design.md](session_manager_design.md) | SessionManager 早期设计提案：会话注册表 + 槽位分配 + 文本通道管理；文首已标注实际实现有重大变更（2026-05-14）。 |
| [summary.md](summary.md) | Pleiades 项目模块现状总结：编译与测试状态、各模块功能与测试覆盖总览（2026-05-17）。 |
| [why_dsv4_cannot_run.md](why_dsv4_cannot_run.md) | 分析 Qwen3 235B 可运行而 DeepSeek V4 Flash 不可运行的原因：模型格式与加载器缺失（2026-05-31）。 |
| [潜在研究方向.md](潜在研究方向.md) | 基于 Petals、exo、PRIMA.cpp、Parallax 等相关工作，梳理四个潜在研究方向并评估学术价值与可行性（2026-01-15）。 |
| [研究方向探讨.md](研究方向探讨.md) | 围绕 Pleiades 现有架构（libp2p + candle GGUF + 流水线并行）探讨三个可能的研究方向，如异构设备最优模型分配。 |
