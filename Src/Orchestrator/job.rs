// Presented by KeJi
// Date ： 2026-05-16

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

/// 作业类型。
///
/// 旧 `Run`/`Coordinator`/`Relay`/`Pipeline`/`Distribute`/`Send`/`Profile` 全部合并为 `Generic`，
/// 业务逻辑由 Core 的 route_* 方法直接编排。
#[derive(Debug, Clone, Copy)]
pub enum JobKind {
    /// 通用任务 (todo!() 占位，未来由 Lua 脚本替代)
    Generic,
    /// 文件接收 (Network → Storage)
    ReceiveFile,
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
