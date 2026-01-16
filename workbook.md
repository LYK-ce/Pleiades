# Workbook

## T1: Rust_Libp2p.md
- Start: 2026-01-14T11:59+08
- End: 2026-01-14T12:00+08
- Status: DONE
- Output: docs/Rust_Libp2p.md
- 功能覆盖: mdns/kad/noise/yamux/request-response
- 需求映射: 发现→mdns+kad | DHT→kad | 连接→tcp+quic+noise+yamux | 命令→req-resp | 文件→req-resp分块

## T2: Rust_Candle.md
- Start: 2026-01-14T13:02+08
- End: 2026-01-14T13:03+08
- Status: DONE
- Output: docs/Rust_Candle.md
- 功能覆盖: candle-core/candle-nn/candle-transformers/safetensors
- 需求映射: 加载→VarBuilder+SafeTensors | 推理→forward | 格式→SafeTensors | 切分→层范围加载 | P2P→Runtime_Command/Response

## T3: Rust_Onnx.md
- Start: 2026-01-14T13:54+08
- End: 2026-01-14T13:56+08
- Status: DONE
- Output: docs/Rust_Onnx.md
- 功能覆盖: ort/Session/ExecutionProvider/extract_model
- 需求映射: 格式→ONNX单文件 | 加载→Session | 推理→run() | 切分→extract_model | GPU→自动回退 | P2P→Runtime_Command/Response | 打包→static

## T4: Rust_Manager.md
- Start: 2026-01-14T14:10+08
- End: 2026-01-14T14:11+08
- Status: DONE
- Output: docs/Rust_Manager.md
- 功能覆盖: 命令定义/事件循环/节点选择/状态管理
- 需求映射: 被动→tokio::select!监听 | 主动→UserInput处理 | 命令→ManagerCommand枚举 | 节点选择→贪心算法
- 外部库: 无需新增，复用tokio/serde/libp2p

## T5: Rust_log.md
- Start: 2026-01-14T14:14+08
- End: 2026-01-14T14:15+08
- Status: DONE
- Output: docs/Rust_log.md
- 推荐库: tracing + tracing-subscriber
- 原因: 异步追踪/libp2p兼容/结构化日志
- 功能: EnvFilter级别控制 | #[instrument]异步span | JSON/pretty输出

## Petals相关工作整理
- Start: 2026-01-15T16:29+08
- End: 2026-01-15T16:31+08
- Status: DONE
- Output: 相关工作/RelatedWork.md (Petals条目)
- 源文件: Petals.pdf(EMNLP2023) + Distributed Inference.pdf(NeurIPS2023)
- 核心内容: Client-Server架构|双缓存容错|D*Lite路由|DHT负载均衡|8-bit量化|PEFT支持
- 性能关键: BLOOM-176B 1.7steps/s@3xA100|比Offloading快10x

## exo相关工作整理
- Start: 2026-01-15T16:32+08
- End: 2026-01-15T16:38+08
- Status: DONE
- Output: 相关工作/RelatedWork.md (exo条目)
- 源: https://github.com/exo-explore/exo (40k stars)
- 核心: 家庭设备AI集群|RDMA/TB5延迟99%↓|Tensor+Pipeline并行|MLX推理|自动设备发现
- 限制: Apple Silicon为主|无微调|Linux仅CPU
