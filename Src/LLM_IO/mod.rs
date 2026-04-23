// Presented by KeJi
// Date ： 2026-04-23

//! LLM_IO 模块 — 大语言模型文本交互层
//!
//! 负责为外部前端（TUI 或 API）与 ML Thread 之间建立有状态的双向文本通道。
//! - 输入：外部前端发送的文本 Prompt（`String`）
//! - 输出：ML Thread 返回的文本 Completion（`String`）

pub mod capability;
pub mod broker;

// ─── 聚合导出 ───────────────────────────────────────────────

pub use capability::{LLM_IO_Capability, LLM_IO_Error, IoChannels, IoFrontend, IoHandle};
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
    }

    // 端到端测试：allocate → 前端发 Prompt → ML 侧收 → ML 侧发回复 → 前端收 → deallocate
    #[tokio::test]
    async fn test_end_to_end_allocate_chat_drop() {
        let broker = LLM_IO_Broker::New();
        let job_id = JobId(100);

        // 分配通道
        let channels = broker.Allocate(job_id).await.unwrap();
        assert!(broker.Is_Active(job_id).await);

        let mut frontend = channels.frontend;
        let mut ml_side = channels.ml_side;

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
