// Presented by KeJi
// Date ： 2026-04-30

//! Tensor_IO 模块 — 张量流管理与热切换层
//!
//! 负责管理分布式推理中的张量流（Tensor Stream），提供：
//! - **Core 管理张量流**：实际的 `libp2p::Stream` 由 Switch 持有，通过 `Mutex` 保护
//! - **ML Thread 只拿接口**：Session 通过 `Arc` 引用访问 Mutex 内的 stream，零拷贝
//! - **Core 热切换**：换目标时锁住 Mutex → 替换 stream → 解锁，ML Thread 无感
//! - **广播支持**：Outbound 使用 `Vec<Stream>`，一次 Send 写入所有目标
//! - **故障上报**：单个目标失败不中断推理，通过 channel 异步通知 Core
//!
//! ## 使用流程
//! 1. Core 建立网络连接（open_tensor_stream + TensorStreamArrived）
//! 2. Core 调用 `switch.Register(job_id, inbound, outbound, rt)` → 返回 (Endpoint, FailureRx)
//! 3. Endpoint 注入 ML Session；FailureRx 由 Core 轮询处理
//! 4. ML Thread 调用 `endpoint.Receive()` / `endpoint.Send()` 做张量 I/O
//! 5. Core 可随时调用 `Swap_Input` / `Set_Outputs` 热切换流目标
//! 6. Job 结束后 Core 调用 `switch.Deregister(job_id)` 清理

pub mod tensor_port_switch;

// ─── 聚合导出 ───────────────────────────────────────────────

pub use tensor_port_switch::{
    Tensor_Port_Switch, Tensor_IO_Endpoint,
    Tensor_IO_Error, FailureReport,
};
