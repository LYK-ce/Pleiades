# SessionManager 预设计

## 1. 定位

SessionManager 是一个扁平的模组，负责：

1. **会话生命周期管理** — 创建/销毁推理会话
2. **IO 通道聚合** — 管理 ML Thread 与多个前端之间的文本通路
3. **槽位分配** — 基于固定槽位的连续批处理调度

ML Engine 退回纯推理能力（加载模型、前向传播、卸载模型），不持有 session 注册表、不关心 IO 路由。

## 2. 核心概念

### Session

```
Session {
    session_id: String
    model: ModelInfo
    slots: [Slot; max_slots]       ← 固定槽位，先占不还
    ml_channel: IoHandle           ← 通向 ML Thread 的唯一通道
    frontends: Vec<IoFrontend>     ← 通向各前端的通道
}
```

- 一个 Session 对应一个 ML Thread、一个模型实例
- 槽位上限 `max_slots` 由 `config.toml` 中 `[Session]` 段手动配置
- 配置项位于 `Src/Config/config.toml` + `Src/Config/config.rs` (`Session_Config.max_slots`)，默认值 4

### Slot

```
Slot {
    slot_id: u32                   ← 服务序列号，从 0 递增
    state: Vacant / Occupied
    owner: FrontendInfo            ← 归属哪个前端
}
```

- 槽位固定不释放（v1 版本），对话结束后仍占位
- 后续实现释放：通知 ML Thread 清理 KV cache，槽位标记为 Vacant

### 微服务模型

每个前端接入方式是一个独立的微服务，通过 `connect(session_id)` 获取 `(slot_id, IoFrontend)`，自行管理自身生命周期：

```
SessionManager   (核心——管理会话和槽位)
    │
    │ connect(session_id) → (slot_id, IoFrontend)
    │
    ├── ChatService    微服务：持有 IoFrontend，对接 TUI Prompt 面板
    ├── ApiService     微服务：持有 IoFrontend，监听端口，HTTP + SSE
    └── NetService     微服务：持有 IoFrontend，桥接 P2P 网络流（后续实现）
```

| 命令 | 启动的微服务 | 微服务职责 |
|------|------------|-----------|
| `chat <session_id>` | ChatService | 调 `connect` 占槽位，将 IoFrontend 对接 TUI Prompt 面板；输⼊/输出直接在 TUI 内完成 |
| `api <session_id>` | ApiService | 调 `connect` 占槽位，启动本地 HTTP server，监听独立端口；将 HTTP 请求体→prompt 送入通道，token 以 SSE 逐条返回 |

**端口管理、HTTP 路由、SSE 流管理、TUI 渲染——全部由微服务负责，SessionManager 不感知。**

## 3. 接口

```
trait SessionManager {
    // ── Session 生命周期 ──

    /// 创建推理会话，加载模型，启动 ML Thread
    /// 返回 session_id
    create_session(model_id: String, config: SessionConfig)
        -> Result<session_id>

    /// 销毁会话，停止 ML Thread，回收所有资源
    destroy_session(session_id)
        -> Result<()>

    // ── 槽位分配 ──

    /// 申请一个槽位，返回 (slot_id, IoFrontend)
    /// 调用方自行决定如何使用 IoFrontend（TUI / HTTP / Network）
    connect(session_id)
        -> Result<(slot_id, IoFrontend)>

    // ── 查询 ──

    /// 列出本地所有活跃的 Session
    list_sessions() -> Vec<SessionInfo>

    // ── 未来 ──

    /// 释放槽位（v2）
    release_slot(session_id, slot_id) -> Result<()>
}
```

## 4. 运行时流程

### 阶段 1：启动推理服务

