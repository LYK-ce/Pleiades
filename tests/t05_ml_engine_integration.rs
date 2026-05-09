//Presented by KeJi
//Date ： 2026-04-27

#![allow(non_snake_case)]

//! ML Engine 模块集成测试
//!
//! 通过 `dyn ML_Engine_Capability` trait 对象验证 ML_Engine_Service 的跨模块集成，
//! 包括完整推理、模型切分和半模型 Session。
//!
//! **前置条件**：需要在 `tests/test_model/` 目录下放置 `Qwen3-0.6B-Q8_0.gguf` 模型文件。
//! 若文件不存在，需要模型的测试将 panic 并输出明确提示。

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tempfile::TempDir;

use pleiades::ml_engine::{
    Inference_Input, Instruction, ML_Engine_Capability, ML_Engine_Error, ML_Engine_Service,
    ML_Session_Config, Pipeline_Params, Set_Target, FLAG1, META1, META2, META5, TENSOR2,
    TOKENID2, TOKENID3,
};
use pleiades::llm_io::IoHandle;
use pleiades::storage::{StorageCapability, StorageManager};

// ─── 常量与辅助函数 ─────────────────────────────────────────

/// 测试模型文件名
const MODEL_FILE_NAME: &str = "Qwen3-0.6B-Q8_0.gguf";

/// 获取测试模型文件的源路径（项目 tests/test_model/ 下）
fn Model_Source_Path() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("tests")
        .join("test_model")
        .join(MODEL_FILE_NAME)
}

/// 确认模型文件存在，否则 panic 并输出明确提示
fn Ensure_Model_Exists() {
    let path = Model_Source_Path();
    if !path.exists() {
        panic!(
            "\n╔══════════════════════════════════════════════════════╗\n\
             ║  测试模型文件不存在！                                  ║\n\
             ║  请将 Qwen3-0.6B-Q8_0.gguf 放置到:                   ║\n\
             ║  tests/test_model/Qwen3-0.6B-Q8_0.gguf               ║\n\
             ╚══════════════════════════════════════════════════════╝\n\
             路径: {}",
            path.display()
        );
    }
}

/// 创建测试环境：空 TempDir + StorageManager + ML_Engine_Service（不含模型文件）
///
/// 用于错误路径测试（TC-01）
async fn Create_Empty_Service() -> (Arc<dyn ML_Engine_Capability>, Arc<StorageManager>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(StorageManager::New(temp_dir.path()).await.unwrap());
    let service = ML_Engine_Service::New(storage.clone());
    let capability: Arc<dyn ML_Engine_Capability> = Arc::new(service);
    (capability, storage, temp_dir)
}

/// 创建测试环境：TempDir + 复制模型 + StorageManager + ML_Engine_Service
///
/// 用于需要真实模型的测试（TC-02 ~ TC-08）
async fn Create_Service_With_Model() -> (Arc<dyn ML_Engine_Capability>, Arc<StorageManager>, TempDir)
{
    Ensure_Model_Exists();
    let temp_dir = TempDir::new().unwrap();

    // 复制模型文件到 TempDir（StorageManager 初始化扫描时会发现）
    let src = Model_Source_Path();
    let dst = temp_dir.path().join(MODEL_FILE_NAME);
    std::fs::copy(&src, &dst).expect("Failed to copy model file to TempDir");

    let storage = Arc::new(StorageManager::New(temp_dir.path()).await.unwrap());
    let service = ML_Engine_Service::New(storage.clone());
    let capability: Arc<dyn ML_Engine_Capability> = Arc::new(service);
    (capability, storage, temp_dir)
}

/// 构造 IoHandle + 前端端点（手动创建通道对）
///
/// 返回 `(io_handle_for_ml_side, input_tx_for_frontend, output_rx_for_frontend)`
fn Create_Io_Channels() -> (
    IoHandle,
    tokio::sync::mpsc::Sender<String>,
    tokio::sync::mpsc::Receiver<String>,
) {
    let (input_tx, input_rx) = tokio::sync::mpsc::channel(64);
    let (output_tx, output_rx) = tokio::sync::mpsc::channel(64);
    let io_handle = IoHandle {
        input_rx,
        output_tx,
    };
    (io_handle, input_tx, output_rx)
}

