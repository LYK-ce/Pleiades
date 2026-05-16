#Presented by KeJi
#Date ： 2026-04-03

# Network Layer Upgrade 修改记录（第二次）

## 改动背景

基于对 Control 层设计的讨论（见 `docs/control.md`），对网络层进行了以下升级：

1. **Response 路由**：原先 Response 在 Node 事件循环中仅做日志后丢弃，现在通过 oneshot channel 路由回 `Send_Bytes()` 调用方
2. **入站请求分流**：原先 `DataReceived` 事件携带 `ResponseChannel`（libp2p 内部类型）上报给上层，现在 Node 截留 `ResponseChannel`，通过 `inbound_tx` 向 Control 层发送不含 libp2p 类型的 `InboundRequest`
3. **厚 API 封装**：NodeHandle 新增 `Send_Bytes`（等 Response）、`Send_Reply`（回复入站请求）、`Send_File`（协商+流式传输）三个面向 Control 层的高级方法

## 修改文件清单

### 1. `Src/Network/node_handle.rs` — 重写

**新增结构体：**
- `InboundRequest`：入站请求（request_id + peer + data_type + payload），不含 libp2p 类型

**NodeCommand 枚举变更：**
- `SendData` 新增字段 `response_tx: Option<oneshot::Sender<Result<DataResponse, String>>>`
- 新增变体 `SendReply { request_id: u64, data_type: DataType, payload: Vec<u8> }`

**NodeHandle 新增方法：**
- `Send_Bytes(peer, data_type, payload) → Result<DataResponse>`：发送数据并等待 Response（30s 超时）
- `Send_Reply(request_id, data_type, payload) → Result<()>`：通过 request_id 回复入站请求
- `Send_File(peer, file_path) → Result<()>`：文件传输（元数据协商 + 流式传输）

**NodeHandle 修改方法：**
- `Send_Data()`：增加 `response_tx: None` 参数（保持 fire-and-forget 语义不变）

### 2. `Src/Network/node.rs` — 修改

**Node 结构体新增字段：**
- `pending_responses: HashMap<OutboundRequestId, oneshot::Sender<Result<DataResponse, String>>>`
- `pending_replies: HashMap<u64, ResponseChannel<DataResponse>>`
- `inbound_tx: mpsc::Sender<InboundRequest>`
- `next_inbound_id: u64`

**Init() 返回值变更：**
- 旧：`Result<(Self, NodeHandle), Box<dyn Error>>`
- 新：`Result<(Self, NodeHandle, mpsc::Receiver<InboundRequest>), Box<dyn Error>>`
- 新增创建 inbound channel 并返回 `inbound_rx`

**Handle_Command() 变更：**
- `SendData` 分支：调用 `send_request()` 后，若 `response_tx` 存在则存入 `pending_responses`
- 新增 `SendReply` 分支：从 `pending_replies` 取出 `ResponseChannel` 发送响应
- `Stop` 分支：清理 `pending_responses`，通过 oneshot 通知等待方节点已停止

**Handle_Request_Response_Event() 变更：**
- Request 分支：不再通过 `event_sender` 发送 `DataReceived`，改为分配 `request_id`、存储 `ResponseChannel` 到 `pending_replies`、通过 `inbound_tx` 转发 `InboundRequest`
- Response 分支：不再仅 `debug!` 日志，改为通过 `OutboundRequestId` 匹配 `pending_responses` 并通过 oneshot 回传
- OutboundFailure 分支：通过 oneshot 通知等待方发送失败

**NetworkEvent 枚举变更：**
- 移除 `DataReceived` 变体（改走 `inbound_tx` 通道）

**新增 import：**
- `tokio::sync::oneshot`
- `libp2p::request_response::OutboundRequestId`
- `super::node_handle::InboundRequest`

### 3. `Src/Network/mod.rs` — 修改

- 更新模块文档，增加 Control 层接口说明
- re-export 列表增加 `InboundRequest`

### 4. `Src/lib.rs` — 修改

- 增加 `pub use network::InboundRequest`

### 5. `Src/Network/network_service.rs` — 删除

- 初始讨论中创建的占位文件，合并设计后不再需要

### 6. `Src/Network/data_protocol.rs` — 未修改

### 7. `Src/Network/stream_protocol.rs` — 未修改

## 架构变化总结

### 旧架构（thin 网络层）

```
Control 层 ──Send_Data()──► NodeHandle ──► Node ──► 远程节点
                                              │
Control 层 ◄──event_rx── NetworkEvent::DataReceived(含 ResponseChannel)
                         NetworkEvent::Response(仅日志，丢弃)
```

问题：
- Response 不上报，调用方不知道对方是否收到
- DataReceived 携带 ResponseChannel（libp2p 类型），Control 层与 libp2p 耦合

### 新架构（thick 网络层）

```
Control 层 ──Send_Bytes()──► NodeHandle ──► Node ──► 远程节点
            .await 阻塞等待            │
Control 层 ◄── Ok(Response) ◄── oneshot ◄── Node 收到 Response

Control 层 ◄── inbound_rx ◄── InboundRequest(request_id, 不含 libp2p 类型)
Control 层 ──Send_Reply(request_id)──► NodeHandle ──► Node ──► 远程节点
```

| 维度 | 旧 | 新 |
|------|----|----|
| 出站 Response | 丢弃（仅日志） | oneshot 路由回调用方 |
| 入站请求 | `NetworkEvent::DataReceived` + `ResponseChannel` | `InboundRequest` + `request_id` |
| Control 层与 libp2p 耦合 | 是（持有 ResponseChannel） | 否（仅 u64 request_id） |
| 发送+等确认 | 不支持 | `Send_Bytes().await → Result<Response>` |
| 文件传输协商 | 手动多步 | `Send_File().await`（一行搞定） |
| 回复入站请求 | 手动持有 ResponseChannel | `Send_Reply(request_id)` |

## 未修改的文件

- `tests/common/network_test.rs` — 仍引用旧 API，需后续适配
- `tests/common/collaborative_test.rs` — 仍引用旧 API，需后续适配
