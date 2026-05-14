# 潜在风险记录

## PeerManager

### 1. 心跳全局写锁竞争

- **风险**：`Update_Profile`（原 `update_heartbeat`）高频调用时，每次心跳都需要获取 `RwLock<HashMap>` 的全局写锁。多节点场景下，心跳更新会阻塞所有读操作（`Get_Peer`、`Get_Peers`、`Count` 等），造成不必要的排队延迟。
- **影响范围**：Network 心跳 → Orchestrator/Scheduler 查询 + Lua 调用链路
- **当前状态**：记录为已知风险，沿用现有 `tokio::sync::RwLock<HashMap>` 方案
- **备用方案**：`DashMap` 分片锁（引入新依赖，同步锁在 tokio 上下文中不完美）
