# 潜在风险记录

## PeerManager

### 1. 心跳全局写锁竞争

- **风险**：`Update_Profile`（原 `update_heartbeat`）高频调用时，每次心跳都需要获取 `RwLock<HashMap>` 的全局写锁。多节点场景下，心跳更新会阻塞所有读操作（`Get_Peer`、`Get_Peers`、`Count` 等），造成不必要的排队延迟。
- **影响范围**：Network 心跳 → Orchestrator/Scheduler 查询 + Lua 调用链路
- **当前状态**：记录为已知风险，沿用现有 `tokio::sync::RwLock<HashMap>` 方案
- **备用方案**：`DashMap` 分片锁（引入新依赖，同步锁在 tokio 上下文中不完美）

## Tensor_Stream

### 1. 单 inference_id 多流

- **风险**：当前 Rendezvous 设计一个 `inference_id` 对应一 inbound + 一 outbound。若未来需冗余备份或多路传输（一个推理会话对应多条 tensor 流），rendezvous 需支持一个 id 匹配多条流。
- **影响范围**：`RendezvousMap` 数据结构 + Lua API 语义
- **当前状态**：先不考虑，当前单 inbound + 单 outbound 满足需求
