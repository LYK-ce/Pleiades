//Presented by KeJi
//Date: 2026-03-12

//! Pleiades 网络层测试模块
//!
//! 测试方案:
//! 1. 在Pleiades_Workspace中创建自己的工作目录，名称为"本机ID+Workspace"
//! 2. 启动监听网络后，等待另一台测试机接入网络
//! 3. 接入网络后，输出对方测试机的id
//! 4. 在工作目录中创建一个名为"本机id+hello.txt"的文件，内容为"本机ID+hello",发送给对方
//! 5. 接收对方发送过来的文件
//!
//! 支持单机多实例测试和局域网多机测试

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

use pleiades::network::{NetworkConfig, NetworkEvent, Node, NodeHandle};
use pleiades::network::protocol::{CommandRequest, FileRequest, FileResponse, PleiadesResponse};

/// 测试配置
pub const TEST_TIMEOUT_SECS: u64 = 60;
pub const WORKSPACE_DIR: &str = "Pleiades_Workspace";

/// 运行网络测试
pub async fn Run_Test() {
    println!("========================================");
    println!("网络层测试开始");
    println!("========================================");

    // 确保主工作目录存在
    let main_workspace = Path::new(WORKSPACE_DIR);
    if !main_workspace.exists() {
        fs::create_dir_all(main_workspace).expect("无法创建主工作目录");
    }

    // 创建网络配置 (LAN模式)
    let network_config = NetworkConfig {
        lan_enabled: true,
        wan_enabled: false,
        transport_protocol: "TCP".to_string(),
        listen_port: 0, // 随机端口
        bootstrap_peers: Vec::new(),
    };

    println!("[配置] LAN模式: {}", network_config.lan_enabled);
    println!("[配置] WAN模式: {}", network_config.wan_enabled);
    println!("[配置] 传输协议: {}", network_config.transport_protocol);

    // 创建事件通道
    let (event_tx, mut event_rx) = mpsc::channel::<NetworkEvent>(100);

    // 初始化网络节点
    println!("[初始化] 正在初始化网络节点...");
    let node_result = Node::Init(network_config, event_tx).await;
    
    match node_result {
        Ok((mut node, handle)) => {
            let local_peer_id = handle.Get_Local_Peer_Id();
            println!("[成功] 本机节点ID: {}", local_peer_id);

            // 步骤1: 创建本机的工作目录 "本机ID+Workspace"
            let local_workspace_name = format!("{}+Workspace", local_peer_id);
            let local_workspace = main_workspace.join(&local_workspace_name);
            if !local_workspace.exists() {
                fs::create_dir_all(&local_workspace)
                    .expect("无法创建本机工作目录");
            }
            println!("[步骤1] 创建本机工作目录: {}", local_workspace.display());

            // 步骤4准备: 在本机工作目录中创建hello文件
            let hello_filename = format!("{}+hello.txt", local_peer_id);
            let hello_filepath = local_workspace.join(&hello_filename);
            let hello_content = format!("{}+hello", local_peer_id);
            
            {
                let mut file = fs::File::create(&hello_filepath)
                    .expect("无法创建hello文件");
                file.write_all(hello_content.as_bytes())
                    .expect("无法写入hello文件");
            }
            println!("[步骤4准备] 创建测试文件: {}", hello_filepath.display());
            println!("[步骤4准备] 文件内容: {}", hello_content);

            // 启动网络服务（在后台任务中）
            let network_task = tokio::spawn(async move {
                if let Err(e) = node.Start().await {
                    println!("[错误] 网络服务错误: {}", e);
                }
            });

            // 步骤2: 启动监听网络后，等待另一台测试机接入网络
            println!("[步骤2] 等待其他节点接入网络 (超时: {}秒)...", TEST_TIMEOUT_SECS);
            println!("[提示] 请在另一个终端或设备上启动另一个测试实例");

            // 等待节点发现和文件交换
            let mut peer_discovered = false;
            let mut remote_peer_id = None;
            let mut file_received = false;
            let mut file_sent = false;
            let mut received_file_content: Option<Vec<u8>> = None;
            let mut received_filename: Option<String> = None;

            // 克隆handle和路径供异步闭包使用
            let handle_for_send = handle.clone();
            let hello_filepath_clone = hello_filepath.clone();
            let local_workspace_clone = local_workspace.clone();
            let sent_content = hello_content.clone();

            let test_result = timeout(
                Duration::from_secs(TEST_TIMEOUT_SECS),
                async {
                    while let Some(event) = event_rx.recv().await {
                        match event {
                            // 步骤3: 接入网络后，输出对方测试机的id
                            NetworkEvent::PeerDiscovered(peer) => {
                                println!("[发现] 发现新节点: {}", peer);
                                if !peer_discovered {
                                    peer_discovered = true;
                                    remote_peer_id = Some(peer);
                                    println!("[步骤3完成] 对方测试机ID = {}", peer);
                                }
                            }
                            NetworkEvent::ConnectionEstablished(peer) => {
                                println!("[连接] 与节点 {} 建立连接", peer);
                                
                                // 步骤4: 发送文件给对方
                                if !file_sent {
                                    println!("[步骤4] 发送文件给对方: {}", hello_filepath_clone.display());
                                    match handle_for_send.Send_File(&peer, &hello_filepath_clone).await {
                                        Ok(_) => {
                                            file_sent = true;
                                            println!("[步骤4完成] 文件发送请求已发送");
                                        }
                                        Err(e) => {
                                            println!("[步骤4错误] 文件发送失败: {}", e);
                                        }
                                    }
                                }
                            }
                            NetworkEvent::ConnectionClosed(peer) => {
                                println!("[断开] 与节点 {} 断开连接", peer);
                            }
                            // 步骤5: 接收对方发送过来的文件
                            NetworkEvent::FileReceived { peer, request, .. } => {
                                println!("[文件] 收到文件请求 from {}", peer);
                                
                                // 根据请求类型处理
                                match &request {
                                    FileRequest::Info { file_id } => {
                                        println!("[文件信息] file_id={}", file_id);
                                    }
                                    FileRequest::Chunk { file_id, offset, length } => {
                                        println!("[文件块] file_id={}, offset={}, length={}", file_id, offset, length);
                                    }
                                    FileRequest::SendFile { filename, data } => {
                                        // 保存接收到的文件到本机工作目录
                                        let save_path = local_workspace_clone.join(&filename);
                                        
                                        match fs::File::create(&save_path) {
                                            Ok(mut file) => {
                                                match file.write_all(&data) {
                                                    Ok(_) => {
                                                        println!("[步骤5] 文件保存成功: {}", save_path.display());
                                                        println!("[步骤5] 文件大小: {} bytes", data.len());
                                                        println!("[步骤5] 文件内容: {}", String::from_utf8_lossy(&data));
                                                        file_received = true;
                                                        received_file_content = Some(data.clone());
                                                        received_filename = Some(filename.clone());
                                                    }
                                                    Err(e) => {
                                                        println!("[步骤5错误] 写入文件失败: {}", e);
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                println!("[步骤5错误] 创建文件失败: {}", e);
                                            }
                                        }
                                    }
                                }
                                
                                println!("[步骤5完成] 处理对方发送的文件请求");
                                
                                // 如果收发都完成，测试结束
                                if file_received && file_sent {
                                    println!("[完成] 网络测试主要流程完成");
                                    break;
                                }
                            }
                            NetworkEvent::CommandReceived { peer, request, .. } => {
                                println!("[命令] 收到命令请求 from {}: {:?}", peer, request);
                                
                                // 如果是Ping命令，可以验证连接
                                match request {
                                    CommandRequest::Ping => {
                                        println!("[Ping] 收到Ping请求，连接验证成功");
                                    }
                                    _ => {}
                                }
                            }
                            NetworkEvent::TensorReceived { peer, request, .. } => {
                                println!("[张量] 收到张量请求 from {}: request_id={}",
                                         peer, request.request_id);
                            }
                            NetworkEvent::RecordFound { key, value } => {
                                println!("[DHT] 记录找到: key={:?}, value_len={}", key, value.len());
                            }
                            NetworkEvent::RecordNotFound { key } => {
                                println!("[DHT] 记录未找到: key={:?}", key);
                            }
                            NetworkEvent::PeerLeft(peer) => {
                                println!("[离开] 节点离开: {}", peer);
                            }
                        }

                        // 检测是否完成所有测试步骤
                        if peer_discovered && file_sent && file_received {
                            println!("[完成] 所有测试步骤完成！");
                            break;
                        }
                        
                        // 如果发送了文件但还没收到，继续等待
                        // 不要break，继续处理事件
                    }
                },
            )
            .await;

            // 文件内容验证
            let content_verified = if let Some(ref content) = received_file_content {
                // 检查接收到的内容是否符合预期格式（对方PeerId+hello）
                let content_str = String::from_utf8_lossy(content);
                content_str.ends_with("+hello")
            } else {
                false
            };

            match test_result {
                Ok(_) => {
                    if peer_discovered {
                        println!("========================================");
                        println!("[结果] 网络测试成功!");
                        println!("  - 步骤1 (创建工作目录): 通过");
                        println!("  - 步骤2 (启动监听): 通过");
                        println!("  - 步骤3 (节点发现): 通过");
                        if let Some(peer) = remote_peer_id {
                            println!("  - 对方ID: {}", peer);
                        }
                        println!("  - 步骤4 (发送文件): {}", if file_sent { "通过" } else { "未执行" });
                        println!("  - 步骤5 (接收文件): {}", if file_received { "通过" } else { "未收到" });
                        
                        // 文件传输验证结果
                        println!("----------------------------------------");
                        println!("[文件传输验证]");
                        if let Some(ref name) = received_filename {
                            println!("  - 接收文件名: {}", name);
                        }
                        if let Some(ref content) = received_file_content {
                            println!("  - 接收文件大小: {} bytes", content.len());
                            println!("  - 接收文件内容: {}", String::from_utf8_lossy(content));
                        }
                        println!("  - 内容格式验证: {}", if content_verified { "通过" } else { "未验证" });
                        println!("========================================");
                    } else {
                        println!("========================================");
                        println!("[结果] 网络测试部分成功");
                        println!("  - 步骤1 (创建工作目录): 通过");
                        println!("  - 步骤2 (启动监听): 通过");
                        println!("  - 步骤3 (节点发现): 未发现其他节点");
                        println!("  - 提示: 请确保另一个测试实例正在运行");
                        println!("========================================");
                    }
                }
                Err(_) => {
                    println!("========================================");
                    println!("[结果] 网络测试超时");
                    println!("  - 步骤1 (创建工作目录): 通过");
                    println!("  - 步骤2 (启动监听): 通过");
                    println!("  - 步骤3 (节点发现): {}", if peer_discovered { "通过" } else { "超时未发现" });
                    println!("  - 步骤4 (发送文件): {}", if file_sent { "通过" } else { "未执行" });
                    println!("  - 步骤5 (接收文件): {}", if file_received { "通过" } else { "未收到" });
                    if file_received {
                        println!("  - 内容格式验证: {}", if content_verified { "通过" } else { "失败" });
                    }
                    println!("  - 提示: 请检查网络配置或启动另一个实例");
                    println!("========================================");
                }
            }

            // 停止网络任务
            network_task.abort();

            // 保留工作目录和测试文件用于验证
            println!("[保留] 本机工作目录: {}", local_workspace.display());
            println!("[保留] 测试文件: {}", hello_filepath.display());
        }
        Err(e) => {
            println!("[错误] 网络节点初始化失败: {}", e);
            println!("[结果] 网络测试失败");
        }
    }

    println!("========================================");
    println!("网络层测试结束");
    println!("========================================");
}

/// 单机多实例测试辅助说明
pub fn Print_Multi_Instance_Guide() {
    println!("========================================");
    println!("单机多实例测试指南");
    println!("========================================");
    println!("");
    println!("每个实例会在 Pleiades_Workspace 目录下创建自己的工作子目录:");
    println!("  - 实例1: Pleiades_Workspace/<PeerId1>+Workspace/");
    println!("  - 实例2: Pleiades_Workspace/<PeerId2>+Workspace/");
    println!("");
    println!("步骤1: 打开第一个终端，运行:");
    println!("  cargo test --test test Test_Network_Only -- --test-threads=1 --nocapture");
    println!("");
    println!("步骤2: 打开第二个终端，运行相同命令:");
    println!("  cargo test --test test Test_Network_Only -- --test-threads=1 --nocapture");
    println!("");
    println!("两个实例会通过mDNS自动发现对方，并进行测试。");
    println!("每个实例的文件操作都在各自的工作子目录中进行。");
    println!("");
    println!("========================================");
}

/// 测试网络配置创建
pub fn Test_Network_Config_Creation() -> bool {
    let config = NetworkConfig {
        lan_enabled: true,
        wan_enabled: false,
        transport_protocol: "TCP".to_string(),
        listen_port: 0,
        bootstrap_peers: Vec::new(),
    };

    config.lan_enabled && !config.wan_enabled && 
    config.transport_protocol == "TCP" && 
    config.listen_port == 0 && 
    config.bootstrap_peers.is_empty()
}

/// 测试工作目录创建
pub fn Test_Workspace_Creation() -> bool {
    let workspace = Path::new(WORKSPACE_DIR);
    if !workspace.exists() {
        if fs::create_dir_all(workspace).is_err() {
            return false;
        }
    }
    workspace.exists()
}

// =====================================
// 文件收发测试模块
// =====================================

/// 文件收发测试配置
pub struct FileTransferTestConfig {
    /// 测试文件名
    pub test_filename: String,
    /// 测试文件内容
    pub test_content: Vec<u8>,
    /// 测试超时时间（秒）
    pub timeout_secs: u64,
}

impl Default for FileTransferTestConfig {
    fn default() -> Self {
        Self {
            test_filename: "test_transfer.txt".to_string(),
            test_content: b"Hello from Pleiades file transfer test!".to_vec(),
            timeout_secs: 60,
        }
    }
}

/// 文件收发测试结果
#[derive(Debug)]
pub struct FileTransferTestResult {
    /// 文件发送是否成功
    pub send_success: bool,
    /// 文件接收是否成功
    pub receive_success: bool,
    /// 文件内容是否一致
    pub content_match: bool,
    /// 发送的文件名
    pub sent_filename: Option<String>,
    /// 接收的文件名
    pub received_filename: Option<String>,
    /// 发送的文件大小
    pub sent_size: usize,
    /// 接收的文件大小
    pub received_size: usize,
    /// 错误信息（如有）
    pub error_message: Option<String>,
}

impl FileTransferTestResult {
    /// 创建成功结果
    pub fn Success(filename: String, size: usize) -> Self {
        Self {
            send_success: true,
            receive_success: true,
            content_match: true,
            sent_filename: Some(filename.clone()),
            received_filename: Some(filename),
            sent_size: size,
            received_size: size,
            error_message: None,
        }
    }

    /// 创建失败结果
    pub fn Failure(error: String) -> Self {
        Self {
            send_success: false,
            receive_success: false,
            content_match: false,
            sent_filename: None,
            received_filename: None,
            sent_size: 0,
            received_size: 0,
            error_message: Some(error),
        }
    }

    /// 检查测试是否全部通过
    pub fn Is_All_Passed(&self) -> bool {
        self.send_success && self.receive_success && self.content_match
    }
}

/// 运行文件收发测试
///
/// 此测试需要两个节点协作完成：
/// - 节点A发送文件给节点B
/// - 节点B接收文件并验证内容
///
/// # Arguments
/// * `config` - 测试配置
///
/// # Returns
/// 测试结果
pub async fn Run_File_Transfer_Test(config: FileTransferTestConfig) -> FileTransferTestResult {
    println!("========================================");
    println!("文件收发测试开始");
    println!("========================================");
    println!("[配置] 测试文件名: {}", config.test_filename);
    println!("[配置] 测试内容大小: {} bytes", config.test_content.len());
    println!("[配置] 超时时间: {} 秒", config.timeout_secs);

    // 确保主工作目录存在
    let main_workspace = Path::new(WORKSPACE_DIR);
    if !main_workspace.exists() {
        if let Err(e) = fs::create_dir_all(main_workspace) {
            return FileTransferTestResult::Failure(format!("无法创建工作目录: {}", e));
        }
    }

    // 创建网络配置 (LAN模式)
    let network_config = NetworkConfig {
        lan_enabled: true,
        wan_enabled: false,
        transport_protocol: "TCP".to_string(),
        listen_port: 0,
        bootstrap_peers: Vec::new(),
    };

    // 创建事件通道
    let (event_tx, mut event_rx) = mpsc::channel::<NetworkEvent>(100);

    // 初始化网络节点
    println!("[初始化] 正在初始化网络节点...");
    let node_result = Node::Init(network_config, event_tx).await;

    match node_result {
        Ok((mut node, handle)) => {
            let local_peer_id = handle.Get_Local_Peer_Id();
            println!("[成功] 本机节点ID: {}", local_peer_id);

            // 创建本机工作目录
            let local_workspace_name = format!("{}+Workspace", local_peer_id);
            let local_workspace = main_workspace.join(&local_workspace_name);
            if !local_workspace.exists() {
                if let Err(e) = fs::create_dir_all(&local_workspace) {
                    return FileTransferTestResult::Failure(format!("无法创建本机工作目录: {}", e));
                }
            }

            // 创建测试文件
            let test_file_path = local_workspace.join(&config.test_filename);
            if let Err(e) = fs::write(&test_file_path, &config.test_content) {
                return FileTransferTestResult::Failure(format!("无法创建测试文件: {}", e));
            }
            println!("[准备] 创建测试文件: {}", test_file_path.display());

            // 启动网络服务
            let network_task = tokio::spawn(async move {
                if let Err(e) = node.Start().await {
                    println!("[错误] 网络服务错误: {}", e);
                }
            });

            println!("[等待] 等待对方节点连接... (超时: {}秒)", config.timeout_secs);

            // 测试状态追踪
            let mut file_sent = false;
            let mut file_received = false;
            let mut received_content: Vec<u8> = Vec::new();
            let mut received_filename: Option<String> = None;

            let handle_for_send = handle.clone();
            let test_file_path_clone = test_file_path.clone();
            let local_workspace_clone = local_workspace.clone();
            let expected_content = config.test_content.clone();

            let test_result = timeout(
                Duration::from_secs(config.timeout_secs),
                async {
                    while let Some(event) = event_rx.recv().await {
                        match event {
                            NetworkEvent::PeerDiscovered(peer) => {
                                println!("[发现] 发现节点: {}", peer);
                            }
                            NetworkEvent::ConnectionEstablished(peer) => {
                                println!("[连接] 与节点 {} 建立连接", peer);
                                
                                // 连接建立后发送测试文件
                                if !file_sent {
                                    println!("[发送] 发送测试文件: {}", test_file_path_clone.display());
                                    match handle_for_send.Send_File(&peer, &test_file_path_clone).await {
                                        Ok(_) => {
                                            file_sent = true;
                                            println!("[发送] 文件发送请求已提交");
                                        }
                                        Err(e) => {
                                            println!("[错误] 文件发送失败: {}", e);
                                        }
                                    }
                                }
                            }
                            NetworkEvent::FileReceived { peer, request, .. } => {
                                println!("[接收] 收到文件请求 from {}", peer);
                                
                                if let FileRequest::SendFile { filename, data } = request {
                                    println!("[接收] 文件名: {}", filename);
                                    println!("[接收] 文件大小: {} bytes", data.len());
                                    
                                    // 保存接收的文件
                                    let save_path = local_workspace_clone.join(&filename);
                                    match fs::write(&save_path, &data) {
                                        Ok(_) => {
                                            println!("[接收] 文件保存成功: {}", save_path.display());
                                            file_received = true;
                                            received_content = data;
                                            received_filename = Some(filename);
                                        }
                                        Err(e) => {
                                            println!("[错误] 文件保存失败: {}", e);
                                        }
                                    }
                                }
                                
                                // 收发都完成后退出
                                if file_sent && file_received {
                                    println!("[完成] 文件收发测试完成");
                                    break;
                                }
                            }
                            NetworkEvent::ConnectionClosed(peer) => {
                                println!("[断开] 与节点 {} 断开连接", peer);
                            }
                            _ => {}
                        }

                        if file_sent && file_received {
                            break;
                        }
                    }
                },
            )
            .await;

            // 停止网络任务
            network_task.abort();

            // 构建测试结果
            let content_match = received_content == expected_content;
            
            match test_result {
                Ok(_) => {
                    println!("========================================");
                    println!("[结果] 文件收发测试完成");
                    println!("  - 文件发送: {}", if file_sent { "成功" } else { "失败" });
                    println!("  - 文件接收: {}", if file_received { "成功" } else { "失败" });
                    println!("  - 内容验证: {}", if content_match { "通过" } else { "失败" });
                    if let Some(ref name) = received_filename {
                        println!("  - 接收文件名: {}", name);
                    }
                    println!("  - 发送大小: {} bytes", config.test_content.len());
                    println!("  - 接收大小: {} bytes", received_content.len());
                    println!("========================================");

                    FileTransferTestResult {
                        send_success: file_sent,
                        receive_success: file_received,
                        content_match,
                        sent_filename: Some(config.test_filename),
                        received_filename,
                        sent_size: config.test_content.len(),
                        received_size: received_content.len(),
                        error_message: None,
                    }
                }
                Err(_) => {
                    println!("========================================");
                    println!("[结果] 文件收发测试超时");
                    println!("  - 文件发送: {}", if file_sent { "成功" } else { "未完成" });
                    println!("  - 文件接收: {}", if file_received { "成功" } else { "未完成" });
                    println!("========================================");

                    FileTransferTestResult {
                        send_success: file_sent,
                        receive_success: file_received,
                        content_match: false,
                        sent_filename: Some(config.test_filename),
                        received_filename,
                        sent_size: config.test_content.len(),
                        received_size: received_content.len(),
                        error_message: Some("测试超时".to_string()),
                    }
                }
            }
        }
        Err(e) => {
            println!("[错误] 网络节点初始化失败: {}", e);
            FileTransferTestResult::Failure(format!("节点初始化失败: {}", e))
        }
    }
}

/// 运行默认配置的文件收发测试
pub async fn Run_File_Transfer_Test_Default() -> FileTransferTestResult {
    Run_File_Transfer_Test(FileTransferTestConfig::default()).await
}

/// 文件收发测试指南
pub fn Print_File_Transfer_Test_Guide() {
    println!("========================================");
    println!("文件收发测试指南");
    println!("========================================");
    println!("");
    println!("此测试验证两个节点之间的文件传输功能:");
    println!("  1. 节点A创建测试文件并发送给节点B");
    println!("  2. 节点B接收文件并保存到本地工作目录");
    println!("  3. 验证接收的文件内容与发送的一致");
    println!("");
    println!("步骤1: 打开第一个终端，运行:");
    println!("  cargo test --test test Test_File_Transfer -- --test-threads=1 --nocapture");
    println!("");
    println!("步骤2: 打开第二个终端，运行相同命令:");
    println!("  cargo test --test test Test_File_Transfer -- --test-threads=1 --nocapture");
    println!("");
    println!("两个节点通过mDNS自动发现后会互相发送测试文件。");
    println!("发送的文件保存在 Pleiades_Workspace/<PeerId>+Workspace/ 目录下。");
    println!("");
    println!("========================================");
}

/// 验证文件传输完整性测试（本地测试，不需要网络）
pub fn Test_File_Content_Integrity() -> bool {
    println!("[测试] 文件内容完整性验证...");
    
    let test_workspace = Path::new(WORKSPACE_DIR).join("integrity_test");
    if !test_workspace.exists() {
        if fs::create_dir_all(&test_workspace).is_err() {
            println!("[失败] 无法创建测试目录");
            return false;
        }
    }

    // 测试数据: 包含各种类型的内容
    let test_cases: Vec<(&str, Vec<u8>)> = vec![
        ("text_file.txt", b"Hello, Pleiades!".to_vec()),
        ("binary_file.bin", vec![0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD]),
        ("unicode_file.txt", "你好，Pleiades！🚀".as_bytes().to_vec()),
        ("large_file.bin", vec![0xAB; 1024]), // 1KB填充
    ];

    let mut all_passed = true;

    for (filename, content) in test_cases {
        let file_path = test_workspace.join(filename);
        
        // 写入文件
        if let Err(e) = fs::write(&file_path, &content) {
            println!("[失败] 无法写入文件 {}: {}", filename, e);
            all_passed = false;
            continue;
        }

        // 读取并验证
        match fs::read(&file_path) {
            Ok(read_content) => {
                if read_content == content {
                    println!("[通过] {} - {} bytes", filename, content.len());
                } else {
                    println!("[失败] {} - 内容不匹配", filename);
                    all_passed = false;
                }
            }
            Err(e) => {
                println!("[失败] 无法读取文件 {}: {}", filename, e);
                all_passed = false;
            }
        }

        // 清理测试文件
        let _ = fs::remove_file(&file_path);
    }

    // 清理测试目录
    let _ = fs::remove_dir(&test_workspace);

    println!("[结果] 文件完整性测试: {}", if all_passed { "全部通过" } else { "存在失败" });
    all_passed
}

/// 测试大文件传输能力（本地模拟）
pub fn Test_Large_File_Handling() -> bool {
    println!("[测试] 大文件处理能力测试...");
    
    let test_workspace = Path::new(WORKSPACE_DIR).join("large_file_test");
    if !test_workspace.exists() {
        if fs::create_dir_all(&test_workspace).is_err() {
            println!("[失败] 无法创建测试目录");
            return false;
        }
    }

    // 测试不同大小的文件
    let sizes: Vec<(usize, &str)> = vec![
        (1024, "1KB"),           // 1 KB
        (1024 * 100, "100KB"),   // 100 KB
        (1024 * 1024, "1MB"),    // 1 MB
        (1024 * 1024 * 5, "5MB"), // 5 MB
    ];

    let mut all_passed = true;

    for (size, label) in sizes {
        let filename = format!("test_{}.bin", label);
        let file_path = test_workspace.join(&filename);
        
        // 生成测试数据
        let content: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
        
        // 写入
        let write_start = std::time::Instant::now();
        if let Err(e) = fs::write(&file_path, &content) {
            println!("[失败] 无法写入 {} 文件: {}", label, e);
            all_passed = false;
            continue;
        }
        let write_time = write_start.elapsed();

        // 读取验证
        let read_start = std::time::Instant::now();
        match fs::read(&file_path) {
            Ok(read_content) => {
                let read_time = read_start.elapsed();
                if read_content == content {
                    println!("[通过] {} - 写入: {:?}, 读取: {:?}",
                             label, write_time, read_time);
                } else {
                    println!("[失败] {} - 内容验证失败", label);
                    all_passed = false;
                }
            }
            Err(e) => {
                println!("[失败] {} - 读取错误: {}", label, e);
                all_passed = false;
            }
        }

        // 清理
        let _ = fs::remove_file(&file_path);
    }

    // 清理测试目录
    let _ = fs::remove_dir(&test_workspace);

    println!("[结果] 大文件处理测试: {}", if all_passed { "全部通过" } else { "存在失败" });
    all_passed
}