```
用户输入: run qwen3.gguf

SessionManager.create_session(model_id, config):
  1. 生成 session_id
  2. 分配 max_slots 个槽位（全部 Vacant）
  3. 创建 channel pair，得 (IoHandle_ml, frontend_side)
  4. 注册 Session 到内部 sessions 表
  5. 返回 (session_id, IoHandle_ml)

ML_Engine.spawn(IoHandle_ml, model_config):
  1. 接收通道端点 + 模型配置
  2. 加载模型，启动 ML Thread
  3. ML Thread 进入等待循环（无 prompt 时休眠/空转）

两者互不感知。SessionManager 不知道 ML Engine 的存在，
ML Engine 不知道 SessionManager 的存在。
唯一的耦合点：IoHandle (mpsc channel pair)。
```

### 阶段 2：建立对话通道

```
用户输入: chat sess-1

TUI:
  1. 启动 ChatService 微服务
  2. ChatService 调 SessionManager::connect(sess-1)
  3. 拿到 (slot_id, IoFrontend)
  4. 激活 Prompt 面板，开始对话

用户输入: api sess-1

TUI:
  1. 启动 ApiService 微服务
  2. ApiService 调 SessionManager::connect(sess-1)
  3. 拿到 (slot_id, IoFrontend)
  4. 在 127.0.0.1:<port> 启动 HTTP SSE 服务
```

### 阶段 3：推理

```
┌─────────────────────────────────────────────────┐
│                  Session                         │
│                                                   │
│  slot_0 (TUI)    ── prompt ──                      │
│  slot_1 (HTTP)   ── prompt ──  ┌─────────────┐  │
│  slot_2 (空)                   │ 组装整帧     │  │
│  ...                           │ [N,seqlen,H] │──┤──→ ML Thread
│  slot_7 (空)                   └─────────────┘  │    前向传播
│                                                   │
│         token_0 ←── TUI        ┌─────────────┐  │
│         token_1 ←── HTTP       │ 按slot_id    │←─┤
│                                │ 拆解输出     │  │
│                                └─────────────┘  │
└─────────────────────────────────────────────────┘
```

- ML Thread 看到的始终是 `[MAX_SLOTS, seqlen, H]`
- 空槽用 attention mask 屏蔽
- 组包策略：即时模式（来一个 prompt 立刻推一帧）
- 输出按 slot_id 拆解，定向回传给发 prompt 的前端

## 5. 与现有模块的关系（后续阶段参考）

| 现有模块 | 后续变化 |
|---------|---------|
| `LLM_IO` (Broker + Capability) | 本阶段升级为 SessionManager，移除 JobId key，换为 SessionId |
| `ML_Engine_Service.sessions` | 后续 session 注册表迁移到 SessionManager |
| `ML_Engine_Capability` trait | 后续精简掉 `Create_Session` / `Shutdown_Session`，保留 `spawn / analyze / split` |
| `Core::Capabilities` | 后续 `io_broker` → `session_manager` |
| `Core::route_user` | 后续不再手动 Allocate + Take，改为 `create_session` + `ml_engine.spawn` |
| `TUI` | 后续不再 `Take_Frontend(job_id)`，改为 `connect(session_id)` |

## 6. 命令约定

| 命令 | 行为 |
|------|------|
| `run <model_path>` | 创建 Session + ML Thread，返回 session_id |
| `chat <session_id>` | 调 `connect(session_id)`，拿 IoFrontend 激活 TUI Prompt 面板 |
| `api <session_id>` | 调 `connect(session_id)`，拿 IoFrontend 启动本地 HTTP SSE 服务 |
| `stop <session_id>` | 销毁 Session，停 ML Thread，回收端口 |
| `sessions` / `ls-sessions` | 列出本地活跃 Session |
| (未来) `close <session_id> <slot_id>` | 释放槽位，清 KV cache |

---

## 7. 实施计划

> 仅修改 `Src/LLM_IO/` 目录内的代码，将其升级为 SessionManager。不动其他模块（即使编译报错）。

### Phase 1：重命名 + 核心类型定义

