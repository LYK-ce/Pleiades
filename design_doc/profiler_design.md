# Profiler 设计方案

## 1. 背景

需要在 ML 层增加对模型层前向推理性能的 Profile 能力。通过加载指定层范围，填充固定的输入 tensor，指令驱动执行单次推理，由结果中的耗时获取性能数据。

潜在调用方：
- **TUI 入口**：用户输入 `Profile <model>` 命令，本机 + 向所有 peer 广播
- **网络层入站**：收到远程 peer 的 profile 请求 → 执行 → 返回耗时

## 2. 目标与完整流程

```
TUI: "Profile xxxmodel"
  │
  ├── 1. 本机 self-profile → layer_time (model_id → Duration)
  │
  ├── 2. PeerManager.list_all → for_each peer:
  │       Network.Send(peer, ProfileRequest { model_id, K=5, device })
  │         ├── peer 有此模型 → profile → 返回 Some(Duration) / K
  │         └── peer 无此模型 → 返回 None
  │
  ├── 3. 收集所有 reply:
  │       PeerManager.update(peer_id, model_id, Some(layer_time))
  │       或 PeerManager.update(peer_id, model_id, None)
  │
  └── 4. Scheduler 后续读取 PeerCapability.layer_time[model_id] 做层划分
```

| 组件 | 职责 |
|------|------|
| TUI | 解析 `Profile <model>` 命令 |
| Orchestrator | 本机 self-profile + 网络广播 + 等待 reply |
| Network | ProfileRequest / ProfileResponse 序列化传输 |
| PeerManager | 存储 `PeerCapability.layer_time: HashMap<model_id → Duration>` |
| Scheduler | 读取 `layer_time[model_id]` 做层划分（后续修改） |

## 3. ML 层修改

### 3.1 instruction.rs — 新增 `FillTensor` 变体

```rust
pub enum MlInstruction {
    // ... 已有 16 条不变 ...

    /// 用固定值填充 tensor，形状从 slot 读取（F64 → usize）
    FillTensor {
        dst: SlotId,              // 目标 tensor 槽位, 如 SLOT_TENSOR1
        shape_slots: Vec<SlotId>, // 形状维度槽位, 如 [SLOT_META7, SLOT_META8, SLOT_META3]
        value: f32,               // 填充值, 如 1.0
    },
}
```

### 3.2 engine.rs — 新增 handler + step 分发

**step() 新增 match 臂**：

```rust
MlInstruction::FillTensor { dst, shape_slots, value } =>
    self.handle_fill_tensor(dst, shape_slots, value),
```

**新增 handler**：

```rust
fn handle_fill_tensor(&mut self, dst: SlotId, shape_slots: Vec<SlotId>, value: f32) -> StepResult {
    let shape: Vec<usize> = shape_slots.iter()
        .map(|s| self.vm.slots.get_f64(*s).unwrap_or(1.0) as usize)
        .collect();
    let device = self.backend.device.clone();
    let tensor = match Tensor::full(value, &shape, &device) {
        Ok(t) => t,
        Err(e) => return StepResult::Abort(format!("FillTensor: 创建失败: {}", e)),
    };
    self.ml_slots.set_tensor(dst, tensor);
    StepResult::Continue
}
```

## 4. ML 程序模板

### 4.1 `programs/ml/profile.tmpl`

```toml
# Profile 程序: 填充固定 tensor → 单次前向推理
# hidden_dim 通过 $hidden_dim 变量注入

[[instructions]]
type = "Const"
value_type = "U64"
value = "1"
dst = "META7"

[[instructions]]
type = "Const"
value_type = "U64"
value = "1"
dst = "META8"

[[instructions]]
type = "Const"
value_type = "U64"
value = "$hidden_dim"
dst = "META3"

[[instructions]]
type = "FillTensor"
dst = "TENSOR1"
shape_slots = ["META7", "META8", "META3"]
value = 1.0

[[instructions]]
type = "Inference"
input_type = "Tensor"
input = "TENSOR1"
```

