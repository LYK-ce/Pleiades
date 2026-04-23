// Presented by KeJi
// Date ： 2026-04-21

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy)]
pub enum JobKind {
    Run,
    WorkerRelay,
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
    Done {
        job_id: JobId,
        result: JobResult,
    },
}