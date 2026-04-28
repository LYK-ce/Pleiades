//Presented by KeJi
//Date ： 2026-04-27

use tokio::sync::oneshot;
use super::job::JobId;

/// 用户命令，来自 CLI / TUI / GUI 等前端接入层。
///
/// 每个变体携带 `reply` 通道，Core 处理完命令后通过 reply 回传结果。
/// 前端 `await` 回复即可获得处理结果（非 fire-and-forget）。
pub enum UserCommand {
    /// 启动本地推理作业
    ///
    /// 回复：`Ok(JobId)` 成功分配的 Job ID；`Err(String)` 编译或分配失败原因
    Run {
        model_path: String,
        reply: oneshot::Sender<Result<JobId, String>>,
    },
    /// 取消指定作业
    ///
    /// 回复：`Ok(())` 已发送取消信号；`Err(String)` Job 不存在
    Cancel {
        job_id: JobId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// 请求优雅退出
    ///
    /// 回复：`()` 确认已进入关闭流程
    Quit {
        reply: oneshot::Sender<()>,
    },
    /// 查询当前节点列表
    ///
    /// 回复：`Ok(Vec<String>)` 节点列表；`Err(String)` 查询失败
    DisplayPeer {
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    /// 修改默认计算设备偏好
    ///
    /// 回复：`Ok(())` 设置成功；`Err(String)` 设置失败
    SetDevice {
        device: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// 网络命令，来自 Network Capability / Control 层，仅包含需要改变作业生命周期的事件。
///
/// **注意**：不实现 `Clone`（`oneshot::Sender` 不可 Clone）。
/// 实际使用中通过 mpsc 通道传输（move 语义），无需 Clone。
pub enum NetworkCommand {
    /// 远程 Coordinator 请求本节点作为 Worker 加入流水线
    ///
    /// 由 Control 层解析 REQUEST_PIPELINE 入站请求后构造。
    /// Core 处理完（compile_relay + spawn）后通过 `reply` 回传 relay_job_id。
    PipelineFlow {
        /// Coordinator 的 PeerId（字符串格式）
        coordinator_peer_id: String,
        /// Coordinator 的 Job ID（用于 Relay 的 OpenTensorStream handshake）
        coordinator_job_id: u64,
        /// 模型文件 ID（本地已有的模型分片）
        model_file_id: String,
        /// 推理设备偏好
        device: String,
        /// 模型层范围 — 起始层
        layer_start: usize,
        /// 模型层范围 — 结束层
        layer_end: usize,
        /// 回复通道：成功返回分配的 Relay JobId，失败返回原因
        reply: oneshot::Sender<Result<JobId, String>>,
    },
}