### 4.2 ProgramSelector 改动

`load_ml_program_vm` 新增参数 `vars: &HashMap<String, String>` 用于解析 `$hidden_dim`：

```rust
pub fn load_ml_program_vm(
    mode: &str,
    params: &Pipeline_Params,
    vars: &HashMap<String, String>,  // 新增: 如 {"hidden_dim" → "2560"}
) -> Result<Vec<MlInst>, SelectorError> {
```

`convert_ml_instruction_vm` 在解析 `FillTensor` 时，将 `shape_slots` 数组中的字符串通过 `resolve_ml_slot` 转换为 `SlotId`。

`resolve_ml_const_value` 中增加对 `$hidden_dim` 的处理：从 `vars` HashMap 查找。

`convert_ml_instruction_vm` 新增 `FillTensor` 类型解析：

```rust
"FillTensor" => {
    let dst = resolve_ml_slot(raw.dst.as_deref().unwrap_or(""))?;
    let value = raw.value.unwrap_or(1.0);
    let shape_slots: Vec<SlotId> = raw.shape_slots
        .as_ref()
        .unwrap_or(&vec![])
        .iter()
        .map(|s| resolve_ml_slot(s.as_str().unwrap_or("")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MlInst::FillTensor { dst, shape_slots, value })
}
```

### 4.3 handle_run_program 不变

`handle_run_program` 不做任何修改：

```rust
let program = ProgramSelector::load_ml_program_vm(mode, &params, &HashMap::new())?;
```

## 5. Orchestrator 层修改

### 5.1 JobKind 新增

```rust
pub enum JobKind {
    Run,
    Relay,
    Pipeline,
    Send,
    Receive,
    Profile,  // 新增
}
```

### 5.2 `programs/orchestrator/Profile.tmpl`

```toml
[meta]
kind = "Profile"

[[instructions]]
type = "Const"
value_type = "String"
value = "$model_path"
dst = "SLOT_MODEL"

[[instructions]]
type = "Const"
value_type = "String"
value = "$device"
dst = "SLOT_DEVICE"

[[instructions]]
type = "Const"
value_type = "U64"
value = "$layer_start"
dst = "SLOT_LAYER_START"

[[instructions]]
type = "Const"
value_type = "U64"
value = "$layer_end"
dst = "SLOT_LAYER_END"

[[instructions]]
type = "CreateSession"
model = "SLOT_MODEL"
device = "SLOT_DEVICE"
start = "SLOT_LAYER_START"
end = "SLOT_LAYER_END"
io = "SLOT_IO"
result = "SLOT_SESSION"

[[instructions]]
type = "Profile"
session = "SLOT_SESSION"
result = "SLOT_RESULT"
```

### 5.3 ProgramSelector 新增模板常量

```rust
const PROFILE_TMPL: &str = include_str!("../../../programs/orchestrator/Profile.tmpl");
```

`select()` 函数增加 `JobKind::Profile` 分支。

### 5.4 新增 Orchestrator 指令 `Profile`

```rust
pub enum OrchestratorInstruction {
    // ... 已有指令不变 ...

    Profile {
        session: SlotId,
        result: SlotId,
    },
}
```

放在 `inference_handler.rs` 中（与 CreateSession/RunProgram 同组）。

### 5.5 handler: `handle_profile`

