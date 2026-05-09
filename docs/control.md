#Presented by KeJi
#Date ： 2026-04-07

# Control 层设计文档

## 1. 整体架构

```
┌─────────────────────────────────────────────────┐
│                  用户接口层                       │
│   cli.rs: 读取 "run model_path prompt" 命令      │
│   通过 cli_tx channel 发送给 Control             │
└──────────────────────┬──────────────────────────┘
                       │ cli_rx
┌──────────────────────▼──────────────────────────┐
│              Control 层 (调度核心)                │
│                                                 │
│   control.rs: tokio::select! {                  │
│     cli_rx      → handle_run()                  │
│     inbound_rx  → handle_inbound()              │
│     event_rx    → handle_event()                │
│   }                                             │
└───────┬─────────────────────────┬───────────────┘
        │                         │
┌───────▼───────┐        ┌───────▼───────┐
│ ML_Service    │        │  NodeHandle   │
│   _Handle     │        │+ inbound_rx   │
│               │        │+ event_rx     │
└───────────────┘        └───────────────┘
```

### 核心设计原则

| 原则 | 说明 |
|------|------|
| 对等节点 | 所有节点运行同一份代码，没有预设 master/slave 角色 |
| 任务发起者 = 协调者 | 谁输入了 run 命令，谁就是这次任务的协调者 |
| 全部接受 | MVP 阶段不拒绝任何请求（文件传输、推理请求一律接受） |
| 统一存储 | 所有文件（模型、切分模型、接收文件）都在 Pleiades_Workspace/ 目录下 |

---

## 2. 文件结构

```
Src/Control/
├── mod.rs           # 模块入口，注册子模块，导出公共接口
├── cli.rs           # CLI 循环：读取用户命令，发送给 Control
└── control.rs       # Control 核心：select! 事件循环 + 调度逻辑
```

---

## 3. cli.rs

### 职责

CLI 是一个运行在独立线程中的阻塞循环，负责：
1. 打印提示符 `pleiades> `
2. 读取用户输入
3. 解析命令
4. 通过 channel 发送给 Control
5. 等待 Control 返回结果并打印

### 命令定义

MVP 阶段只有一个命令：

```
run <model_path> <prompt>
```

例如：
```
pleiades> run Pleiades_Workspace/Qwen3-0.6B-Q8_0.gguf "你好"
```

### CLI_Command 枚举

```rust
enum CLI_Command {
    Run {
        model_path: PathBuf,
        prompt: String,
        reply: oneshot::Sender<Result<String>>,
    },
    Quit,
}
```

### 运行方式

CLI 在 `tokio::task::spawn_blocking` 中运行（因为 stdin 读取是阻塞操作），内部使用 `blocking_send` 和 `blocking_recv` 与 Control 通信。

```rust
// CLI 主循环（spawn_blocking 内）
loop {
    print!("pleiades> ");
    flush stdout;
    read line from stdin;

    if line == "quit" {
        cli_tx.blocking_send(CLI_Command::Quit);
        break;
    }

    // 解析 "run <model_path> <prompt>"
    let (model_path, prompt) = parse_run_command(line);
    let (reply_tx, reply_rx) = oneshot::channel();
    cli_tx.blocking_send(CLI_Command::Run { model_path, prompt, reply: reply_tx });

    // 同步等待结果
    match reply_rx.blocking_recv() {
        Ok(Ok(msg)) => println!("{}", msg),
        Ok(Err(e))  => println!("[错误] {}", e),
        Err(_)      => println!("[错误] Control 未响应"),
    }
}
```

---

## 4. control.rs

### 职责

Control 是整个系统的调度核心，通过 `tokio::select!` 同时处理三个事件源：

1. **cli_rx**：CLI 发来的用户命令
2. **inbound_rx**：其他节点发来的网络请求（InboundRequest）
3. **event_rx**：网络事件（连接/断开/文件到达等）

### 主事件循环

```rust
loop {
    tokio::select! {
        Some(cmd) = cli_rx.recv() => {
            match cmd {
                CLI_Command::Run { model_path, prompt, reply } => {
                    let result = Handle_Run(&ml_service, &node_handle, &model_path, &prompt).await;
                    let _ = reply.send(result);
                }
                CLI_Command::Quit => {
                    // 优雅退出
                    ml_service.Shutdown().await;
                    node_handle.Stop().await;
                    break;
                }
            }
        }
        Some(req) = inbound_rx.recv() => {
            Handle_Inbound(&ml_service, &node_handle, req).await;
        }
        Some(evt) = event_rx.recv() => {
            Handle_Event(evt).await;
        }
    }
}
```

### Handle_Run() —— 处理 run 命令

当用户输入 `run model_path prompt` 时执行：