/// 构造单机推理指令序列
///
/// ```text
/// Input → Encode → Set(META2, max_tokens) → Set(FLAG1, false)
/// → Prefill(TOKENID3) → CopyMeta(META5, META1)
/// → Sample(TENSOR2) → Decode → Output
/// → Loop [ BreakIf, Inference(TOKENID2), Sample(TENSOR2), Decode, Output ]
/// → EndOutput
/// ```
fn Standalone_Inference_Program(max_tokens: usize) -> Vec<Instruction> {
    vec![
        Instruction::Input,
        Instruction::Encode,
        Instruction::Set {
            target: Set_Target::Meta(META2, max_tokens as f64),
        },
        Instruction::Set {
            target: Set_Target::Flag(FLAG1, false),
        },
        Instruction::Prefill { input: TOKENID3 },
        Instruction::CopyMeta {
            src: META5,
            dst: META1,
        },
        Instruction::Sample {
            tensor_reg: TENSOR2,
        },
        Instruction::Decode,
        Instruction::Output,
        Instruction::Loop {
            body: vec![
                Instruction::BreakIf,
                Instruction::Inference {
                    input: Inference_Input::Tokens(TOKENID2),
                },
                Instruction::Sample {
                    tensor_reg: TENSOR2,
                },
                Instruction::Decode,
                Instruction::Output,
            ],
        },
        Instruction::EndOutput,
    ]
}

// ─── TC-01: 错误路径（无需模型） ────────────────────────────

/// TC-01: 错误路径
///
/// 通过 `dyn ML_Engine_Capability` trait 对象验证：
/// - Shutdown 不存在的 session → SessionNotFound
/// - Run_Program 不存在的 session → SessionNotFound
/// - Create_Session 不存在的模型 → SessionCreationFailed + 无 Storage 锁泄漏
#[tokio::test]
async fn tc01_error_paths() {
    let (cap, storage, _tmp) = Create_Empty_Service().await;

    // 1. Shutdown 不存在的 session → SessionNotFound
    let result = cap.Shutdown_Session("nonexistent").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ML_Engine_Error::SessionNotFound(id) => assert_eq!(id, "nonexistent"),
        other => panic!("Expected SessionNotFound, got {:?}", other),
    }

    // 2. Run_Program 不存在的 session → SessionNotFound
    let cancel = Arc::new(AtomicBool::new(false));
    let result = cap
        .Run_Program("nonexistent", vec![], Pipeline_Params::default(), cancel)
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ML_Engine_Error::SessionNotFound(id) => assert_eq!(id, "nonexistent"),
        other => panic!("Expected SessionNotFound, got {:?}", other),
    }

    // 3. Create_Session 不存在的模型 → SessionCreationFailed
    let config = ML_Session_Config {
        session_id: "err-sess".to_string(),
        model_file_id: "nonexistent.gguf".to_string(),
        layer_start: 0,
        layer_end: 29,
        device: "cpu".to_string(),
        tensor_io: None,
    };
    let (io_handle, _input_tx, _output_rx) = Create_Io_Channels();
    let result = cap.Create_Session(config, io_handle).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ML_Engine_Error::SessionCreationFailed(msg) => {
            assert!(
                msg.contains("Storage"),
                "Expected Storage error, got: {}",
                msg
            );
        }
        other => panic!("Expected SessionCreationFailed, got {:?}", other),
    }

    // 4. 验证无 Storage 锁泄漏：索引表应为空
    let list = storage.list().await.unwrap();
    assert!(
        list.is_empty(),
        "No files should be registered after failed Create_Session"
    );
}

// ─── TC-02: Analyze 完整模型 ────────────────────────────────

