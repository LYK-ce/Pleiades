//Presented by KeJi
//Date ： 2026-05-16

//! ML Engine 独立操作
//!
//! 提供无需 Session 的模型分析/切分操作。
//! 由 `gguf_model_manager.rs` 提供底层实现。

#![allow(non_snake_case)]

use std::path::Path;

use super::gguf_model_manager::{GGUF_Analyze_And_Convert, GGUF_Split_Model, Model_Arch_Info};

// ============================================================
// 模型分析
// ============================================================

/// 解析 GGUF/PGGUF 模型文件，返回架构元信息。
///
/// 如果是原始 .gguf：自动计算 model_id + layer_bitmap，转换并重命名为 .pgguf。
/// 如果已是 .pgguf：直接读取已存储的 model_id + layer_bitmap。
///
/// 使用 `tokio::task::spawn_blocking` 执行同步 I/O，
/// 调用方必须在 tokio runtime 上下文中调用。
pub async fn analyze_model(gguf_file_path: &Path) -> Result<Model_Arch_Info, String> {
    let path = gguf_file_path.to_path_buf();
    tokio::task::spawn_blocking(move || GGUF_Analyze_And_Convert(&path).map(|(a, _)| a))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
        .map_err(|e| format!("GGUF_Analyze_And_Convert failed: {e}"))
}

// ============================================================
// 模型切分
// ============================================================

/// 从 GGUF 文件切分出指定层范围的子模型。
///
/// 输出文件自动命名为 `{stem}_split_{start}_{end}.pgguf`，
/// 保存在 `output_dir` 目录下。
///
/// 使用 `tokio::task::spawn_blocking` 执行同步 I/O，
/// 调用方必须在 tokio runtime 上下文中调用。
pub async fn split_model(
    gguf_file_path: &Path,
    split_start: usize,
    split_end: usize,
    output_dir: &Path,
) -> Result<(), String> {
    let src = gguf_file_path.to_path_buf();
    let out = output_dir.to_path_buf();
    tokio::task::spawn_blocking(move || GGUF_Split_Model(&src, split_start, split_end, &out))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
        .map_err(|e| format!("GGUF_Split_Model failed: {e}"))
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn analyze_nonexistent_file_returns_error() {
        let result = analyze_model(Path::new("nonexistent.gguf")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn split_invalid_range_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = split_model(
            Path::new("nonexistent.gguf"),
            10,
            5, // start > end
            tmp.path(),
        )
        .await;
        assert!(result.is_err());
    }
}