```rust
async fn handle_profile(&mut self, session: SlotId, result: SlotId) -> StepResult {
    // 1. 从槽位读 hidden_dim（Create_Session 顺带写入）
    let hidden_dim = match self.vm.slots.get_u64(SLOT_HIDDEN_DIM) {
        Ok(dim) => dim as usize,
        Err(e) => return StepResult::Abort(format!("Profile: hidden_dim 未设置: {}", e)),
    };

    // 2. 构建 vars，加载 profile 程序
    let params = Pipeline_Params::default();
    let mut vars = HashMap::new();
    vars.insert("hidden_dim".to_string(), hidden_dim.to_string());
    let program = match ProgramSelector::load_ml_program_vm("profile", &params, &vars) {
        Ok(p) => p,
        Err(e) => return StepResult::Abort(format!("Profile: 加载程序失败: {}", e)),
    };

    // 3. 读 session_id
    let session_id = match self.vm.slots.get_string(session) {
        Ok(id) => id.clone(),
        Err(e) => return StepResult::Abort(format!("Profile: session_id 为空: {}", e)),
    };

    // 4. 执行
    let cancel_flag = Arc::new(AtomicBool::new(false));
    match self.capabilities.ml_engine.Run_Program_VM(
        &session_id,
        program,
        params,
        cancel_flag,
    ).await {
        Ok(pipeline_result) => {
            self.vm.slots.set(result, SlotValue::F64(pipeline_result.duration.as_secs_f64()));
            StepResult::Continue
        }
        Err(e) => StepResult::Abort(format!("Profile: 执行失败: {:?}", e)),
    }
}
```

### 5.6 Create_Session 顺带写入 hidden_dim

`handle_create_session` 在拿到 `Model_Info` 后额外写一个槽位：

```rust
// 原有逻辑 ...
let model_info = capabilities.ml_engine.Create_Session(config, io_handle).await?;

// 新增：写入 hidden_dim 槽位，供 Profile 指令读取
self.vm.slots.set(
    SLOT_HIDDEN_DIM,
    SlotValue::U64(model_info.embedding_length as u64),
);
```

### 5.7 Orchestrator_VM 槽位新增

```rust
// slots.rs
pub const SLOT_HIDDEN_DIM: SlotId = SlotId(302);
```

### 5.8 step() 分发

`engine.rs` 中 `step()` 新增 match 臂：

```rust
OrchestratorInstruction::Profile { session, result } =>
    self.handle_profile(session, result).await,
```

## 6. TUI 层修改

### 6.1 命令解析

新增 `Profile` 命令：

```rust
// command.rs
pub enum UserCommand {
    // ... 已有 ...
    Profile { model_id: String },
}

// 解析: "Profile qwen3-0.6b" → UserCommand::Profile { model_id: "qwen3-0.6b" }
```

### 6.2 Core 层处理

`branch_command.rs` 新增 `Profile` 分支，和 `Pipeline` 一样——spawn job → 立即返回：

```rust
UserCommand::Profile { model_id } => {
    let vars = vars![("model_id", &model_id), ("device", "cuda")];
    self.spawn_job(JobKind::Profile, vars, move |result| {
        if let Ok(r) = result {
            for (peer_id, layer_time) in r.peer_times {
                self.peer_manager.update_capability(&peer_id, |cap| {
                    cap.layer_time.insert(model_id.clone(), layer_time);
                });
            }
        }
    })?;
    reply.send(Ok("Profile job started".into()));
}
```

Job 内部的 VM handler `handle_profile` 负责：
1. 本机 self-profile
2. 向所有 peer 广播 `ProfileRequest`（for 循环 send_data）
3. 构建 `ProfileResult { peer_times }`

和 `EstablishStreams` 的 for 循环 `send_data` + `.await` 完全一致——阻塞的是 Job 的 VM task，不阻塞 Core。

## 7. PeerManager 修改

### 7.1 PeerCapability 新增字段

```rust
// peer_manager.rs 或 peer_management mod

pub struct PeerCapability {
    pub peer_id: String,
    pub device: String,                    // "cpu" / "cuda"
    pub layer_time: HashMap<String, Duration>,  // 新增: model_id → 单层平均耗时
    // ... 已有字段不变
}
```

### 7.2 更新接口

```rust
impl PeerManager {
    pub fn update_capability<F>(&self, peer_id: &str, modifier: F)
    where
        F: FnOnce(&mut PeerCapability),
    {
        let mut peers = self.peers.write().unwrap();
        if let Some(info) = peers.get_mut(peer_id) {
            modifier(&mut info.capability);
        }
    }
}
```

