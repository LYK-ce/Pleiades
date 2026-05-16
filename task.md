# Pleiades 任务列表

> Presented by KeJi
> Date: 2026-05-16

---

## Lua 脚本引擎迁移

> 用 mlua 替代全部自研 VM（Vm_Base + ML_VM + Orchestrator_VM）和 TOML 模板系统。
> 详见 docs/lua.md

### Lua 基础设施搭建 (Phase 1)

- [ ] Cargo.toml 添加 mlua 依赖
- [ ] 实现 Lua 沙箱环境 (sandbox)
- [ ] 实现 ProgramRegistry 脚本扫描
- [ ] 注册示范能力函数 (get_available_peers + analyze_model + plan_uniform)
- [ ] 编写 programs/pipeline.lua 替代 Pipeline.tmpl
- [ ] cargo test 全量通过验证

### 能力函数迁移 (Phase 2)

- [ ] 注册全部 A/B/C/D 级能力函数到 Lua
- [ ] 迁移全部 TOML 模板 → .lua 脚本
- [ ] Core::route_user 简化为统一执行分支
- [ ] JobKind 精简
- [ ] 删除 program_selector.rs + TOML 模板

### TUI 集成 (Phase 3)

- [ ] 命令补全数据源切换为 ProgramRegistry::command_names()
- [ ] reload 命令实现
- [ ] Commands_Reloaded 事件通知 TUI 刷新

### 遗留代码清理 (Phase 4)

- [ ] 删除 Vm_Base/ 整个目录
- [ ] 删除 ML_VM/ 中 instruction.rs / engine.rs / slots.rs
- [ ] 删除 Orchestrator_VM/ 中 instruction.rs / engine.rs / slots.rs
- [ ] 整理 Capabilities 结构体
- [ ] 全量测试回归

---

## Pipeline 端到端联调

- [ ] Coordinator + Worker 完整推理流程验证
- [ ] Core 分支完整拆分 (branch_user / command / stream / lifecycle)
- [ ] Relay Job 指令完善 (compile_relay)
- [ ] 模型分片分发端到端实现 (Distribute Job)

---

## Bug 修复 & 优化

### GGUF 模型加载优化

> 改用 Layer_Weights::New() 替代 From_Extracted()，单次打开文件顺序加载。
> 详见 classified_docs/problem.md 问题14

- [ ] 修改 gguf_model.rs 中 GGUF_Load_Model 函数
- [ ] 创建全局 Gguf 读取器，直接调用 Layer_Weights::New()

### GGUF_Model_Inference 接口修正

> 签名从 (model, &[u8], offset) 改为 (model, &Tensor, offset)。
> 详见 classified_docs/problem.md 问题13

- [ ] 修改 GGUF_Model_Inference 签名
- [ ] 移除 Token_Ids_To_Bytes() 辅助函数
- [ ] 更新 ml_thread_engine.rs 中 Prefill/Inference 指令执行逻辑
- [ ] 更新相关测试

### 心跳故障处理

> 详见 classified_docs/problem.md 问题15

- [ ] 滑动窗口机制：连续 N 次失败才标记 Disconnected
- [ ] 超时自动重连：指数退避重试
- [ ] 推理任务影响处理：通知 Control 层中断/重分配
- [ ] 心跳成功率监控统计
- [ ] 与 ConnectionClosed 事件协调（避免重复处理）

### PeerStore 提取

> 将 connected_peers 提取为 Arc<RwLock<HashMap>> 共享组件。
> 详见 classified_docs/problem.md 问题8

- [ ] 新建 PeerStore 组件 (Arc<std::sync::RwLock<HashMap<PeerId, PeerInfo>>>)
- [ ] Network_Service 持有写锁
- [ ] NodeHandle / Control / TUI 持有读锁
- [ ] 移除 NodeCommand::GetPeers / GetPeerInfo
- [ ] Get_Peers / Get_Peer_Info 改为同步方法

### pending_responses 超时清理

> 详见 classified_docs/problem.md 问题9

- [ ] pending 条目附带 created_at 时间戳
- [ ] Register/Route 操作时扫描清理过期条目
- [ ] 过期 pending_responses 通过 oneshot 发送超时错误
- [ ] 过期 pending_replies 直接丢弃 ResponseChannel

---

## 性能优化

### ML Engine 推理路径优化

> 数据平面与控制平面分离 + 零拷贝张量传输。
> 详见 classified_docs/problem.md 问题11

- [ ] 专用 Tensor Stream 通道 (与 problem 5 合并实施)
- [ ] Control 内部 fast-path (inference pipeline task)
- [ ] 零拷贝优化：bytes::Bytes 引用计数共享
- [ ] 零拷贝优化：bytemuck::cast_slice reinterpret
- [ ] 预分配 send_buffer 减少 GPU→CPU 转换

### mDNS 服务名自定义

> 避免发现不相关的 Pleiades 实例。
> 详见 classified_docs/problem.md 问题12

- [ ] 自定义 mDNS 服务名或集群标识机制

---

## 人类评审

<!-- 在此区域写下评审意见 -->
