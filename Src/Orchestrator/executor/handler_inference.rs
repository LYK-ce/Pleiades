//Presented by KeJi
//Date ： 2026-04-27

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use crate::orchestrator::slot::{SlotId, SlotValue};
use crate::orchestrator::compiler::Compiler;
use crate::ml_engine::capability::ML_Session_Config;
use crate::ml_engine::ml_thread_engine_instruction::Pipeline_Params;
use super::task_engine::StepResult;

impl super::TaskEngine {
    /// 处理 CreateSession 指令
    ///
    /// 1. 从 `model` 槽位 `get_string` 获取模型路径（作为 `model_file_id`）
    /// 2. 从 `device` 槽位 `get_string` 获取设备字符串（缺失则默认 "cpu"）
    /// 3. 从 `start` 槽位 `get_u64` 获取 layer_start（缺失则默认 0）
    /// 4. 从 `end` 槽位 `get_u64` 获取 layer_end（缺失则默认 usize::MAX）
    /// 5. 从 `io` 槽位 `take_io_handle` 获取 IoHandle
    /// 6. 构造 `ML_Session_Config`（session_id 由 job_id 生成）
    /// 7. 调用 `capabilities.ml_engine.Create_Session(config, io_handle).await`
    /// 8. 成功 → 将 `session_id` 存入 `result` 槽位（SlotValue::String），返回 Continue
    /// 9. 失败 → 返回 Abort
    pub(super) async fn handle_create_session(
        &mut self,
        model: SlotId,
        device: SlotId,
        start: SlotId,
        end: SlotId,
        io: SlotId,
        tensor_io: Option<SlotId>,
        result: SlotId,
    ) -> StepResult {
        // 1. 获取模型路径
        let model_file_id = match self.slots.get_string(model) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("CreateSession: model slot error: {}", e)),
        };

        // 2. 获取设备字符串（允许缺失，默认 "cpu"）
        let device_str = match self.slots.get_string(device) {
            Ok(s) => s.clone(),
            Err(_) => "cpu".to_string(),
        };

        // 3. 获取 layer_start（允许缺失，默认 0）
        let layer_start = match self.slots.get_u64(start) {
            Ok(v) => v as usize,
            Err(_) => 0,
        };

        // 4. 获取 layer_end（允许缺失，默认 usize::MAX）
        let layer_end = match self.slots.get_u64(end) {
            Ok(v) => v as usize,
            Err(_) => usize::MAX,
        };

        // 5. 取出 IoHandle（take 语义，消费一次）
        let io_handle = match self.slots.take_io_handle(io) {
            Ok(h) => h,
            Err(e) => return StepResult::Abort(format!("CreateSession: io slot error: {}", e)),
        };

        // 6. 如果有 tensor_io 槽位，take Tensor_IO_Handle
        let tensor_io_handle = match tensor_io {
            Some(slot) => {
                match self.slots.take_tensor_io(slot) {
                    Ok(h) => Some(h),
                    Err(e) => return StepResult::Abort(format!("CreateSession: tensor_io slot error: {}", e)),
                }
            }
            None => None,
        };

        // 7. 构造 ML_Session_Config
        let session_id = format!("job-{}", self.job_id.0);
        let config = ML_Session_Config {
            session_id: session_id.clone(),
            model_file_id,
            layer_start,
            layer_end,
            device: device_str,
            tensor_io: tensor_io_handle,
        };

        // 8. 调用 ML_Engine_Capability
        match self.capabilities.ml_engine.Create_Session(config, io_handle).await {
            Ok(_model_info) => {
                // 9. 成功：将 session_id 存入 result 槽位
                self.slots.set(result, SlotValue::String(session_id));
                StepResult::Continue
            }
            Err(e) => {
                // 10. 失败：返回 Abort
                StepResult::Abort(format!("CreateSession failed: {}", e))
            }
        }
    }

    /// 处理 ShutdownSession 指令
    ///
    /// 1. 从 `session` 槽位 `get_string` 获取 session_id
    /// 2. 调用 `capabilities.ml_engine.Shutdown_Session(&session_id).await`
    /// 3. 返回 Continue（即使失败也尽力清理，不 Abort）
    pub(super) async fn handle_shutdown_session(&mut self, session: SlotId) -> StepResult {
        // 1. 获取 session_id（尽力清理，读不到也不 Abort）
        let session_id = match self.slots.get_string(session) {
            Ok(s) => s.clone(),
            Err(_) => {
                // 槽位为空或类型不匹配，无需清理，直接 Continue
                return StepResult::Continue;
            }
        };

        // 2. 调用 Shutdown_Session（尽力清理，忽略错误）
        let _ = self.capabilities.ml_engine.Shutdown_Session(&session_id).await;

        // 3. 始终返回 Continue
        StepResult::Continue
    }

    /// 处理 RunProgram 指令：向 Session 提交 ML 指令序列执行推理
    ///
    /// 1. 从 `session` 槽位 `get_string` 获取 session_id
    /// 2. 构造 ML 指令序列（通过 `Compiler::build_run_ml_program`）
    /// 3. 构造 `Pipeline_Params`（使用默认值）
    /// 4. 调用 `capabilities.ml_engine.Run_Program(session_id, program, params, cancel_flag).await`
    /// 5. 成功 → 将完成标记写入 `result` 槽位，返回 Continue
    /// 6. 失败 → 返回 Abort
    pub(super) async fn handle_run_program(&mut self, session: SlotId, result: SlotId) -> StepResult {
        // 1. 获取 session_id
        let session_id = match self.slots.get_string(session) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("RunProgram: session slot error: {}", e)),
        };

        // 2. 构造 ML 指令序列（单机推理）
        let params = Pipeline_Params::default();
        let program = Compiler::build_run_ml_program(&params);

        // 3. 构造 cancel_flag（当前使用非取消标志，后续可接入 CancellationToken）
        let cancel_flag = Arc::new(AtomicBool::new(false));

        // 4. 调用 ML_Engine_Capability::Run_Program
        match self.capabilities.ml_engine.Run_Program(
            &session_id,
            program,
            params,
            cancel_flag,
        ).await {
            Ok(_pipeline_result) => {
                // 5. 成功：写入完成标记
                self.slots.set(result, SlotValue::String("done".to_string()));
                StepResult::Continue
            }
            Err(e) => {
                // 6. 失败：返回 Abort
                StepResult::Abort(format!("RunProgram failed: {}", e))
            }
        }
    }

    /// 处理 AnalyzeModel 指令：分析模型文件结构（无需 Session）
    ///
    /// 1. 从 `model` 槽位 `get_string` 获取 model_file_id
    /// 2. 调用 `capabilities.ml_engine.Analyze_Model(&model_file_id).await`
    /// 3. 成功 → 将 Model_Info 存入 `result` 槽位（SlotValue::ModelInfo）→ Continue
    /// 4. 失败 → Abort
    pub(super) async fn handle_analyze_model(&mut self, model: SlotId, result: SlotId) -> StepResult {
        // 1. 获取 model_file_id
        let model_file_id = match self.slots.get_string(model) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("AnalyzeModel: model slot error: {}", e)),
        };

        // 2. 调用 ML_Engine_Capability::Analyze_Model
        match self.capabilities.ml_engine.Analyze_Model(&model_file_id).await {
            Ok(model_info) => {
                // 3. 成功：将 Model_Info 存入 result 槽位
                self.slots.set(result, SlotValue::ModelInfo(model_info));
                StepResult::Continue
            }
            Err(e) => {
                // 4. 失败：返回 Abort
                StepResult::Abort(format!("AnalyzeModel failed: {}", e))
            }
        }
    }

    /// 处理 SplitModel 指令：切分模型文件（无需 Session）
    ///
    /// 1. 从 `source` 槽位 `get_string` 获取 source_file_id
    /// 2. 从 `start` 槽位 `get_u64` 获取 layer_start
    /// 3. 从 `end` 槽位 `get_u64` 获取 layer_end
    /// 4. 从 `output` 槽位 `get_string` 获取 output_file_id
    /// 5. 调用 `capabilities.ml_engine.Split_Model(&source, start, end, &output).await`
    /// 6. 成功 → Continue
    /// 7. 失败 → Abort
    pub(super) async fn handle_split_model(
        &mut self,
        source: SlotId,
        start: SlotId,
        end: SlotId,
        output: SlotId,
    ) -> StepResult {
        // 1. 获取 source_file_id
        let source_file_id = match self.slots.get_string(source) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("SplitModel: source slot error: {}", e)),
        };

        // 2. 获取 layer_start
        let layer_start = match self.slots.get_u64(start) {
            Ok(v) => v as usize,
            Err(e) => return StepResult::Abort(format!("SplitModel: start slot error: {}", e)),
        };

        // 3. 获取 layer_end
        let layer_end = match self.slots.get_u64(end) {
            Ok(v) => v as usize,
            Err(e) => return StepResult::Abort(format!("SplitModel: end slot error: {}", e)),
        };

        // 4. 获取 output_file_id
        let output_file_id = match self.slots.get_string(output) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("SplitModel: output slot error: {}", e)),
        };

        // 5. 调用 ML_Engine_Capability::Split_Model
        match self.capabilities.ml_engine.Split_Model(
            &source_file_id,
            layer_start,
            layer_end,
            &output_file_id,
        ).await {
            Ok(()) => {
                // 6. 成功
                StepResult::Continue
            }
            Err(e) => {
                // 7. 失败：返回 Abort
                StepResult::Abort(format!("SplitModel failed: {}", e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::task_engine::{TaskEngine, StepResult};
    use super::super::Capabilities;
    use crate::orchestrator::job::JobId;
    use crate::orchestrator::slot::{SlotId, SlotValue};
    use crate::orchestrator::UiCapability;
    use crate::orchestrator::test_utils::StubNetwork;
    use crate::storage::StorageManager;
    use crate::llm_io::{LLM_IO_Broker, LLM_IO_Capability, IoHandle};
    use crate::orchestrator::tensor_io_broker::Tensor_IO_Broker;
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::ml_thread_engine_instruction::{
        Instruction, Pipeline_Params, Pipeline_Result, Model_Info,
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    // ─── 可配置的 Mock ML Engine ─────────────────────────────

    /// Mock ML Engine，支持配置各方法的成功/失败行为
    struct MockMLEngine {
        create_session_result: tokio::sync::Mutex<Option<Result<Model_Info, ML_Engine_Error>>>,
        shutdown_session_result: tokio::sync::Mutex<Option<Result<(), ML_Engine_Error>>>,
        analyze_model_result: tokio::sync::Mutex<Option<Result<Model_Info, ML_Engine_Error>>>,
        split_model_result: tokio::sync::Mutex<Option<Result<(), ML_Engine_Error>>>,
    }

    /// 构造标准的 mock Model_Info
    fn mock_model_info() -> Model_Info {
        Model_Info {
            architecture: "mock_arch".to_string(),
            num_layers: 12,
            has_input_head: true,
            has_output_head: true,
            has_tokenizer: true,
            eos_token_id: 151643,
        }
    }

    impl MockMLEngine {
        /// 创建始终成功的 Mock（所有方法均成功）
        fn success() -> Self {
            Self {
                create_session_result: tokio::sync::Mutex::new(Some(Ok(mock_model_info()))),
                shutdown_session_result: tokio::sync::Mutex::new(Some(Ok(()))),
                analyze_model_result: tokio::sync::Mutex::new(Some(Ok(mock_model_info()))),
                split_model_result: tokio::sync::Mutex::new(Some(Ok(()))),
            }
        }

        /// 创建 CreateSession 失败的 Mock
        fn create_fails(error_msg: &str) -> Self {
            Self {
                create_session_result: tokio::sync::Mutex::new(Some(Err(
                    ML_Engine_Error::SessionCreationFailed(error_msg.to_string()),
                ))),
                shutdown_session_result: tokio::sync::Mutex::new(Some(Ok(()))),
                analyze_model_result: tokio::sync::Mutex::new(Some(Ok(mock_model_info()))),
                split_model_result: tokio::sync::Mutex::new(Some(Ok(()))),
            }
        }

        /// 创建 ShutdownSession 失败的 Mock
        fn shutdown_fails(error_msg: &str) -> Self {
            Self {
                create_session_result: tokio::sync::Mutex::new(Some(Ok(mock_model_info()))),
                shutdown_session_result: tokio::sync::Mutex::new(Some(Err(
                    ML_Engine_Error::SessionNotFound(error_msg.to_string()),
                ))),
                analyze_model_result: tokio::sync::Mutex::new(Some(Ok(mock_model_info()))),
                split_model_result: tokio::sync::Mutex::new(Some(Ok(()))),
            }
        }

        /// 创建 AnalyzeModel 失败的 Mock
        fn analyze_fails(error_msg: &str) -> Self {
            Self {
                create_session_result: tokio::sync::Mutex::new(Some(Ok(mock_model_info()))),
                shutdown_session_result: tokio::sync::Mutex::new(Some(Ok(()))),
                analyze_model_result: tokio::sync::Mutex::new(Some(Err(
                    ML_Engine_Error::ModelAnalysisFailed(error_msg.to_string()),
                ))),
                split_model_result: tokio::sync::Mutex::new(Some(Ok(()))),
            }
        }

        /// 创建 SplitModel 失败的 Mock
        fn split_fails(error_msg: &str) -> Self {
            Self {
                create_session_result: tokio::sync::Mutex::new(Some(Ok(mock_model_info()))),
                shutdown_session_result: tokio::sync::Mutex::new(Some(Ok(()))),
                analyze_model_result: tokio::sync::Mutex::new(Some(Ok(mock_model_info()))),
                split_model_result: tokio::sync::Mutex::new(Some(Err(
                    ML_Engine_Error::ModelSplitFailed(error_msg.to_string()),
                ))),
            }
        }
    }

    #[async_trait]
    impl ML_Engine_Capability for MockMLEngine {
        async fn Create_Session(
            &self,
            _config: ML_Session_Config,
            _io_handle: IoHandle,
        ) -> Result<Model_Info, ML_Engine_Error> {
            self.create_session_result
                .lock()
                .await
                .take()
                .unwrap_or(Err(ML_Engine_Error::SessionCreationFailed(
                    "mock exhausted".to_string(),
                )))
        }

        async fn Shutdown_Session(
            &self,
            _session_id: &str,
        ) -> Result<(), ML_Engine_Error> {
            self.shutdown_session_result
                .lock()
                .await
                .take()
                .unwrap_or(Err(ML_Engine_Error::SessionNotFound(
                    "mock exhausted".to_string(),
                )))
        }

        async fn Run_Program(
            &self,
            _session_id: &str,
            _program: Vec<Instruction>,
            _params: Pipeline_Params,
            _cancel_flag: Arc<AtomicBool>,
        ) -> Result<Pipeline_Result, ML_Engine_Error> {
            unimplemented!("mock")
        }

        async fn Analyze_Model(
            &self,
            _model_file_id: &str,
        ) -> Result<Model_Info, ML_Engine_Error> {
            self.analyze_model_result
                .lock()
                .await
                .take()
                .unwrap_or(Err(ML_Engine_Error::ModelAnalysisFailed(
                    "mock exhausted".to_string(),
                )))
        }

        async fn Split_Model(
            &self,
            _source_file_id: &str,
            _start: usize,
            _end: usize,
            _output_file_id: &str,
        ) -> Result<(), ML_Engine_Error> {
            self.split_model_result
                .lock()
                .await
                .take()
                .unwrap_or(Err(ML_Engine_Error::ModelSplitFailed(
                    "mock exhausted".to_string(),
                )))
        }
    }

    // ─── 辅助函数 ─────────────────────────────────────────────

    async fn make_caps(ml_engine: impl ML_Engine_Capability + 'static) -> (Arc<Capabilities>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::New(temp_dir.path()).await.unwrap();
        let caps = Arc::new(Capabilities {
            storage,
            ml_engine: Box::new(ml_engine),
            network: Box::new(StubNetwork),
            ui: UiCapability,
            io_broker: LLM_IO_Broker::New(),
            tensor_io_broker: Tensor_IO_Broker::New(),
        });
        (caps, temp_dir)
    }

    /// 创建测试用 IoHandle
    async fn make_io_handle(caps: &Arc<Capabilities>, job_id: JobId) -> IoHandle {
        caps.io_broker.Allocate(job_id).await.unwrap();
        caps.io_broker.Take_ML_Side(job_id).await.unwrap()
    }

    // ─── CreateSession 测试 ──────────────────────────────────

    /// TC-01: CreateSession 正常创建
    /// 验证：session_id 写入 result 槽位，返回 Continue
    #[tokio::test]
    async fn tc01_create_session_success() {
        let job_id = JobId(100);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        // 设置 model 槽位
        engine.slots.set(SlotId(1), SlotValue::String("test_model.gguf".to_string()));
        // 设置 device 槽位
        engine.slots.set(SlotId(2), SlotValue::String("cpu".to_string()));
        // 设置 io 槽位
        let io_handle = make_io_handle(&caps, job_id).await;
        engine.slots.set(SlotId(3), SlotValue::IoHandle(io_handle));

        let result = engine
            .handle_create_session(SlotId(1), SlotId(2), SlotId(5), SlotId(6), SlotId(3), None, SlotId(4))
            .await;

        // 验证返回 Continue
        assert!(matches!(result, StepResult::Continue));

        // 验证 result 槽位存储了 session_id
        let session_id = engine.slots.get_string(SlotId(4)).unwrap();
        assert_eq!(session_id, "job-100");
    }

    /// TC-02: CreateSession 模型路径缺失
    /// 验证：返回 Abort，result 槽位为空
    #[tokio::test]
    async fn tc02_create_session_model_missing() {
        let job_id = JobId(101);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        // model 槽位为空（未设置）
        // 设置 device 和 io 槽位
        engine.slots.set(SlotId(2), SlotValue::String("cpu".to_string()));
        let io_handle = make_io_handle(&caps, job_id).await;
        engine.slots.set(SlotId(3), SlotValue::IoHandle(io_handle));

        let result = engine
            .handle_create_session(SlotId(1), SlotId(2), SlotId(5), SlotId(6), SlotId(3), None, SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("model slot error")));
        // result 槽位应为空
        assert!(engine.slots.get(SlotId(4)).is_none());
    }

    /// TC-03: CreateSession IoHandle 缺失
    /// 验证：返回 Abort
    #[tokio::test]
    async fn tc03_create_session_io_missing() {
        let job_id = JobId(102);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("test_model.gguf".to_string()));
        engine.slots.set(SlotId(2), SlotValue::String("cpu".to_string()));
        // io 槽位为空

        let result = engine
            .handle_create_session(SlotId(1), SlotId(2), SlotId(5), SlotId(6), SlotId(3), None, SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("io slot error")));
    }

    /// TC-04: CreateSession ML Engine 返回错误
    /// 验证：返回 Abort，携带错误信息
    #[tokio::test]
    async fn tc04_create_session_ml_engine_error() {
        let job_id = JobId(103);
        let (caps, _temp_dir) = make_caps(MockMLEngine::create_fails("model not found")).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("bad_model.gguf".to_string()));
        engine.slots.set(SlotId(2), SlotValue::String("cpu".to_string()));
        let io_handle = make_io_handle(&caps, job_id).await;
        engine.slots.set(SlotId(3), SlotValue::IoHandle(io_handle));

        let result = engine
            .handle_create_session(SlotId(1), SlotId(2), SlotId(5), SlotId(6), SlotId(3), None, SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("CreateSession failed")));
        // result 槽位应为空
        assert!(engine.slots.get(SlotId(4)).is_none());
    }

    /// TC-05: CreateSession 设备槽位缺失时默认 "cpu"
    /// 验证：即使 device 槽位为空，仍能成功创建（使用默认值）
    #[tokio::test]
    async fn tc05_create_session_device_default() {
        let job_id = JobId(104);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("test_model.gguf".to_string()));
        // device 槽位为空（不设置）
        let io_handle = make_io_handle(&caps, job_id).await;
        engine.slots.set(SlotId(3), SlotValue::IoHandle(io_handle));

        let result = engine
            .handle_create_session(SlotId(1), SlotId(2), SlotId(5), SlotId(6), SlotId(3), None, SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Continue));
        let session_id = engine.slots.get_string(SlotId(4)).unwrap();
        assert_eq!(session_id, "job-104");
    }

    // ─── ShutdownSession 测试 ────────────────────────────────

    /// TC-06: ShutdownSession 正常关闭
    /// 验证：返回 Continue
    #[tokio::test]
    async fn tc06_shutdown_session_success() {
        let job_id = JobId(200);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, caps);

        // 模拟 session_id 已写入槽位
        engine.slots.set(SlotId(10), SlotValue::String("job-200".to_string()));

        let result = engine.handle_shutdown_session(SlotId(10)).await;
        assert!(matches!(result, StepResult::Continue));
    }

    /// TC-07: ShutdownSession 槽位为空
    /// 验证：返回 Continue（尽力清理，无需 Abort）
    #[tokio::test]
    async fn tc07_shutdown_session_empty_slot() {
        let job_id = JobId(201);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, caps);

        // 槽位为空
        let result = engine.handle_shutdown_session(SlotId(10)).await;
        assert!(matches!(result, StepResult::Continue));
    }

    /// TC-08: ShutdownSession ML Engine 返回错误
    /// 验证：仍返回 Continue（尽力清理，忽略错误）
    #[tokio::test]
    async fn tc08_shutdown_session_ml_engine_error() {
        let job_id = JobId(202);
        let (caps, _temp_dir) = make_caps(MockMLEngine::shutdown_fails("session not found")).await;
        let mut engine = TaskEngine::new(job_id, caps);

        engine.slots.set(SlotId(10), SlotValue::String("nonexistent-session".to_string()));

        let result = engine.handle_shutdown_session(SlotId(10)).await;
        assert!(matches!(result, StepResult::Continue));
    }

    // ─── AnalyzeModel 测试 ────────────────────────────────────

    /// TC-09: AnalyzeModel 正常分析
    /// 验证：Model_Info 写入 result 槽位，返回 Continue
    #[tokio::test]
    async fn tc09_analyze_model_success() {
        let job_id = JobId(300);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        // 设置 model 槽位
        engine.slots.set(SlotId(1), SlotValue::String("test_model.gguf".to_string()));

        let result = engine
            .handle_analyze_model(SlotId(1), SlotId(2))
            .await;

        // 验证返回 Continue
        assert!(matches!(result, StepResult::Continue));

        // 验证 result 槽位存储了 ModelInfo
        let model_info = engine.slots.get_model_info(SlotId(2)).unwrap();
        assert_eq!(model_info.architecture, "mock_arch");
        assert_eq!(model_info.num_layers, 12);
        assert!(model_info.has_input_head);
        assert!(model_info.has_output_head);
        assert!(model_info.has_tokenizer);
        assert_eq!(model_info.eos_token_id, 151643);
    }

    /// TC-10: AnalyzeModel 模型路径缺失
    /// 验证：返回 Abort，result 槽位为空
    #[tokio::test]
    async fn tc10_analyze_model_missing() {
        let job_id = JobId(301);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        // model 槽位为空（未设置）
        let result = engine
            .handle_analyze_model(SlotId(1), SlotId(2))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("model slot error")));
        // result 槽位应为空
        assert!(engine.slots.get(SlotId(2)).is_none());
    }

    /// TC-11: AnalyzeModel ML Engine 返回错误
    /// 验证：返回 Abort，携带错误信息
    #[tokio::test]
    async fn tc11_analyze_model_ml_engine_error() {
        let job_id = JobId(302);
        let (caps, _temp_dir) = make_caps(MockMLEngine::analyze_fails("bad format")).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("bad_model.gguf".to_string()));

        let result = engine
            .handle_analyze_model(SlotId(1), SlotId(2))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("AnalyzeModel failed")));
        // result 槽位应为空
        assert!(engine.slots.get(SlotId(2)).is_none());
    }

    // ─── SplitModel 测试 ──────────────────────────────────────

    /// TC-12: SplitModel 正常切分
    /// 验证：返回 Continue
    #[tokio::test]
    async fn tc12_split_model_success() {
        let job_id = JobId(400);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        // 设置所有输入槽位
        engine.slots.set(SlotId(1), SlotValue::String("source_model.gguf".to_string()));
        engine.slots.set(SlotId(2), SlotValue::U64(0));
        engine.slots.set(SlotId(3), SlotValue::U64(14));
        engine.slots.set(SlotId(4), SlotValue::String("split_front.gguf".to_string()));

        let result = engine
            .handle_split_model(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Continue));
    }

    /// TC-13: SplitModel 源文件槽位缺失
    /// 验证：返回 Abort
    #[tokio::test]
    async fn tc13_split_model_source_missing() {
        let job_id = JobId(401);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        // source 槽位为空
        engine.slots.set(SlotId(2), SlotValue::U64(0));
        engine.slots.set(SlotId(3), SlotValue::U64(14));
        engine.slots.set(SlotId(4), SlotValue::String("split_front.gguf".to_string()));

        let result = engine
            .handle_split_model(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("source slot error")));
    }

    /// TC-14: SplitModel start 槽位缺失
    /// 验证：返回 Abort
    #[tokio::test]
    async fn tc14_split_model_start_missing() {
        let job_id = JobId(402);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("source_model.gguf".to_string()));
        // start 槽位为空
        engine.slots.set(SlotId(3), SlotValue::U64(14));
        engine.slots.set(SlotId(4), SlotValue::String("split_front.gguf".to_string()));

        let result = engine
            .handle_split_model(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("start slot error")));
    }

    /// TC-15: SplitModel end 槽位缺失
    /// 验证：返回 Abort
    #[tokio::test]
    async fn tc15_split_model_end_missing() {
        let job_id = JobId(403);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("source_model.gguf".to_string()));
        engine.slots.set(SlotId(2), SlotValue::U64(0));
        // end 槽位为空
        engine.slots.set(SlotId(4), SlotValue::String("split_front.gguf".to_string()));

        let result = engine
            .handle_split_model(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("end slot error")));
    }

    /// TC-16: SplitModel output 槽位缺失
    /// 验证：返回 Abort
    #[tokio::test]
    async fn tc16_split_model_output_missing() {
        let job_id = JobId(404);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("source_model.gguf".to_string()));
        engine.slots.set(SlotId(2), SlotValue::U64(0));
        engine.slots.set(SlotId(3), SlotValue::U64(14));
        // output 槽位为空

        let result = engine
            .handle_split_model(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("output slot error")));
    }

    /// TC-17: SplitModel ML Engine 返回错误
    /// 验证：返回 Abort，携带错误信息
    #[tokio::test]
    async fn tc17_split_model_ml_engine_error() {
        let job_id = JobId(405);
        let (caps, _temp_dir) = make_caps(MockMLEngine::split_fails("io error")).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("source_model.gguf".to_string()));
        engine.slots.set(SlotId(2), SlotValue::U64(0));
        engine.slots.set(SlotId(3), SlotValue::U64(14));
        engine.slots.set(SlotId(4), SlotValue::String("split_front.gguf".to_string()));

        let result = engine
            .handle_split_model(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("SplitModel failed")));
    }

    /// TC-18: SplitModel start 槽位类型不匹配
    /// 验证：返回 Abort（String 不是 U64）
    #[tokio::test]
    async fn tc18_split_model_start_type_mismatch() {
        let job_id = JobId(406);
        let (caps, _temp_dir) = make_caps(MockMLEngine::success()).await;
        let mut engine = TaskEngine::new(job_id, Arc::clone(&caps));

        engine.slots.set(SlotId(1), SlotValue::String("source_model.gguf".to_string()));
        // start 槽位为 String 而非 U64
        engine.slots.set(SlotId(2), SlotValue::String("not_a_number".to_string()));
        engine.slots.set(SlotId(3), SlotValue::U64(14));
        engine.slots.set(SlotId(4), SlotValue::String("split_front.gguf".to_string()));

        let result = engine
            .handle_split_model(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
            .await;

        assert!(matches!(result, StepResult::Abort(ref s) if s.contains("start slot error")));
    }
}
