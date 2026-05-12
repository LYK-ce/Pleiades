//Presented by KeJi
//Date ： 2026-04-23

//! ML Engine 服务实现
//!
//! `ML_Engine_Service` 实现 `ML_Engine_Capability` trait，
//! 对上层（Orchestrator）提供统一的 ML 能力接口。
//!
//! ## 职责
//! - 管理 Session 生命周期（创建、运行、关闭）
//! - 通过 Storage 管理模型文件访问（读锁保护）
//! - 调用底层 Session_Handle 完成推理
//!
//! ## 内部结构
//! - `storage: Arc<StorageManager>` — 路径解析和文件锁
//! - `sessions: Mutex<HashMap<String, SessionEntry>>` — Session 注册表
//! - `SessionEntry` = `Session_Handle` + `ReadGuard`

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread;
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{info, warn};

use crate::llm_io::IoHandle;
use crate::storage::StorageManager;
use crate::storage::ReadGuard;
use crate::storage::StorageCapability;
use super::gguf_model_manager::{GGUF_Analyze, GGUF_Split_Model, Model_Arch_Info};
use super::session::{Session_Config, Session_Handle, Session_Thread};
use crate::ml_engine::ml_vm::MlInstruction;
use super::pipeline::{
    Pipeline_Params, Pipeline_Result, Model_Info,
};
use super::capability::{
    ML_Engine_Capability, ML_Engine_Error, ML_Session_Config,
};

// ─── Session 注册条目 ───────────────────────────────────────

/// Session 注册条目（内部类型）
///
/// 持有 Session_Handle（用于命令通信）和 ReadGuard（保护模型文件）。
/// Shutdown_Session 移除条目时，ReadGuard drop → Storage 读锁释放。
struct SessionEntry {
    /// 内部 Session 句柄
    handle: Session_Handle,
    /// Storage 读锁守卫 — Session 存活期间保护模型文件
    _storage_guard: ReadGuard,
}

// ─── ML_Engine_Service ──────────────────────────────────────

/// ML Engine 服务实现
///
/// 实现 `ML_Engine_Capability` trait，对上层提供统一的 ML 能力接口。
/// 内部持有 Storage 引用（用于路径解析和文件锁）和 Session 注册表。
///
/// ## Session 生命周期
/// 1. `Create_Session`: Storage acquire_read → spawn OS thread → 注册 SessionEntry
/// 2. `Run_Program`: 查表 → clone Handle → 提交指令序列
/// 3. `Shutdown_Session`: 移除 SessionEntry → 发送 Shutdown → ReadGuard drop 释放锁
///
/// ## deallocate 语义
/// Shutdown_Session 从注册表移除 SessionEntry，Handle 发送 Shutdown 命令停止线程，
/// ReadGuard drop 释放 Storage 文件锁。
pub struct ML_Engine_Service {
    /// Storage 引用 — 用于路径解析和文件锁
    storage: Arc<StorageManager>,
    /// Session 注册表
    sessions: Mutex<HashMap<String, SessionEntry>>,
}

