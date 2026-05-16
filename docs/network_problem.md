# Network 问题记录

Presented by KeJi
Date ： 2026-05-16

## 问题1：UpdateInfo 批量测速策略不当

### 现状

`Handle_Command::UpdateInfo` 遍历**所有已知节点**串行执行带宽测试，每个节点 3 秒，N 个节点耗时 3N 秒。测速结果写回 PeerManager。

```rust
// network_service.rs Handle_Command
NodeCommand::UpdateInfo { reply } => {
    let peers = self.peer_handle.List_Peers().await;  // 全部节点
    for peer_info in &peers {
        self.Test_Bandwidth(&peer_info.peer_id).await; // 逐个测
    }
}
```

### 问题

1. **粒度错误**：带宽测试应该是针对**指定节点**的操作，不应隐式遍历全体。当前"遍历全体"的策略是 Network 层越权决策。
2. **阻塞时间长**：节点数多时，串行测速会长时间阻塞事件循环中 `cmd_rx` 的处理（其他命令排队等待）。
3. **无法按需触发**：上层（Orchestrator/Lua）无法只测某一个感兴趣的节点。

### 预期修正

`NodeCommand::UpdateInfo` 应接受目标 `peer_id` 参数，仅对该节点执行测速：

```rust
NodeCommand::UpdateInfo { peer, reply } => {
    match self.Test_Bandwidth(&peer).await {
        Ok(mbps) => { let _ = reply.send(Ok(mbps)); }
        Err(e)  => { let _ = reply.send(Err(e)); }
    }
}
```

遍历、调度、频率控制等策略交给上层（Lua 脚本 / Orchestrator）决定。

### 状态

待重构。
