// Presented by KeJi
// Date ： 2026-04-21

use super::job::JobId;

/// 用户命令，来自 CLI / TUI / GUI 等前端接入层
#[derive(Debug, Clone)]
pub enum UserCommand {
    /// 启动本地推理作业
    Run {
        model_path: String,
    },
    /// 取消指定作业
    Cancel {
        job_id: JobId,
    },
    /// 请求优雅退出
    Quit,
    /// 查询当前节点列表
    DisplayPeer,
    /// 修改默认计算设备偏好
    SetDevice {
        device: String,
    },
}

/// 网络命令，来自 Network Capability，仅包含需要改变作业生命周期的事件
#[derive(Debug, Clone)]
pub enum NetworkCommand {
    /// 远程 Coordinator 请求本节点作为 Worker 加入流水线
    PipelineFlow {
        peer_id: String, // 占位符，实际可能有更多字段
    },
}