/// TC-02: Analyze 完整模型
///
/// `Analyze_Model` → `Model_Info` → 验证 architecture、num_layers、head 标志
#[tokio::test]
async fn tc02_analyze_complete_model() {
    let (cap, _storage, _tmp) = Create_Service_With_Model().await;

    let model_info = cap.Analyze_Model(MODEL_FILE_NAME).await.unwrap();

    assert_eq!(
        model_info.architecture, "qwen3",
        "Expected qwen3 architecture"
    );
    assert_eq!(
        model_info.num_layers, 28,
        "Expected 28 transformer blocks for Qwen3-0.6B"
    );
    // 完整模型: has_input_head = !is_split(false) || split_start==0 → true
    assert!(
        model_info.has_input_head,
        "Complete model should have input head"
    );
    // 完整模型: has_output_head = !is_split(false) || split_end==N+1 → true
    assert!(
        model_info.has_output_head,
        "Complete model should have output head"
    );
}

// ─── TC-03: Create + Shutdown 生命周期 ──────────────────────

/// TC-03: Create + Shutdown 生命周期
///
/// - `Create_Session(0, 29)` → `Model_Info`（验证 has_input_head=true, has_output_head=true）
/// - `Shutdown_Session` → 验证 Storage 读锁释放（acquire_write 成功）
#[tokio::test]
async fn tc03_create_shutdown_lifecycle() {
    let (cap, storage, _tmp) = Create_Service_With_Model().await;

    let config = ML_Session_Config {
        session_id: "lifecycle-sess".to_string(),
        model_file_id: MODEL_FILE_NAME.to_string(),
        layer_start: 0,
        layer_end: 29,
        device: "cpu".to_string(),
        tensor_io: None,
    };
    let (io_handle, _input_tx, _output_rx) = Create_Io_Channels();

    // Create session → Model_Info
    let model_info = cap.Create_Session(config, io_handle).await.unwrap();

    // 验证 Model_Info（来自 Session_Thread 的 GGUF_Load_Model）
    assert_eq!(model_info.architecture, "qwen3");
    assert_eq!(model_info.num_layers, 28);
    assert!(
        model_info.has_input_head,
        "Full model (0-29) should have input head"
    );
    assert!(
        model_info.has_output_head,
        "Full model (0-29) should have output head"
    );
    assert!(
        model_info.has_tokenizer,
        "Full model should have tokenizer"
    );

    // Shutdown session → 释放 Storage 读锁
    cap.Shutdown_Session("lifecycle-sess").await.unwrap();

    // 验证 Storage 读锁确已释放：acquire_write 应成功
    let (_path, guard) = storage.acquire_write(MODEL_FILE_NAME).await.unwrap();
    drop(guard);
}

// ─── TC-04: 完整单机推理 ────────────────────────────────────

/// TC-04: 完整单机推理
///
/// Create → IoHandle 通信 → Run_Program（单机指令序列, max_tokens=5）
/// → frontend 收到流式 token → 验证 Pipeline_Result 非空 → Shutdown
#[tokio::test]
async fn tc04_full_standalone_inference() {
    let (cap, _storage, _tmp) = Create_Service_With_Model().await;

    let config = ML_Session_Config {
        session_id: "infer-sess".to_string(),
        model_file_id: MODEL_FILE_NAME.to_string(),
        layer_start: 0,
        layer_end: 29,
        device: "cpu".to_string(),
        tensor_io: None,
    };
    let (io_handle, input_tx, mut output_rx) = Create_Io_Channels();

    // Create session
    let _model_info = cap.Create_Session(config, io_handle).await.unwrap();

    // 构造推理指令序列和参数
    let program = Standalone_Inference_Program(5);
    let params = Pipeline_Params {
        max_tokens: 5,
        temperature: 0.0, // greedy 采样，确保确定性
        seed: 42,
        eos_token_id: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));

    // Spawn Run_Program 任务（在 Session 线程中执行指令序列）
    let cap_clone = cap.clone();
    let run_handle = tokio::spawn(async move {
        cap_clone
            .Run_Program("infer-sess", program, params, cancel)
            .await
    });

    // 从前端侧发送 prompt
    input_tx.send("Hello".to_string()).await.unwrap();
    drop(input_tx); // 不再需要发送更多输入

    // Spawn 输出收集器（从 output_rx 收集流式 token）
    let collector_handle = tokio::spawn(async move {
        let mut received_tokens = Vec::new();
        while let Some(token) = output_rx.recv().await {
            received_tokens.push(token);
        }
        received_tokens
    });

    // 等待 Run_Program 完成（120 秒超时，CPU 推理可能较慢）
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        run_handle,
    )
    .await
    .expect("Run_Program timed out after 120s")
    .expect("Run_Program task panicked")
    .expect("Run_Program returned error");

    // 验证 Pipeline_Result
    assert!(
        !result.generated_tokens.is_empty(),
        "Generated tokens should not be empty"
    );
    assert!(
        !result.prompt_tokens.is_empty(),
        "Prompt tokens should not be empty"
    );
    assert!(
        result.total_steps > 0,
        "Total steps should be positive (got 0)"
    );

    // Shutdown session → 关闭 io_handle.output_tx → collector 收到 None → 结束
    cap.Shutdown_Session("infer-sess").await.unwrap();

    // 收集前端收到的流式 token
    let received_tokens = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        collector_handle,
    )
    .await
    .expect("Collector timed out after 5s")
    .expect("Collector task panicked");

    assert!(
        !received_tokens.is_empty(),
        "Frontend should have received streamed tokens"
    );
}