| 文件 | 操作 | 说明 |
|------|------|------|
| `Src/LLM_IO/` → `Src/Session/` | 重命名目录 | `LLM_IO` 升级为 `Session` |
| **新建** `Src/Session/session.rs` | 创建文件 | 定义 `Session` 结构体（session_id, model_info, slots, ml_channel）；定义 `SessionInfo` 公开视图 |
| **新建** `Src/Session/slot.rs` | 创建文件 | 定义 `Slot` 结构体（slot_id, state: Vacant/Occupied）；定义 `SlotState` 枚举 |
| `Src/Session/mod.rs` | 修改声明 | `mod session; mod slot;` + 类型导出 |
| `Cargo.toml` → `lib.rs` | 更新引用 | `mod LLM_IO` → `mod Session` |

### Phase 2：SessionManager 结构体 + Capability trait

| 文件 | 操作 | 说明 |
|------|------|------|
| **重命名** `broker.rs` → `manager.rs` | 重命名 | `LLM_IO_Broker` → `SessionManager` |
| `manager.rs` | 重写结构体 | `HashMap<JobId, ChannelEntry>` → `HashMap<session_id, Session>`；读取 `config.toml` `[Session].max_slots` 初始化 |
| **删除** `capability.rs` | 删除旧文件 | `LLM_IO_Capability` trait（Allocate/Deallocate） |
| **新建** `capability.rs` | 创建文件 | 定义 `Session_Capability` trait（create_session / destroy_session / connect / list_sessions / release_slot） + `Session_Error` 错误类型 |
| `mod.rs` | 更新导出 | 删除旧导出（`LLM_IO_Capability` / `LLM_IO_Broker` / `LLM_IO_Error` / `IoHandle` / `IoFrontend`）；导出新类型（`Session_Capability` / `SessionManager` / `Session_Error` / `SessionInfo` / `IoHandle` / `IoFrontend`） |

### Phase 3：实现 trait 方法

| 文件 | 操作 | 说明 |
|------|------|------|
| `manager.rs` | 实现 `create_session()` | 生成 session_id → 分配 max_slots 个槽位 → 创建 channel pair → 注册 Session → 返回 `(session_id, IoHandle_ml)` |
| `manager.rs` | 实现 `connect()` | 查找 Session → slot_counter +=1 分配槽位 → 记录归属 → 返回 `(slot_id, IoFrontend)` |
| `manager.rs` | 实现 `list_sessions()` | 遍历 sessions 表，返回 `Vec<SessionInfo>` |
| `manager.rs` | 实现 `destroy_session()` | 移除 sessions 条目，drop 所有 Sender → 通道关闭 |
| `manager.rs` | 实现 `release_slot()` | 标记槽位 Vacant（v1 空实现，返回 Ok） |

### Phase 4：编写设计文档

| 文件 | 操作 | 说明 |
|------|------|------|
| `design_doc/session_manager_design.md` | 新建文件 | 格式参考 `design_doc/storage_design.md`，完整记录 SessionManager 模块设计，包含数据结构、Trait 定义、核心实现、模块导出、协作关系、已知风险 |

### 最终 trait 方法清单

```rust
#[async_trait]
pub trait Session_Capability: Send + Sync {
    /// 创建 Session（分配槽位 + channel pair），返回 (session_id, IoHandle_ml)
    /// 调用方自行将 IoHandle_ml 传给 ML Engine 启动线程
    async fn create_session(&self, model_id: String, config: SessionConfig)
        -> Result<(String, IoHandle), Session_Error>;

    /// 销毁 Session，清理所有资源
    async fn destroy_session(&self, session_id: &str)
        -> Result<(), Session_Error>;

    /// 申请槽位，返回 (slot_id, IoFrontend)
    async fn connect(&self, session_id: &str)
        -> Result<(u32, IoFrontend), Session_Error>;

    /// 列出所有活跃 Session
    fn list_sessions(&self) -> Vec<SessionInfo>;

    /// 释放槽位（v1 空实现，预留接口）
    async fn release_slot(&self, session_id: &str, slot_id: u32)
        -> Result<(), Session_Error>;
}
```