impl ML_Engine_Service {
    /// 创建 ML_Engine_Service 实例
    ///
    /// # 参数
    /// - `storage`: Storage 管理器引用（Arc 共享）
    pub fn New(storage: Arc<StorageManager>) -> Self {
        Self {
            storage,
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ML_Engine_Capability for ML_Engine_Service {
    async fn Create_Session(
        &self,
        config: ML_Session_Config,
        io_handle: IoHandle,
    ) -> Result<Model_Info, ML_Engine_Error> {
        let session_id = config.session_id.clone();

        info!(
            "ML_Engine_Service: Create_Session [{}] (file: {}, layers: {}-{}, device: {})",
            session_id, config.model_file_id, config.layer_start, config.layer_end, config.device
        );

        // Step 1: 通过 Storage 获取模型文件路径 + 读锁
        let (model_path, storage_guard) = self
            .storage
            .acquire_read(&config.model_file_id)
            .await
            .map_err(|e| {
                ML_Engine_Error::SessionCreationFailed(format!(
                    "Storage acquire_read failed: {}", e
                ))
            })?;

        info!(
            "ML_Engine_Service: [{}] Storage 读锁获取成功, path: {}",
            session_id,
            model_path.display()
        );

        // Step 2: 创建命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel(32);

        // Step 3: 创建就绪信号通道
        let (ready_tx, ready_rx) = oneshot::channel();

        // Step 4: 构造 Session_Config（使用 Storage 解析后的物理路径）
        let session_config = Session_Config {
            model_path,
            layer_start: config.layer_start,
            layer_end: config.layer_end,
            device: config.device,
        };

        // Step 5: 启动 OS 线程
        let thread_session_id = session_id.clone();
        let tensor_io = config.tensor_io;
        thread::spawn(move || {
            Session_Thread(
                thread_session_id,
                session_config,
                cmd_rx,
                io_handle,
                tensor_io,
                ready_tx,
            );
        });

        // Step 6: await 就绪信号
        let model_info = ready_rx
            .await
            .map_err(|_| {
                ML_Engine_Error::SessionCreationFailed(format!(
                    "Session [{}]: 线程在发送就绪信号前终止",
                    session_id
                ))
            })?
            .map_err(|e| {
                ML_Engine_Error::SessionCreationFailed(format!(
                    "Session [{}]: 模型加载失败: {}",
                    session_id, e
                ))
            })?;

        info!(
            "ML_Engine_Service: [{}] Session 创建成功 (arch: {}, layers: {})",
            session_id, model_info.architecture, model_info.num_layers
        );

        // Step 7: 构造 Handle 并注册 SessionEntry
        let handle = Session_Handle::New(session_id.clone(), cmd_tx);
        let entry = SessionEntry {
            handle,
            _storage_guard: storage_guard,
        };

        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_id, entry);

        Ok(model_info)
    }

    async fn Shutdown_Session(
        &self,
        session_id: &str,
    ) -> Result<(), ML_Engine_Error> {
        info!("ML_Engine_Service: Shutdown_Session [{}]", session_id);

        // Step 1: 从 sessions 表中移除 SessionEntry
        let mut sessions = self.sessions.lock().await;
        let entry = sessions.remove(session_id).ok_or_else(|| {
            ML_Engine_Error::SessionNotFound(session_id.to_string())
        })?;
        drop(sessions); // 释放锁后再执行 Shutdown（避免持锁等待线程退出）

        // Step 2: 发送 Shutdown 命令
        if let Err(e) = entry.handle.Shutdown().await {
            warn!(
                "ML_Engine_Service: [{}] Shutdown 命令发送失败（线程可能已退出）: {}",
                session_id, e
            );
        }

        // Step 3: entry._storage_guard 在此处 drop → 释放 Storage 读锁
        info!("ML_Engine_Service: [{}] Session 已关闭", session_id);
        Ok(())
    }

    async fn Run_Program_VM(
        &self,
        session_id: &str,
        program: Vec<MlInstruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<Pipeline_Result, ML_Engine_Error> {
        info!(
            "ML_Engine_Service: Run_Program_VM [{}] ({} 条指令)",
            session_id,
            program.len()
        );

        let handle = {
            let sessions = self.sessions.lock().await;
            let entry = sessions.get(session_id).ok_or_else(|| {
                ML_Engine_Error::SessionNotFound(session_id.to_string())
            })?;
            entry.handle.clone()
        };

        let result = handle
            .Run_Program_VM(program, params, cancel_flag)
            .await
            .map_err(|e| {
                ML_Engine_Error::ProgramFailed(format!(
                    "Session [{}]: {}", session_id, e
                ))
            })?;

        info!(
            "ML_Engine_Service: [{}] VM Program 执行完成 (steps={}, tokens={})",
            session_id, result.total_steps, result.generated_tokens.len()
        );

        Ok(result)
    }

    async fn Analyze_Model(
        &self,
        model_file_id: &str,
    ) -> Result<Model_Info, ML_Engine_Error> {
        info!("ML_Engine_Service: Analyze_Model [{}]", model_file_id);

        // Step 1: 通过 Storage 获取临时读锁 + 路径
        let (model_path, _guard) = self
            .storage
            .acquire_read(model_file_id)
            .await
            .map_err(|e| {
                ML_Engine_Error::ModelAnalysisFailed(format!(
                    "Storage acquire_read failed: {}", e
                ))
            })?;

        // Step 2: 在阻塞线程中执行 GGUF_Analyze（同步 I/O）
        let path_clone = model_path.clone();
        let arch_info = tokio::task::spawn_blocking(move || {
            GGUF_Analyze(&path_clone)
        })
        .await
        .map_err(|e| {
            ML_Engine_Error::ModelAnalysisFailed(format!("spawn_blocking failed: {}", e))
        })?
        .map_err(|e| {
            ML_Engine_Error::ModelAnalysisFailed(format!("GGUF_Analyze failed: {}", e))
        })?;

        // Step 3: 转换 Model_Arch_Info → Model_Info
        let layer_sizes_bytes = build_layer_sizes_for_analyze(&arch_info);
        let model_info = Model_Info {
            architecture: arch_info.architecture.clone(),
            num_layers: arch_info.num_layers,
            embedding_length: arch_info.embedding_length,
            has_input_head: !arch_info.is_split || arch_info.split_start == 0,
            has_output_head: !arch_info.is_split
                || arch_info.split_end == arch_info.num_layers + 1,
            has_tokenizer: false, // Analyze 不加载 tokenizer
            eos_token_id: arch_info.eos_token_id,
            layer_sizes_bytes,
            num_kv_heads: arch_info.head_count_kv,
            head_dim: arch_info.head_dim,
            context_length: arch_info.context_length,
            vocab_size: arch_info.vocab_size,
        };

        info!(
            "ML_Engine_Service: Analyze 完成 (arch: {}, layers: {}, split: {})",
            arch_info.architecture, arch_info.num_layers, arch_info.is_split
        );

        // _guard 在此处 drop → 读锁释放
        Ok(model_info)
    }

    async fn Split_Model(
        &self,
        source_file_id: &str,
        start: usize,
        end: usize,
        output_file_id: &str,
    ) -> Result<(), ML_Engine_Error> {
        info!(
            "ML_Engine_Service: Split_Model [{}] layers {}-{} → [{}]",
            source_file_id, start, end, output_file_id
        );

        // Step 1: 获取源文件读锁 + 路径
        let (src_path, _src_guard) = self
            .storage
            .acquire_read(source_file_id)
            .await
            .map_err(|e| {
                ML_Engine_Error::ModelSplitFailed(format!(
                    "Storage acquire_read(source) failed: {}", e
                ))
            })?;

        // Step 2: 获取输出文件写锁 + 路径
        let (out_path, _out_guard) = self
            .storage
            .acquire_write(output_file_id)
            .await
            .map_err(|e| {
                ML_Engine_Error::ModelSplitFailed(format!(
                    "Storage acquire_write(output) failed: {}", e
                ))
            })?;

        // Step 3: 在阻塞线程中执行 GGUF_Split_Model（同步 I/O）
        // 注意: GGUF_Split_Model 将第 4 个参数视为输出目录（非文件路径），
        //       内部自动生成文件名 "{stem}_split_{start}_{end}.pgguf"。
        //       因此传入 out_path 的父目录（Storage base_dir），
        //       使生成的文件落在 Storage 管理的目录中。
        //       调用方的 output_file_id 应与生成的文件名一致。
        let src_clone = src_path.clone();
        let out_dir = out_path.parent().unwrap_or(&out_path).to_path_buf();
        tokio::task::spawn_blocking(move || {
            GGUF_Split_Model(&src_clone, start, end, &out_dir)
        })
        .await
        .map_err(|e| {
            ML_Engine_Error::ModelSplitFailed(format!("spawn_blocking failed: {}", e))
        })?
        .map_err(|e| {
            ML_Engine_Error::ModelSplitFailed(format!("GGUF_Split_Model failed: {}", e))
        })?;

        info!(
            "ML_Engine_Service: Split 完成 [{}] → [{}]",
            source_file_id, output_file_id
        );

        // Guards 在此处 drop → 锁释放
        Ok(())
    }
}

/// 从 Model_Arch_Info 构建 layer_sizes_bytes 数组
fn build_layer_sizes_for_analyze(arch_info: &Model_Arch_Info) -> Vec<usize> {
    let num_layers = arch_info.num_layers;
    let mut sizes = vec![0usize; num_layers + 2];

    for t in &arch_info.non_layer_tensors {
        if t.name == "token_embd.weight" {
            sizes[0] += t.size_bytes;
        }
    }

    for layer in &arch_info.layers {
        if layer.layer_index < num_layers {
            sizes[layer.layer_index + 1] = layer.total_size_bytes;
        }
    }

    for t in &arch_info.non_layer_tensors {
        if t.name == "output_norm.weight" || t.name == "output.weight" {
            sizes[num_layers + 1] += t.size_bytes;
        }
    }

    sizes
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// 辅助：创建 ML_Engine_Service 测试实例
    async fn create_test_service() -> (ML_Engine_Service, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(
            StorageManager::New(temp_dir.path()).await.unwrap(),
        );
        let service = ML_Engine_Service::New(storage);
        (service, temp_dir)
    }

    /// TC-01: 构造 ML_Engine_Service 实例验证
    #[tokio::test]
    async fn test_service_new() {
        let (service, _tmp) = create_test_service().await;
        let sessions = service.sessions.lock().await;
        assert!(sessions.is_empty());
    }

    /// TC-02: Shutdown 不存在的 session 返回 SessionNotFound
    #[tokio::test]
    async fn test_shutdown_nonexistent_session() {
        let (service, _tmp) = create_test_service().await;
        let result = service.Shutdown_Session("nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ML_Engine_Error::SessionNotFound(id) => assert_eq!(id, "nonexistent"),
            other => panic!("expected SessionNotFound, got {:?}", other),
        }
    }

    /// TC-03: Run_Program_VM 不存在的 session 返回 SessionNotFound
    #[tokio::test]
    async fn test_run_program_nonexistent_session() {
        use crate::ml_engine::ml_vm::MlInstruction;
        let (service, _tmp) = create_test_service().await;
        let cancel = Arc::new(AtomicBool::new(false));
        let result = service
            .Run_Program_VM("nonexistent", vec![], Pipeline_Params::default(), cancel)
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ML_Engine_Error::SessionNotFound(id) => assert_eq!(id, "nonexistent"),
            other => panic!("expected SessionNotFound, got {:?}", other),
        }
    }

    /// TC-04: Create_Session 文件不存在返回 SessionCreationFailed
    #[tokio::test]
    async fn test_create_session_file_not_found() {
        let (service, _tmp) = create_test_service().await;
        let config = ML_Session_Config {
            session_id: "sess-001".to_string(),
            model_file_id: "nonexistent.gguf".to_string(),
            layer_start: 0,
            layer_end: 10,
            device: "cpu".to_string(),
            tensor_io: None,
        };
        let (_, input_rx) = tokio::sync::mpsc::channel(8);
        let (output_tx, _) = tokio::sync::mpsc::channel(8);
        let io_handle = IoHandle { input_rx, output_tx };
        let result = service.Create_Session(config, io_handle).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ML_Engine_Error::SessionCreationFailed(msg) => {
                assert!(msg.contains("Storage acquire_read failed"));
            }
            other => panic!("expected SessionCreationFailed, got {:?}", other),
        }
    }

    /// TC-05: Analyze_Model 文件不存在返回 ModelAnalysisFailed
    #[tokio::test]
    async fn test_analyze_model_file_not_found() {
        let (service, _tmp) = create_test_service().await;
        let result = service.Analyze_Model("nonexistent.gguf").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ML_Engine_Error::ModelAnalysisFailed(msg) => {
                assert!(msg.contains("Storage acquire_read failed"));
            }
            other => panic!("expected ModelAnalysisFailed, got {:?}", other),
        }
    }

    /// TC-06: Split_Model 源文件不存在返回 ModelSplitFailed
    #[tokio::test]
    async fn test_split_model_source_not_found() {
        let (service, _tmp) = create_test_service().await;
        let result = service
            .Split_Model("nonexistent.gguf", 0, 10, "output.pgguf")
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ML_Engine_Error::ModelSplitFailed(msg) => {
                assert!(msg.contains("Storage acquire_read(source) failed"));
            }
            other => panic!("expected ModelSplitFailed, got {:?}", other),
        }
    }
}
