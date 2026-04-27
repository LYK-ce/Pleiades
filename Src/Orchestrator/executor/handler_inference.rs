//Presented by KeJi
//Date ： 2026-04-27

use crate::orchestrator::slot::{SlotId, SlotValue};
use crate::ml_engine::capability::ML_Session_Config;
use super::task_engine::StepResult;

impl super::TaskEngine {
    /// 处理 CreateSession 指令
    ///
    /// 1. 从 `model` 槽位 `get_string` 获取模型路径（作为 `model_file_id`）
    /// 2. 从 `device` 槽位 `get_string` 获取设备字符串（缺失则默认 "cpu"）
    /// 3. 从 `io` 槽位 `take_io_handle` 获取 IoHandle
    /// 4. 构造 `ML_Session_Config`（session_id 由 job_id 生成）
    /// 5. 调用 `capabilities.ml_engine.Create_Session(config, io_handle).await`
    /// 6. 成功 → 将 `session_id` 存入 `result` 槽位（SlotValue::String），返回 Continue
    /// 7. 失败 → 返回 Abort
    pub(super) async fn handle_create_session(
        &mut self,
        model: SlotId,
        device: SlotId,
        io: SlotId,
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

        // 3. 取出 IoHandle（take 语义，消费一次）
        let io_handle = match self.slots.take_io_handle(io) {
            Ok(h) => h,
            Err(e) => return StepResult::Abort(format!("CreateSession: io slot error: {}", e)),
        };

        // 4. 构造 ML_Session_Config
        let session_id = format!("job-{}", self.job_id.0);
        let config = ML_Session_Config {
            session_id: session_id.clone(),
            model_file_id,
            layer_start: 0,
            layer_end: usize::MAX,
            device: device_str,
            tensor_io: None,
        };

        // 5. 调用 ML_Engine_Capability
        match self.capabilities.ml_engine.Create_Session(config, io_handle).await {
            Ok(_model_info) => {
                // 6. 成功：将 session_id 存入 result 槽位
                self.slots.set(result, SlotValue::String(session_id));
                StepResult::Continue
            }
            Err(e) => {
                // 7. 失败：返回 Abort
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
    /// 1. 从 `session` 槽位读取 session_id
    /// 2. 构造 ML 指令序列和参数（由 Compiler 预设到槽位或内置默认）
    /// 3. 调用 `capabilities.ml_engine.Run_Program(session_id, program, params, cancel_flag).await`
    /// 4. 结果写入 `result` 槽位
    pub(super) async fn handle_run_program(&mut self, _session: SlotId, _result: SlotId) -> StepResult {
        // 占位符：Phase 2C/3 实现
        StepResult::Continue
    }

    /// 处理 AnalyzeModel 指令：分析模型文件结构（无需 Session）
    ///
    /// 1. 从 `model` 槽位读取 model_file_id
    /// 2. 调用 `capabilities.ml_engine.Analyze_Model(model_file_id).await`
    /// 3. 将 Model_Info 存入 `result` 槽位
    pub(super) async fn handle_analyze_model(&mut self, _model: SlotId, _result: SlotId) -> StepResult {
        // 占位符：Phase 3 实现
        StepResult::Continue
    }

    /// 处理 SplitModel 指令：切分模型文件（无需 Session）
    ///
    /// 1. 从各槽位读取 source_file_id、start、end、output_file_id
    /// 2. 调用 `capabilities.ml_engine.Split_Model(source, start, end, output).await`
    pub(super) async fn handle_split_model(
        &mut self,
        _source: SlotId,
        _start: SlotId,
        _end: SlotId,
        _output: SlotId,
    ) -> StepResult {
        // 占位符：Phase 3 实现
        StepResult::Continue
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
    use crate::ml_engine::capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
    use crate::ml_engine::ml_thread_engine_instruction::{
        Instruction, Pipeline_Params, Pipeline_Result, Model_Info,
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    // ─── 可配置的 Mock ML Engine ─────────────────────────────

    /// Mock ML Engine，支持配置 CreateSession/ShutdownSession 的成功/失败行为
    struct MockMLEngine {
        create_session_result: tokio::sync::Mutex<Option<Result<Model_Info, ML_Engine_Error>>>,
        shutdown_session_result: tokio::sync::Mutex<Option<Result<(), ML_Engine_Error>>>,
    }

    impl MockMLEngine {
        /// 创建始终成功的 Mock
        fn success() -> Self {
            let model_info = Model_Info {
                architecture: "mock_arch".to_string(),
                num_layers: 12,
                has_input_head: true,
                has_output_head: true,
                has_tokenizer: true,
                eos_token_id: 151643,
            };
            Self {
                create_session_result: tokio::sync::Mutex::new(Some(Ok(model_info))),
                shutdown_session_result: tokio::sync::Mutex::new(Some(Ok(()))),
            }
        }

        /// 创建 CreateSession 失败的 Mock
        fn create_fails(error_msg: &str) -> Self {
            Self {
                create_session_result: tokio::sync::Mutex::new(Some(Err(
                    ML_Engine_Error::SessionCreationFailed(error_msg.to_string()),
                ))),
                shutdown_session_result: tokio::sync::Mutex::new(Some(Ok(()))),
            }
        }

        /// 创建 ShutdownSession 失败的 Mock
        fn shutdown_fails(error_msg: &str) -> Self {
            let model_info = Model_Info {
                architecture: "mock_arch".to_string(),
                num_layers: 12,
                has_input_head: true,
                has_output_head: true,
                has_tokenizer: true,
                eos_token_id: 151643,
            };
            Self {
                create_session_result: tokio::sync::Mutex::new(Some(Ok(model_info))),
                shutdown_session_result: tokio::sync::Mutex::new(Some(Err(
                    ML_Engine_Error::SessionNotFound(error_msg.to_string()),
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
            unimplemented!("mock")
        }

        async fn Split_Model(
            &self,
            _source_file_id: &str,
            _start: usize,
            _end: usize,
            _output_file_id: &str,
        ) -> Result<(), ML_Engine_Error> {
            unimplemented!("mock")
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
        });
        (caps, temp_dir)
    }

    /// 创建测试用 IoHandle
    async fn make_io_handle(caps: &Arc<Capabilities>, job_id: JobId) -> IoHandle {
        let channels = caps.io_broker.Allocate(job_id).await.unwrap();
        channels.ml_side
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
            .handle_create_session(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
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
            .handle_create_session(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
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
            .handle_create_session(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
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
            .handle_create_session(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
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
            .handle_create_session(SlotId(1), SlotId(2), SlotId(3), SlotId(4))
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
}
