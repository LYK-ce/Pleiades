//Presented by KeJi
//Date: 2026-03-12

//! Pleiades 测试组件 - 主入口
//!
//! 测试总入口，可以通过命令行参数选择进行某一项测试
//! - all: 测试所有
//! - network: 测试网络
//! - runtime: 测试运行时
//! - scheduler: 测试调度器(待实现)
//!
//! 使用方法:
//! ```bash
//! # 运行所有测试 (默认)
//! cargo test --test test -- --test-threads=1 --nocapture
//!
//! # 单机多实例测试：在两个终端分别运行上述命令
//!
//! # 运行Runtime测试 (需要先生成测试数据)
//! python tests/test_models/generate_test_onnx_model.py
//! cargo test --test test Test_Runtime -- --nocapture
//! ```

mod common;

/// 测试入口点 - 运行所有测试
#[tokio::test]
async fn Test_All() {
    println!("========================================");
    println!("Pleiades 测试框架启动");
    println!("测试类型: all");
    println!("========================================");

    // 运行单元测试
    Run_Unit_Tests();
    
    // 运行网络测试
    Run_Network_Test().await;
    
    // 运行Runtime测试
    Run_Runtime_Test();
    
    // Run_Scheduler_Test().await; // 待实现

    println!("========================================");
    println!("Pleiades 测试框架结束");
    println!("========================================");
}

/// 运行单元测试
fn Run_Unit_Tests() {
    println!("[单元测试] 开始...");
    
    // 测试网络配置创建
    let config_test = common::network_test::Test_Network_Config_Creation();
    println!("  - 网络配置创建: {}", if config_test { "通过" } else { "失败" });
    assert!(config_test, "网络配置创建测试失败");
    
    // 测试工作目录创建
    let workspace_test = common::network_test::Test_Workspace_Creation();
    println!("  - 工作目录创建: {}", if workspace_test { "通过" } else { "失败" });
    assert!(workspace_test, "工作目录创建测试失败");
    
    // 测试文件内容完整性
    let integrity_test = common::network_test::Test_File_Content_Integrity();
    println!("  - 文件内容完整性: {}", if integrity_test { "通过" } else { "失败" });
    assert!(integrity_test, "文件内容完整性测试失败");
    
    println!("[单元测试] 完成");
}

/// 运行网络测试
async fn Run_Network_Test() {
    println!("[网络测试] 开始...");
    common::network_test::Run_Test().await;
    println!("[网络测试] 完成");
}

/// 运行Runtime测试
fn Run_Runtime_Test() {
    println!("[Runtime测试] 开始...");
    // 运行加载模型测试
    common::runtime_test::Test_Load_Model_And_Print_EP();
    println!("[Runtime测试] 完成");
}

/// 独立运行网络测试
#[tokio::test]
async fn Test_Network_Only() {
    println!("========================================");
    println!("Pleiades 网络测试");
    println!("========================================");
    
    common::network_test::Run_Test().await;
    
    println!("========================================");
    println!("网络测试结束");
    println!("========================================");
}

/// 单元测试 - 网络配置
#[test]
fn Test_Network_Config() {
    assert!(common::network_test::Test_Network_Config_Creation());
}

/// 单元测试 - 工作目录
#[test]
fn Test_Workspace() {
    assert!(common::network_test::Test_Workspace_Creation());
}

/// 显示测试指南
#[test]
fn Test_Print_Guide() {
    common::network_test::Print_Multi_Instance_Guide();
}

// =====================================
// Runtime测试
// =====================================

/// 独立运行Runtime测试 - 运行所有 runtime 测试
#[test]
fn Test_Runtime_Only() {
    println!("========================================");
    println!("Pleiades Runtime测试");
    println!("========================================");
    
    // 运行加载模型测试
    println!("\n[1/3] 测试加载模型...");
    common::runtime_test::Test_Load_Model_And_Print_EP();
    
    // 运行完整模型测试
    println!("\n[2/3] 测试完整模型推理...");
    common::runtime_test::Test_Full_Model();
    
    // 运行切分模型测试
    println!("\n[3/3] 测试切分模型推理...");
    common::runtime_test::Test_Split_Model();
    
    println!("========================================");
    println!("所有 Runtime 测试完成");
    println!("========================================");
}

/// 运行加载模型并打印Execution Provider测试
#[test]
fn Test_Runtime_Load_Model() {
    common::runtime_test::Test_Load_Model_And_Print_EP();
}

/// 运行完整模型测试
#[test]
fn Test_Runtime_Full_Model() {
    common::runtime_test::Test_Full_Model();
}

/// 运行切分模型测试
#[test]
fn Test_Runtime_Split_Model() {
    common::runtime_test::Test_Split_Model();
}

// =====================================
// 文件收发测试
// =====================================

/// 文件收发测试 - 需要两个节点协作
#[tokio::test]
async fn Test_File_Transfer() {
    println!("========================================");
    println!("Pleiades 文件收发测试");
    println!("========================================");
    
    let result = common::network_test::Run_File_Transfer_Test_Default().await;
    
    println!("========================================");
    println!("文件收发测试结果汇总:");
    println!("  - 发送成功: {}", result.send_success);
    println!("  - 接收成功: {}", result.receive_success);
    println!("  - 内容验证: {}", result.content_match);
    if let Some(ref err) = result.error_message {
        println!("  - 错误信息: {}", err);
    }
    println!("========================================");
    
    // 注意: 不强制assert，因为需要两个节点协作
    // 单节点运行会因超时导致部分失败，这是预期行为
}

/// 文件收发测试 - 使用自定义配置
#[tokio::test]
async fn Test_File_Transfer_Custom() {
    println!("========================================");
    println!("Pleiades 文件收发测试 (自定义配置)");
    println!("========================================");
    
    // 使用更大的测试文件
    let config = common::network_test::FileTransferTestConfig {
        test_filename: "custom_test_file.bin".to_string(),
        test_content: vec![0xAB; 10 * 1024], // 10KB测试文件
        timeout_secs: 90,
    };
    
    let result = common::network_test::Run_File_Transfer_Test(config).await;
    
    println!("测试完成: {}", if result.Is_All_Passed() { "全部通过" } else { "部分失败" });
}

/// 显示文件收发测试指南
#[test]
fn Test_Print_File_Transfer_Guide() {
    common::network_test::Print_File_Transfer_Test_Guide();
}

/// 本地测试 - 文件完整性验证（不需要网络）
#[test]
fn Test_File_Integrity_Local() {
    assert!(common::network_test::Test_File_Content_Integrity());
}

/// 本地测试 - 大文件处理能力（不需要网络）
#[test]
fn Test_Large_File_Local() {
    assert!(common::network_test::Test_Large_File_Handling());
}
