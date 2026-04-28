// Presented by KeJi
// Date ： 2026-04-27

//! LLM_IO 模块 — 大语言模型文本交互层
//!
//! 负责为外部前端（TUI 或 API）与 ML Thread 之间建立有状态的双向文本通道。
//! - 输入：外部前端发送的文本 Prompt（`String`）
//! - 输出：ML Thread 返回的文本 Completion（`String`）
//!
//! ## 使用流程
//! 1. 编排层调用 `broker.Allocate(job_id)` 创建通道对
//! 2. 编排层调用 `broker.Take_ML_Side(job_id)` 取出 ML 侧端点，注入 JobExecutor
//! 3. 前端/测试调用 `broker.Take_Frontend(job_id)` 取出前端侧端点，进行文本交互
//! 4. Job 结束后调用 `broker.Deallocate(job_id)` 清理内部索引

pub mod capability;
pub mod broker;

// ─── 聚合导出 ───────────────────────────────────────────────

pub use capability::{LLM_IO_Capability, LLM_IO_Error, IoFrontend, IoHandle};
pub use broker::LLM_IO_Broker;

// ─── 模块级集成测试 ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::job::JobId;

    // 验证外部视角的 use 路径正确
    #[test]
    fn test_module_imports_compile() {
        // 如果以下类型无法引用，编译即失败
        let _: fn() -> LLM_IO_Broker = LLM_IO_Broker::New;
        fn _assert_trait_object(_: &dyn LLM_IO_Capability) {}
        let _ = LLM_IO_Error::AllocationFailed("test".to_string());
        let _ = LLM_IO_Error::NotFound("test".to_string());
    }

    // 端到端测试：Allocate → Take 两端 → 前端发 Prompt → ML 侧收 → ML 侧发回复 → 前端收 → Deallocate
    #[tokio::test]
    async fn test_end_to_end_allocate_chat_drop() {
        let broker = LLM_IO_Broker::New();
        let job_id = JobId(100);

        // 分配通道
        broker.Allocate(job_id).await.unwrap();
        assert!(broker.Is_Active(job_id).await);

        // 取出两端
        let mut frontend = broker.Take_Frontend(job_id).await.unwrap();
        let mut ml_side = broker.Take_ML_Side(job_id).await.unwrap();

        // 前端发送 Prompt
        frontend.input_tx.send("What is Rust?".to_string()).await.unwrap();

        // ML 侧接收 Prompt
        let prompt = ml_side.input_rx.recv().await.unwrap();
        assert_eq!(prompt, "What is Rust?");

        // ML 侧发送 Completion
        ml_side.output_tx.send("Rust is a systems programming language.".to_string()).await.unwrap();

        // 前端接收 Completion
        let completion = frontend.output_rx.recv().await.unwrap();
        assert_eq!(completion, "Rust is a systems programming language.");

        // 多轮对话
        frontend.input_tx.send("Tell me more.".to_string()).await.unwrap();
        let prompt_2 = ml_side.input_rx.recv().await.unwrap();
        assert_eq!(prompt_2, "Tell me more.");

        ml_side.output_tx.send("It focuses on safety and performance.".to_string()).await.unwrap();
        let completion_2 = frontend.output_rx.recv().await.unwrap();
        assert_eq!(completion_2, "It focuses on safety and performance.");

        // 回收通道
        broker.Deallocate(job_id).await.unwrap();
        assert!(!broker.Is_Active(job_id).await);

        // drop 两端
        drop(frontend);
        drop(ml_side);
    }
}
