# 规则文件

agent必须严格遵守以下规则



# 思考模式
1. agent必须用中文进行思考

# 代码规范
## 归属规范
每一份代码文件必须在开头添加以下注释,注释规范应当严格对应当前语言：
'''
Presented by KeJi
Date ： Current date
'''
## 命名规范
1. 变量 采用lower snake case规范进行命名
2. 函数 采用Pascal snake case规范进行命名
3. 常数 采用Upper snake case规范进行命名

## Lua 绑定规范
1. 禁止在 mlua 闭包（`create_function`、`create_async_function`、`add_method`、`add_method_mut`）内编写业务逻辑。
2. 所有业务逻辑必须提取为独立的 Rust 函数（`fn` 或 `impl` 方法），闭包仅负责：
   - Lua 类型 → Rust 类型转换（`AnyUserData::borrow`、参数解包）
   - 调用提取好的 Rust 函数
   - `map_err` 转换为 `mlua::Error`
3. 正例：
   ```rust
   // MlSession 独立实现
   impl MlSession {
       pub fn encode(&self, text: &str) -> Result<Vec<u32>, String> { ... }
   }
   // 闭包只做薄胶水
   methods.add_method("encode", |_, sess, text: String| {
       sess.encode(&text).map_err(|e| mlua::Error::runtime(e))
   });
   ```
4. 反例：
   ```rust
   // ❌ 17 行业务逻辑直接写在闭包内
   methods.add_method("analyze_model", |lua, path: String| {
       let info = ...;  let t = lua.create_table()?;
       t.set("architecture", info.architecture)?;
       t.set("num_layers", info.num_layers)?;  // 共 15 个字段
       ...
   });
   ```


# 工作流程

## 目录结构
- `Task/` — 当前待执行/进行中的任务文件
  - `archived task/` — 已完成的任务归档
- `Workbook/` — 工作记录（命名格式 `wb_{id}_{description}.md`）
- `docs/` — 设计文档、说明文档

## 任务文件规范
每个任务文件命名格式：`task_{id}_{description}.md`，存储在 `Task/` 目录下。
- 例如：`Task/task_6_session_manager.md`
- 已完成的任务移入 `Task/archived task/`
- agent 只有在人类同意的情况下才能对任务文件进行修改。

## Workbook 规范
Workbook 目录中的工作记录文件与任务文件一一对应，命名方式为 `wb_{id}_{description}.md`（与 `task_{id}_{description}.md` 共享相同的 id 和 description）。用于 agent 记录工作进度，作为工作上下文。agent 在完成每一项任务后必须记录必要信息和重要细节。若文件不存在，agent 应创建一个。采用最高效、最精简的记录方式，无需考虑人类可读性。确保若启动新的 agent，其可基于现有 workbook 文件快速切换至当前工作上下文。

## Agent 工作流程
1. 阅读 `Task/` 目录下的任务文件，根据完成情况和人类评审意见决定下一项任务
2. 更新对应 workbook，记录任务开始时间
3. 根据人类的要求执行特定任务
4. 更新 workbook，记录依赖关系、结束时间，使用最精简的方式记录必要信息
5. 若任务完成，将任务文件移入 `Task/archived task/`
6. 结束任务







# 核心架构原则

## Core 主循环设计原则

Core 的 `tokio::select!` 主循环（5 个 branch）**必须是纯路由层**：

- **只做匹配 + spawn**：每个 branch 只负责识别事件类型并分发，不得在此执行任何业务逻辑或 I/O 操作。
- **毫秒级返回**：任何可能阻塞的操作（文件 I/O、网络传输、模型加载等）必须通过 `tokio::spawn`（或 `std::thread::spawn`）异步化。
- **锁的持有时间最短**：若需持有 `Mutex`，只在 HashMap 插入/删除等瞬时操作期间持有，不得在持有锁期间做 I/O。

```rust
// ✅ 正确：主循环只做路由
UserCommand::Session { model_id } => {
    let session_mgr = self.session_mgr.clone();
    tokio::spawn(async move {
        let id = session_mgr.lock().unwrap().create_session(&model_id);
        // ...
    });
}

// ✅ 正确：Stream 接收也 spawn 出去
Network_Inbound_Event::FileStreamArrived { peer, stream } => {
    let caps = self.capabilities.clone();
    tokio::spawn(async move {
        // 文件接收的全部 I/O 在这里
    });
}

// ❌ 错误：在主循环内做网络 I/O
Network_Inbound_Event::FileStreamArrived { peer, mut stream } => {
    Read_File_Stream_Header(&mut stream).await;   // 阻塞主循环！
}
```

---

# 分布式推理测试验证流程

## 测试环境

- 测试目录：`/vepfs-mlp2/c20250205/240804016/Test_Environment/`
- 节点 1/2/3 对应子目录 `1/`、`2/`、`3/`
- 每节点含：`Pleiades` binary、`.config/`、`Pleiades_Workspace/`、`programs/`、`Log/`
- 模型：`Qwen3-30B-A3B-Q4_K_M.pgguf`（18.5GB），三个节点均有副本
- 共 4 张物理 GPU（编号 0-3）
- chat 客户端：`Test_Environment/1/Tool/chat.py`