## 8. Network 层修改

### 8.1 NetworkProtocol 新增 ProfileRequest/ProfileResponse

```rust
// command.rs — NetworkProtocol 枚举新增
pub enum NetworkProtocol {
    // ... 已有 ...
    ProfileRequest {
        model_id: String,
        layer_count: u32,   // K = 5
        device: String,      // "cpu" / "cuda"
    },
}
```

**序列化**：`"PROFILE|{model_id}|{layer_count}|{device}"`

**请求方**（`handle_profile` 内 for 循环）：
```rust
for peer in self.capabilities.peer_manager.list_all() {
    let cmd = NetworkProtocol::ProfileRequest { model_id, layer_count: 5, device: "cuda" };
    let payload = Serialize_Network_Command(&cmd);
    let response = self.capabilities.network.send_data(
        peer.peer_id, DataType::Command, payload
    ).await?;
    // 解析 response → Duration
}
```

### 8.2 入站处理

`branch_command.rs` 的 `handle_network_command` 新增 match 臂：

```rust
NetworkProtocol::ProfileRequest { model_id, layer_count, device } => {
    info!("ProfileRequest from {}, model={}, layers={}", req.peer, model_id, layer_count);

    // 本机 self-profile（直接 await，同 Establish_Tensor_Stream 模式）
    let duration = self.profile_local(&model_id, layer_count as usize, &device).await;

    let response = match duration {
        Some(d) => format!("OK|{}", d.as_micros()),
        None => "FAIL|model not found".to_string(),
    };
    self.capabilities.network.send_response(
        req.request_id, DataType::Command, response.into_bytes()
    ).await;
}
```

## 9. ML Profile 执行流程

```
Orchestrator                          ML_Engine_Service               Session_Thread
    │                                       │                              │
    │ Create_Session(layer_start=1,         │                              │
    │               layer_end=K, ...)        │                              │
    │──────────────────────────────────────→│ spawn Session_Thread         │
    │                                       │─────────────────────────────→│
    │                                       │       GGUF_Load_Model(1,K)   │
    │                                       │←─ ready_tx: Model_Info ─────│
    │←── Model_Info ────────────────────────│     { embedding_length }     │
    │                                       │                              │
    │ 执行 Profile 指令:                    │                              │
    │   读 SLOT_HIDDEN_DIM → H              │                              │
    │   load_ml_program_vm("profile",       │                              │
    │     params, vars{"hidden_dim"→H})     │                              │
    │   → [ Const(1→META7)                  │                              │
    │       Const(1→META8)                  │                              │
    │       Const(H→META3)                  │                              │
    │       FillTensor(→TENSOR1)            │                              │
    │       Inference(Tensor,TENSOR1) ]     │                              │
    │                                       │                              │
    │   Run_Program_VM(session_id, program,  │                              │
    │                 params, cancel_flag)   │                              │
    │──────────────────────────────────────→│ Run_Program_VM               │
    │                                       │─────────────────────────────→│
    │                                       │         ML_VM::execute        │
    │                                       │     Instant::now()            │
    │                                       │       FillTensor  (~μs)       │
    │                                       │       Inference   (~ms, K层)  │
    │                                       │     start.elapsed()           │
    │                                       │←─ reply: Pipeline_Result ────│
    │←── Pipeline_Result { duration } ──────│                              │
    │                                       │                              │
    │ Shutdown_Session(id)                  │                              │
    │──────────────────────────────────────→│ Shutdown                    │
    │                                       │─────────────────────────────→│
    │                                       │               GGUF_Unload    │
    │                                       │                              │
    ▼                                       ▼                              ▼
单层均值 = duration / K
```

## 10. 设计决策