```
Handle_Run(ml_service, node_handle, model_path, prompt):

Step 1: 查询网络中的节点
    peers = node_handle.Get_Peers().await
    N = peers.len() + 1  // 包含自己
    输出: "[Pleiades] 发现 {N} 台设备"

Step 2: 分析模型
    arch_info = ml_service.Analyze_Model(model_path).await
    total_layers = arch_info.num_layers + 2  // 层0(embedding) + 层1~N(transformer) + 层N+1(output)
    输出: "[Pleiades] 模型: {architecture}, {total_layers} 层"

Step 3: 计算均分方案
    layers_per_node = total_layers / N
    remainder = total_layers % N
    // 前 remainder 个节点多分 1 层
    // 例: 30层, 3台 → 10, 10, 10 (0-9, 10-19, 20-29)
    // 例: 30层, 4台 → 8, 8, 7, 7  (0-7, 8-15, 16-22, 23-29)

    assignments = []
    current = 0
    for i in 0..N:
        count = layers_per_node + (1 if i < remainder else 0)
        assignments.push((current, current + count - 1))
        current += count

    输出: "[Pleiades] 分配方案: ..."

Step 4: 切分模型
    output_dir = "Pleiades_Workspace/"
    for (start, end) in assignments:
        ml_service.Split_Model(model_path, start, end, output_dir).await
    输出: "[Pleiades] 模型切分完成, 生成 {N} 个文件"

Step 5: 分发文件给其他节点
    // assignments[0] 是自己的，不用发
    // assignments[1..] 分别发给 peers[0], peers[1], ...
    for (peer, (start, end)) in peers.iter().zip(assignments[1..].iter()):
        split_file_path = format!("Pleiades_Workspace/{stem}_split_{start}_{end}.pgguf")
        node_handle.Send_File(peer, split_file_path).await
    输出: "[Pleiades] 模型文件已发送给所有节点"

Step 6: 返回结果
    return Ok("模型分发完成")
```

### Handle_Inbound() —— 处理入站请求

MVP 阶段第一步只需处理文件接收（文件由网络层自动保存到 Pleiades_Workspace/）。

```
Handle_Inbound(ml_service, node_handle, req):
    // MVP 阶段: 所有请求全部接受
    match req.data_type {
        DataType::File => {
            // 文件传输通知，自动接受
            node_handle.Send_Reply(req.request_id, DataType::Command, b"ACCEPT").await
            输出: "[Pleiades] 文件接收请求已接受"
        }
        _ => {
            // 后续扩展: 处理 LOAD 命令、tensor 推理请求等
            node_handle.Send_Reply(req.request_id, DataType::Command, b"OK").await
        }
    }
```

### Handle_Event() —— 处理网络事件

```
Handle_Event(event):
    match event {
        NetworkEvent::PeerDiscovered(peer) => {
            输出: "[Pleiades] 发现节点: {peer}"
        }
        NetworkEvent::ConnectionEstablished(peer) => {
            输出: "[Pleiades] 连接建立: {peer}"
        }
        NetworkEvent::FileStreamReceived { peer, file_path } => {
            输出: "[Pleiades] 收到文件: {file_path} (来自 {peer})"
        }
        NetworkEvent::FileStreamError { peer, error } => {
            输出: "[Pleiades] 文件传输错误: {error} (节点 {peer})"
        }
        _ => { /* 忽略 */ }
    }
```

---

## 5. 节点运行状态

每个节点启动后同时运行两种模式：

```
┌──────────────────┐  ┌──────────────────┐
│  CLI 等待用户输入  │  │  inbound 监听    │
│  (主动模式)       │  │  (被动模式)      │
│                  │  │                  │
│  用户输入 run →  │  │ 收到文件 → 保存  │
│  成为协调者      │  │ 到Pleiades_      │
│  分析+切分+分发  │  │ Workspace/       │
└──────────────────┘  └──────────────────┘
```

两种模式通过 tokio::select! 同时运行，任何节点在任何时刻都可能：
- 作为协调者（自己输入了 run）
- 作为参与者（收到其他节点发来的文件或命令）

---

## 6. 初始化流程

main.rs 中的启动顺序：

```
1. 读取 config.toml 配置
2. 初始化 Network: Node::Init(config) → (node, node_handle, inbound_rx)
3. 启动 Node 事件循环: tokio::spawn(node.Start())
4. 初始化 ML Service: ML_Service_Handle::Init()
5. 启动 CLI: tokio::task::spawn_blocking(cli_loop)
6. 启动 Control 事件循环: control_loop(cli_rx, inbound_rx, event_rx, ml_service, node_handle)
```

---

## 7. 验证场景

两台局域网设备（A 和 B）：

```
设备 A:                                      设备 B:
$ pleiades                                   $ pleiades
[Pleiades] 节点启动                           [Pleiades] 节点启动
[Pleiades] PeerId: 12D3Koo...(A)            [Pleiades] PeerId: 12D3Koo...(B)
[Pleiades] 发现节点: 12D3Koo...(B)          [Pleiades] 发现节点: 12D3Koo...(A)
[Pleiades] 连接建立: 12D3Koo...(B)          [Pleiades] 连接建立: 12D3Koo...(A)

pleiades> run Pleiades_Workspace/Qwen3-0.6B-Q8_0.gguf "你好"
[Pleiades] 发现 2 台设备
[Pleiades] 模型: qwen3, 30 层
[Pleiades] 分配: A(0-14), B(15-29)
[Pleiades] 模型切分完成
[Pleiades] 文件发送中...                      [Pleiades] 收到文件: Qwen3-0.6B-Q8_0_split_15_29.pgguf
[Pleiades] ✓ 模型分发完成
```

---

## 8. 后续扩展（不在当前阶段实现）

| 阶段 | 内容 |
|------|------|
| 第二阶段 | 协调者发送 LOAD Command → 各节点加载切分模型 |
| 第三阶段 | 协调者发送 INFERENCE_MODE Command → 建立流水线链 |
| 第四阶段 | 协调者驱动自回归循环: Encode → Pipeline x120 → Decode |

---

## 9. 版本记录

| 版本 | 日期 | 内容 |
|------|------|------|
| v1.0 | 2026-04-07 | 初始设计，明确 CLI/Control 第一阶段：run 命令 → 切分 → 分发 |
