JobExecutor 设计文档（补全版）
定位：统一执行框架。所有业务共享同一结构，接收预编译的 TaskProgram，驱动内部 TaskEngine 完成生命周期管理。
1. 模块归属
TaskEngine 不是独立组件，它是 JobExecutor 的私有内部模块。
plain
复制
src/orchestrator/
└── executor/
    ├── mod.rs           // JobExecutor 结构体 + run() 主循环（公开）
    └── task_engine.rs   // TaskEngine + SlotFile + TaskInstruction（私有，外部不可访问）
executor/mod.rs 中 mod task_engine;（不加 pub），外部模块无法引用 TaskEngine。
Core、Portal、Compiler 只知道 JobExecutor，不感知 TaskEngine 的存在。
2. 与 TaskEngine 的关系
驱动器与私有引擎。
TaskEngine 是 JobExecutor 的内部解释器，负责：
解释执行 TaskProgram 的 TaskInstruction
维护 SlotFile（跨步骤显式状态）
推进指令指针 IP
执行补偿序列
JobExecutor 负责：
持有 TaskEngine 实例
通过 select! 监听 Cancel 与 step() 结果
根据 StepResult 推进 JobState
发送死亡通知
边界：Executor 调用 task_engine.step()，但不干预指令内部执行；TaskEngine 返回 StepResult，但不感知 Cancel 信号和状态机。
3. 为什么 TaskEngine 必须私有？
表格
理由	说明
封装	指令解释是 Executor 的实现细节，外部不应依赖
测试分层	TaskEngine 可独立单元测试（构造 TaskProgram → 验证 SlotFile 状态），无需启动完整 tokio runtime
代码体量	step() 需要 match 所有 TaskInstruction，独立文件防止 Executor 文件膨胀
状态隔离	TaskEngine 维护 ip + slots（解释器状态），Executor 维护 state（生命周期状态），避免混在一起
4. 执行循环（不变）
rust
复制
// executor/mod.rs
mod task_engine;
use task_engine::{TaskEngine, StepResult};

pub struct JobExecutor {
    job_id: JobId,
    kind: JobKind,
    task_engine: TaskEngine,  // 私有持有
    cancel: CancellationToken,
    capabilities: Arc<Capabilities>,
    io: IoHandle,
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    state: JobState,
}

impl JobExecutor {
    pub async fn run(mut self) {
        self.task_engine.load(self.program);
        
        loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => {
                    self.task_engine.enter_compensation().await;
                    break;
                }
                result = self.task_engine.step(&self.capabilities) => {
                    match result {
                        StepResult::Continue => continue,
                        StepResult::Ready => { self.state = JobState::Ready; }
                        StepResult::Done => break,
                        StepResult::Abort(e) => {
                            self.report_error(e).await;
                            self.task_engine.enter_compensation().await;
                            break;
                        }
                    }
                }
            }
        }
        
        let _ = self.lifecycle_tx.send(LifecycleEvent::Done(self.job_id, ...)).await;
        self.cleanup().await;
    }
}
5. 边界红线（新增）
TaskEngine 禁止暴露给 Executor 外部。Core、Portal、Compiler 不得引用 TaskEngine 或 SlotFile。
TaskEngine 内部禁止 select!。纯顺序执行单条指令。
TaskEngine 不感知 Cancel。Cancel 由 Executor 的 select! 捕获，通过 enter_compensation() 切换执行序列。