| 决策 | 选择 | 理由 |
|------|------|------|
| ML 模板化 | 是，`programs/ml/profile.tmpl` | 与 run/relay/coordinator 统一 |
| Orchestrator 模板化 | 是，`programs/orchestrator/Profile.tmpl` | 与 Run/Relay/Pipeline 统一 |
| Shape 机制 | Const 注入 dim → FillTensor 读取 slot | 重用已有 Const 体系，不引入新变量解析 |
| 计时方式 | `Pipeline_Result.duration` | 已有，不需要 Timer 指令 |
| 加载层数 | K=5 固定值 | 假设设备存有完整模型，5 层可反映计算性能 |
| Profile 作为独立 JobKind | 是 | 复用 job 生命周期、回调、网络回复 |
| hidden_dim 传递 | `load_ml_program_vm` 新增 vars 参数 | 干净，不污染 Pipeline_Params |
| Profile 命令入口 | TUI 独立命令 | 不耦合到 Pipeline，用户手动触发 |
| layer_time 类型 | `HashMap<String, Duration>` | 不同模型耗时不同，按 model_id 索引 |
| 网络协议 | ProfileRequest / ProfileResponse | 标准 command/reply 模式 |

## 11. 修改清单总览

| 文件 | 改动 |
|------|------|
| `Src/ML_Engine/ML_VM/instruction.rs` | 新增 `MlInstruction::FillTensor` 变体 |
| `Src/ML_Engine/ML_VM/engine.rs` | 新增 `handle_fill_tensor` + step 分发 |
| `programs/ml/profile.tmpl` | 新建，profile 程序模板（5 条指令） |
| `programs/orchestrator/Profile.tmpl` | 新建，profile 编排模板 |
| `Src/Orchestrator/job.rs` | `JobKind` 新增 `Profile` |
| `Src/Orchestrator/Orchestrator_VM/instruction.rs` | 新增 `Profile { session, result }` |
| `Src/Orchestrator/Orchestrator_VM/slots.rs` | 新增 `SLOT_HIDDEN_DIM` (302) |
| `Src/Orchestrator/Orchestrator_VM/inference_handler.rs` | 新增 `handle_profile`; `handle_create_session` 顺带写入 `SLOT_HIDDEN_DIM` |
| `Src/Orchestrator/Orchestrator_VM/engine.rs` | step() 新增 Profile 分发 |
| `Src/Orchestrator/program_selector.rs` | 新增 Profile 模板常量; `select()` 新增 Profile 分支; `load_ml_program_vm` 新增 vars 参数; 新增 FillTensor/shape_slots 解析 |
| `Src/Orchestrator/core/branch_command.rs` | 新增 Profile 命令分支 |
| `Src/Orchestrator/command.rs` | `UserCommand` 新增 `Profile` |
| `Src/PeerManagement/*` | `PeerCapability` 新增 `layer_time: HashMap<String, Duration>`; `PeerManager` 新增 `update_capability` |
| `Src/Network/*` | 新增 `ProfileRequest`/`ProfileResponse` |

## 12. 不影响的部分

- `session.rs` — 不变
- `capability.rs` — 不变
- `service.rs` — 不变
- `ml_vm/slots.rs` — 不变
- `pipeline.rs` — 不变
- `handle_run_program` — 完全不动
- `scheduler_handler.rs` — 不动（Scheduler 读取 layer_time 是后续修改）

## 13. 实施步骤

### Phase 1 — ML 层 FillTensor 指令
1. `Src/ML_Engine/ML_VM/instruction.rs` — 新增 `FillTensor` 变体
2. `Src/ML_Engine/ML_VM/engine.rs` — 新增 `handle_fill_tensor` + step 分发
3. `cargo test --lib ml_vm` 验证

### Phase 2 — ML 程序模板 + ProgramSelector
4. 新建 `programs/ml/profile.tmpl`
5. `Src/Orchestrator/program_selector.rs`:
   - 新增 `ML_PROFILE_TMPL` 常量
   - `RawMLInstruction` 新增 `shape_slots: Option<Vec<String>>` 字段
   - `load_ml_program_vm` 新增 `vars` 参数
   - `convert_ml_instruction_vm` 新增 `FillTensor` 解析
   - `resolve_ml_const_value` 支持 `$hidden_dim` 从 vars 查找