## 步骤 0：编译 & 同步

```bash
cd /root/code/Pleiades
cargo build --release

cp target/release/Pleiades /vepfs-mlp2/c20250205/240804016/Test_Environment/1/
cp target/release/Pleiades /vepfs-mlp2/c20250205/240804016/Test_Environment/2/
cp target/release/Pleiades /vepfs-mlp2/c20250205/240804016/Test_Environment/3/

cp -r programs/* /vepfs-mlp2/c20250205/240804016/Test_Environment/1/programs/
cp -r programs/* /vepfs-mlp2/c20250205/240804016/Test_Environment/2/programs/
cp -r programs/* /vepfs-mlp2/c20250205/240804016/Test_Environment/3/programs/
```

---

## 测试 1：双卡模式（2 节点）

### GPU 分配

| 节点 | 物理 GPU | CUDA_VISIBLE_DEVICES |
|------|---------|---------------------|
| A (目录1) | 0, 1 | `0,1` |
| B (目录2) | 2, 3 | `0,1` |

### 启动

| 终端 | 命令 |
|------|------|
| T1 | `cd /vepfs-mlp2/c20250205/240804016/Test_Environment/1 && CUDA_VISIBLE_DEVICES=0,1 ./Pleiades` |
| T2 | `cd /vepfs-mlp2/c20250205/240804016/Test_Environment/2 && CUDA_VISIBLE_DEVICES=0,1 ./Pleiades` |

### T1 操作序列

```
session create Qwen3-30B-A3B-Q4_K_M.pgguf
  → session_id = 1

session inference pipeline 1 Qwen3-30B-A3B-Q4_K_M.pgguf
  → 等待 "流水线就绪"

api 1
  → API 启动在 http://127.0.0.1:{port}
```

### 多轮对话 (chat.py)

```bash
cd /vepfs-mlp2/c20250205/240804016/Test_Environment/1/Tool/
python chat.py --url http://127.0.0.1:{port} -s "你是一个乐于助人的助手"
```

```
> 你好，接下来你的每一句话要以meow~结尾
  ← 应回复: ...meow~

> 今天天气真好，我们去散步吧
  ← 上下文记忆: 仍以 meow~ 结尾

> 请背诵一下静夜思
  ← 长文本 + meow~

> /clear
  ← 历史清空

> 你最喜欢的动物是什么？
  ← /clear 后不再带 meow~（验证上下文已清除）

> /exit
```

### 退出

```
T1> quit
T2> quit
```

---

## 测试 2：单卡模式（3 节点）

### GPU 分配

| 节点 | 物理 GPU | CUDA_VISIBLE_DEVICES |
|------|---------|---------------------|
| A (目录1) | 0 | `0` |
| B (目录2) | 1 | `0` |
| C (目录3) | 2 | `0` |

### 启动

| 终端 | 命令 |
|------|------|
| T1 | `cd /vepfs-mlp2/c20250205/240804016/Test_Environment/1 && CUDA_VISIBLE_DEVICES=0 ./Pleiades` |
| T2 | `cd /vepfs-mlp2/c20250205/240804016/Test_Environment/2 && CUDA_VISIBLE_DEVICES=0 ./Pleiades` |
| T3 | `cd /vepfs-mlp2/c20250205/240804016/Test_Environment/3 && CUDA_VISIBLE_DEVICES=0 ./Pleiades` |

### T1 操作序列

```
session create Qwen3-30B-A3B-Q4_K_M.pgguf
  → session_id = 1

session inference pipeline_coord_single 1 Qwen3-30B-A3B-Q4_K_M.pgguf
  → 等待模型加载完成

api 1
  → API 启动
```

### 多轮对话

```bash
cd /vepfs-mlp2/c20250205/240804016/Test_Environment/1/Tool/
python chat.py --url http://127.0.0.1:{port} -s "你是一个乐于助人的助手"
```

```
> 你好，接下来你的每一句话要以meow~结尾
  ← 应回复: ...meow~

> 用一句话总结一下量子力学
  ← 上下文记忆: 仍以 meow~ 结尾

> /clear
  ← 历史清空

> 你最喜欢什么颜色？
  ← /clear 后不再带 meow~（验证上下文已清除）

> /exit
```

### 退出

```
T1> quit
T2> quit
T3> quit
```

---

## 验证要点

| 验证项 | 测试 1 双卡 | 测试 2 单卡 |
|--------|:---:|:---:|
| 节点 mDNS 自动发现 | ✓ | ✓ |
| 模型加载成功 | ✓ | ✓ |
| 流水线桥接建立 | ✓ | ✓ |
| 首 token 正常生成 | ✓ | ✓ |
| 流式输出逐 token 显示 | ✓ | ✓ |
| 多轮上下文记忆（meow~） | ✓ | ✓ |
| /clear 后上下文正确清除 | ✓ | ✓ |
| quit 正常退出 | ✓ | ✓ |