// ─── TC-05: Split 前半模型 ──────────────────────────────────

/// TC-05: Split 前半模型
///
/// `Split_Model(0, 14)` → `Analyze` 产物 → 验证：
/// - `is_split=true`（通过 has_input_head + has_output_head 间接验证）
/// - `split_start=0` → `has_input_head=true`
/// - `split_end=14` → `has_output_head=false`
/// - `num_layers=28`（保留原值）
#[tokio::test]
async fn tc05_split_front_half() {
    let (cap, _storage, _tmp) = Create_Service_With_Model().await;

    // GGUF_Split_Model 自动生成文件名: {stem}_split_{start}_{end}.pgguf
    // 源文件 stem = "Qwen3-0.6B-Q8_0"
    let output_file = "Qwen3-0.6B-Q8_0_split_0_14.pgguf";

    // 切分 layers 0-14（embedding + blk.0~blk.13）
    // output_file_id 必须与 GGUF_Split_Model 自动生成的文件名一致
    cap.Split_Model(MODEL_FILE_NAME, 0, 14, output_file)
        .await
        .unwrap();

    // 分析切分产物
    let info = cap.Analyze_Model(output_file).await.unwrap();

    assert_eq!(info.architecture, "qwen3");
    assert_eq!(
        info.num_layers, 28,
        "Split file should preserve original block_count"
    );
    // Analyze: has_input_head = !is_split || split_start==0 → true (split_start=0)
    assert!(
        info.has_input_head,
        "Front half (0-14) should have input head"
    );
    // Analyze: has_output_head = !is_split || split_end==29 → false (split_end=14)
    assert!(
        !info.has_output_head,
        "Front half (0-14) should NOT have output head"
    );
}

// ─── TC-06: Split 后半模型 ──────────────────────────────────

/// TC-06: Split 后半模型
///
/// `Split_Model(15, 29)` → `Analyze` 产物 → 验证：
/// - `split_start=15` → `has_input_head=false`
/// - `split_end=29` → `has_output_head=true`
/// - `num_layers=28`（保留原值）
#[tokio::test]
async fn tc06_split_back_half() {
    let (cap, _storage, _tmp) = Create_Service_With_Model().await;

    // GGUF_Split_Model 自动生成文件名: {stem}_split_{start}_{end}.pgguf
    let output_file = "Qwen3-0.6B-Q8_0_split_15_29.pgguf";

    // 切分 layers 15-29（blk.14~blk.27 + output_norm + lm_head）
    cap.Split_Model(MODEL_FILE_NAME, 15, 29, output_file)
        .await
        .unwrap();

    // 分析切分产物
    let info = cap.Analyze_Model(output_file).await.unwrap();

    assert_eq!(info.architecture, "qwen3");
    assert_eq!(
        info.num_layers, 28,
        "Split file should preserve original block_count"
    );
    // Analyze: has_input_head = !is_split || split_start==0 → false (split_start=15)
    assert!(
        !info.has_input_head,
        "Back half (15-29) should NOT have input head"
    );
    // Analyze: has_output_head = !is_split || split_end==29 → true (split_end=29)
    assert!(
        info.has_output_head,
        "Back half (15-29) should have output head"
    );
}

