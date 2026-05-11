// Presented by KeJi
// Date ： 2026-04-27

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy)]
pub enum JobKind {
    /// 单机推理
    Run,
    /// 分布式协调者（旧版，保留兼容）
    Coordinator,
    /// 分布式中继 Worker
    Relay,
    /// 文件分发（模型分片 + 发送）
    Distribute,
    /// 文件接收（入站文件流 → Storage 写入）
    Receive,
    /// 单文件发送（send <file> <peer>）
    Send,
    /// 三阶段分布式流水线编排（Coordinator 侧）
    Pipeline,
    /// 模型性能 Profile
    Profile,
}

#[derive(Debug, Clone, Copy)]
pub enum JobState {
    Preparing,
    Ready,
    Executing,
    CleaningUp,
    Done,
}

#[derive(Debug, Clone)]
pub enum JobResult {
    Success,
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
pub enum LifecycleEvent {
    Done { job_id: JobId, result: JobResult },
}
