//Presented by KeJi
//Date ： 2026-04-27

#![allow(non_snake_case)]

//! D 组：全链路端到端推理集成测试
//!
//! 验证 `UserCommand::Run` → `Core::route_user` → `Compiler::compile_run` → `spawn_job` →
//! `JobExecutor` → `ML_Engine_Service` → `Session_Thread` → 真实推理输出的完整链路。
//! 经过 Core 的 select! 主循环，模拟真实运行环境。
//!
//! **当前状态**：因 Orchestrator 模块重构（Capabilities 结构调整、Tensor_IO_Broker 移除），
//! 本文件需要重写环境构建部分。暂时禁用所有测试用例。
//!
//! **前置条件**：TC-01 需要在 `tests/test_model/` 目录下放置 `Qwen3-0.6B-Q8_0.gguf` 模型文件。

// TODO: 待 Orchestrator 稳定后重写端到端测试
// 原始测试包括:
// - tc01_single_inference_e2e: 单机推理全链路
// - tc02_compile_error_path: 编译错误路径