// ─── TC-07: 前半模型 Session ────────────────────────────────

/// TC-07: 前半模型 Session
///
/// 用 TC-05 产物 → `Create_Session(0, 14)` → 验证：
/// - `has_input_head=true`（start==0）
/// - `has_output_head=false`（14 != 29）
/// - `has_tokenizer=false`（split 文件清空了 tokenizer metadata）
/// → `Shutdown_Session`
#[tokio::test]
async fn tc07_front_half_session() {
    let (cap, storage, _tmp) = Create_Service_With_Model().await;

    // GGUF_Split_Model 自动生成文件名
    let split_file = "Qwen3-0.6B-Q8_0_split_0_14.pgguf";

    // 先切分出前半模型
    cap.Split_Model(MODEL_FILE_NAME, 0, 14, split_file)
        .await
        .unwrap();

    // 用切分产物创建 Session
    let config = ML_Session_Config {
        session_id: "front-sess".to_string(),
        model_file_id: split_file.to_string(),
        layer_start: 0,
        layer_end: 14,
        device: "cpu".to_string(),
        tensor_io: None,
    };
    let (io_handle, _input_tx, _output_rx) = Create_Io_Channels();

    let model_info = cap.Create_Session(config, io_handle).await.unwrap();

    // 验证 Model_Info（来自 GGUF_Load_Model）
    // has_input_head = (start==0) = true
    assert!(
        model_info.has_input_head,
        "Front half session should have input head"
    );
    // has_output_head = (end==max_layer_index) = (14==29) = false
    assert!(
        !model_info.has_output_head,
        "Front half session should NOT have output head"
    );
    // 切分文件 tokenizer metadata 已清空
    assert!(
        !model_info.has_tokenizer,
        "Split file should NOT have tokenizer"
    );

    // Shutdown session
    cap.Shutdown_Session("front-sess").await.unwrap();

    // 验证 Storage 读锁释放
    let (_path, guard) = storage.acquire_write(split_file).await.unwrap();
    drop(guard);
}

// ─── TC-08: 并发 Analyze ────────────────────────────────────

/// TC-08: 并发 Analyze
///
/// 5 并发 `Analyze_Model` → 全部成功、结果一致
#[tokio::test]
async fn tc08_concurrent_analyze() {
    let (cap, _storage, _tmp) = Create_Service_With_Model().await;

    let mut handles = Vec::new();
    for _i in 0..5 {
        let cap_clone = cap.clone();
        let file_name = MODEL_FILE_NAME.to_string();
        handles.push(tokio::spawn(async move {
            cap_clone.Analyze_Model(&file_name).await
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await.unwrap().unwrap();
        results.push(result);
    }

    // 验证所有结果一致
    let first = &results[0];
    for (i, info) in results.iter().enumerate().skip(1) {
        assert_eq!(
            info.architecture, first.architecture,
            "Architecture mismatch at concurrent result {}",
            i
        );
        assert_eq!(
            info.num_layers, first.num_layers,
            "num_layers mismatch at concurrent result {}",
            i
        );
        assert_eq!(
            info.has_input_head, first.has_input_head,
            "has_input_head mismatch at concurrent result {}",
            i
        );
        assert_eq!(
            info.has_output_head, first.has_output_head,
            "has_output_head mismatch at concurrent result {}",
            i
        );
        assert_eq!(
            info.eos_token_id, first.eos_token_id,
            "eos_token_id mismatch at concurrent result {}",
            i
        );
    }
}