6. `cargo test --lib program_selector` 验证

### Phase 3 — Orchestrator 层 Profile 指令
7. 新建 `programs/orchestrator/Profile.tmpl`
8. `Src/Orchestrator/Orchestrator_VM/instruction.rs` — 新增 `Profile`
9. `Src/Orchestrator/Orchestrator_VM/slots.rs` — 新增 `SLOT_HIDDEN_DIM`
10. `Src/Orchestrator/Orchestrator_VM/inference_handler.rs` — 新增 `handle_profile`; `handle_create_session` 写入 hidden_dim
11. `Src/Orchestrator/Orchestrator_VM/engine.rs` — step() 新增分发
12. `Src/Orchestrator/job.rs` — `JobKind::Profile`
13. `Src/Orchestrator/program_selector.rs` — `select()` Profile 分支
14. `cargo test --lib orchestrator_vm` 验证

### Phase 4 — TUI + PeerManager + Network
15. `Src/Orchestrator/command.rs` — `UserCommand::Profile`
16. `Src/Orchestrator/core/branch_command.rs` — Profile 分支
17. `Src/PeerManagement/*` — `PeerCapability.layer_time` + `update_capability`
18. `Src/Network/*` — `ProfileRequest`/`ProfileResponse`
19. `cargo test` 全量验证

## 14. Profile 内存查询

### 14.1 背景

Profile 除了测试节点推理性能（layer_time），还需记录节点剩余可用内存，供 Scheduler 后续做资源感知的层划分。剩余内存是模型加载前的空闲值——Profile 完成后模型会被卸载。

### 14.2 查询顺序

```
查询剩余内存 → Create_Session（加载模型） → Run_Program_VM（测 layer_time） → Shutdown_Session（卸载）
```

**先查内存，再加载模型**，确保拿到真实空闲内存，避免模型加载后的占用干扰结果。

### 14.3 按设备查询

`handle_profile` 中 `device` 参数已从模板传入（SLOT_DEVICE），按设备分发：

| 设备 | 查询内容 | 库 |
|------|---------|-----|
| CPU | 系统 DRAM 空闲内存 | `sysinfo` crate → `System::available_memory()` |
| CUDA | GPU VRAM 空闲内存 | `cudarc` crate → `driver::result::mem_get_info()` |

```rust
// handle_profile 中的调用位置（Create_Session 之前）
let device = self.vm.slots.get_string(SLOT_DEVICE)?.to_lowercase();
let free_memory_mb = match device.as_str() {
    "cuda" => query_cuda_memory()?,
    _      => query_system_memory(),
};
```

### 14.4 数据流

**本机 Profile**：
```
查询内存 → free_memory_mb → 写入 PeerCapability.memory_mb（Update_Capability）
```

**远程 Profile**：
```
ProfileRequest → 远端查询内存 → ProfileResponse "OK|{layer_time}|{memory_mb}"
→ 请求方解析 → 写入 PeerCapability.memory_mb + PeerInfo.bandwidth_mbps
```

PeerCapability 已有 `memory_mb: u64` 字段，无需新增。

### 14.5 实施计划

| 步骤 | 文件 | 改动 |
|------|------|------|
| 1 | `Cargo.toml` | 新增 `sysinfo`、`cudarc` 依赖 |
| 2 | `Src/Orchestrator/Orchestrator_VM/inference_handler.rs` | `handle_profile` 中 Create_Session 前查询内存，写入 PeerCapability |
| 3 | `Src/Orchestrator/core/branch_command.rs` | Profile_Request handler 返回格式扩展 `OK|{layer_time}|{memory_mb}` |
| 4 | `Src/Orchestrator/Orchestrator_VM/inference_handler.rs` | peer 回包解析增加 memory_mb，写入 PeerCapability |
| 5 | `cargo test` | 全量验证
