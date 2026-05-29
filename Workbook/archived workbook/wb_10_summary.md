# wb_10_summary — 2026-05-29 (基于最新代码重新审查)

## 事务 1: Architecture 文档更新 (重做)
- 完成: 2026-05-29
- 差异: 11🔴 + 17🟡 + 3🟢
- 主要修复: Network trait 完全重写、ML_Engine 4函数签名、Session_Manager trait→具体类型、TUI布局+Prompt行删除、VM Lua API修正、Config补全、B5语义修正、Capabilities类型

## 事务 2: README
- 前置操作已创建，本次检查确认项目结构包含新模块(API/)

## 事务 3: 清理
- __pycache__ 已在之前清理
- builtin legacy 脚本已归档

## 事务 4: Code Review (基于最新代码 e66e4a3)

> ✅ **已排除的误报**:
> - ~~ML_Engine 绕过 Storage~~ → Storage 返回 PathBuf，下游用 std::fs 读写是正确用法
> - ~~Lua 闭包违反绑定规范~~ → 闭包只做 Rust→Lua 类型转换，真正的业务逻辑在独立 fn 中

### 🟡 中等问题

|#|问题|位置|
|-|-|-|
|1|SessionManager::new()返回Arc<Mutex<>>藕合并发策略 | manager.rs:27-37 |
|2|LocalStreamHub.accept()使用thread::sleep忙等待 | local_tensor_stream/mod.rs:59-75 |
|3|SupportedModel构造逻辑重复(3处) | main.rs:163-199, branch_user.rs:306-316 |
|4|unsafe缺少SAFETY注释(4处) | lua_tensor.rs:75/81/121/127 |
|5|JobExecutor是空壳stub(run()立即返回Success) | job_executor.rs:49-54 |
|6|broadcast_local_info忽略所有错误 | Network/mod.rs:134-141 |
|7|Has_Active_Session()硬编码false | app.rs:351-353 |
|8|Mutex::lock().unwrap() poison panic风险 | branch_user.rs:379/384/431, branch_stream.rs:64 |

### 🟢 轻微问题

|#|问题|
|-|-|
|9|归属注释格式不一致(空格/全角半角冒号)|
|10|#![allow(non_snake_case)]/#![allow(dead_code)]范围过大(lib.rs crate级别)|
|11|StorageManager::New不填充模型元信息(等到flush)|
|12|PeerId::random()作为bootstrap(Kademlia路由问题)|
|13|slot_tokens[&slot_id]潜在panic(应用.get())|

### 已记入 potential_risk.md

|#|风险|位置|
|-|-|-|
|PR-21|std::sync::Mutex 阻塞 tokio worker 线程 | branch_user.rs:379-431, branch_stream.rs:64 |

### 规范符合度

|规范|符合度|备注|
|-|-|-|
|归属注释|🟡90%|格式不一致|
|命名规范|✅95%|allow(non_snake_case)遮蔽|
|Lua绑定(薄胶水)|✅|类型转换在闭包内，业务逻辑在独立fn|
|Storage API使用|✅|PathBuf返下游，std::fs是唯一读写方式|
|Core只做路由|✅100%|select!5分支严格遵守|

### 代码优点
1. Core主循环严格遵守"路由+spawn"原则
2. Storage锁系统设计完善(RAII+惰性发现+路径防护)
3. EventBus 4类型开闭原则
4. MlSession空壳支持load/unload/reload循环
5. GGUF→PGGUF自动转换+weight tying处理
6. Network层分层清晰(NodeHandle+stream::Control+RendezvousMap)
7. 测试覆盖全面(Storage 30+)
