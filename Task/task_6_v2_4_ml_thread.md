# Task 6 v2.4: ML Thread 接入 — 单轮对话闭环

> Presented by KeJi
> Date: 2026-05-25

---

## 背景

当前链路：

```
✅ Chat → Session: prompt 输入 → encode → EventBus 打印 token count
❌ Session → ML Thread: 不存在
```

本阶段目标：**闭合单轮推理链路**。

```
Prompt> 你好
  → Chat ──"chat-{id}"──► Session
                             ├─ encode → tokens
                             ├─ tensorize → Tensor
                             └─ send_frame("ml-{id}") ──► ML Thread
                                                            ├─ recv tensor
                                                            ├─ forward → logits
                                                            └─ send_frame ──► Session
                                                                                ├─ sample → token
                                                                                ├─ decode → text
                                                                                └─ EventBus 发布
```

---

## 设计

### 连接架构

```
                    LocalStreamHub
                         │
         ┌───────────────┼───────────────┐
         │               │               │
  "chat-{id}"            │        "ml-{id}"
  (chat task open)       │   (ML Thread open, Session accept_async)
                          │
                     Session.spawn()
                     task (select!)
```

Session 同时管理两条流：一条收 prompt，一条收 logits。ML Thread 通过 Lua 脚本启动。

### Lua 脚本：`inference.lua`

```lua
COMMAND = "inference"
DESCRIPTION = "启动 ML Thread，连接到指定 Session 并执行 forward"

-- 用法: exec inference session_id=<id> model_path=<path>
function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    -- 1. 通过 Storage 获取模型路径
    local handle = caps.storage_acquire_read(model_path)
    local path = handle:path()

    -- 2. 加载模型
    local sess = ml.new("cpu")
    sess:load_model(path, 0, 999999)

    handle:release()

    -- 3. 连接 Session
    local stream = local_tensor.open_stream("ml-" .. session_id)

    -- 4. forward loop
    while true do
        local tensor, offset = local_tensor.recv_tensor(stream, "cpu")
        local logits = sess:forward(tensor, offset)
        local_tensor.send_tensor(stream, logits, offset)
    end
end
```

### Session.spawn() 改动

当前：`loop { recv from chat → encode → EventBus }`

改为：`select! { chat.recv, ml.accept/recv }`

```rust
tokio::spawn(async move {
    // 1. 加载 tokenizer（不变）
    // 2. accept_async("chat-{id}") — 等 chat（不变）

    // 3. 等 ML Thread 连接
    let ml_id = format!("ml-{}", session_id);
    let mut ml_stream = stream_hub.accept_async(&ml_id, 60_000).await?;

    // 4. select! loop
    let mut buf = Tensor_Buffer::New(4096);
    let mut ml_buf = Tensor_Buffer::New(1024 * 1024);
    loop {
        select! {
            // 收 prompt 来自 chat
            result = local_recv_frame(&mut chat_stream, &mut buf) => {
                let text = ...;
                let tokens = ml.encode(&text)?;
                let tensor = ml.tensorize(&tokens)?;
                // 序列化 tensor → bytes → send to ml_stream
                local_send_frame(&mut ml_stream, 0, &tensor_bytes).await?;
            }

            // 收 logits 来自 ML Thread
            result = local_recv_frame(&mut ml_stream, &mut ml_buf) => {
                let logits_tensor = deserialize(ml_buf);
                let token = ml.sample(&logits_tensor, 0.8)?;
                let text = ml.decode(token)?;
                event_bus.Publish("Session {id}: {text}");
            }
        }
    }
});
```

---

## 改动清单

### 1. `session.rs` — spawn() 改为 select! + ML 流

| 改动 | 说明 |
|------|------|
| 签名不变 | `spawn(stream_hub, event_bus, storage)` |
| 新增 `accept_async("ml-{id}")` | 等 ML Thread 连入 |
| `loop { recv }` → `select! { chat, ml }` | 双向收发 |
| chat 分支 | encode → tensorize → 序列化 tensor → send ML |
| ml 分支 | 反序列化 logits → sample → decode → EventBus |

> **tensor 序列化**：Session 调用 `tensorize()` 得到 `candle_core::Tensor` 后需要序列化为 bytes 才能通过 `local_send_frame` 发送。可以复用已有的 `LuaTensor` 序列化机制（`to_vec` → bytes），ML Thread 侧用 `tensor_from_bytes` 反序列化。

### 2. `programs/user/inference.lua` — 新 Lua 脚本

新建文件，内容如上设计。

### 3. TUI 不需要改动

用户通过 `exec inference session_id=1 model_path=test.pgguf` 触发。

---

## 实施步骤

| 步骤 | 内容 | 文件 |
|:--:|------|------|
| 1 | 创建 `ml-thread` 分支 | — |
| 2 | 写 `inference.lua` | `programs/user/inference.lua` |
| 3 | 改造 `Session::spawn()` — select! + ML 流 | `session.rs` |
| 4 | `cargo test --lib` 全量通过 | — |
| 5 | 合并回 `session-manager-reforge` | — |

---

## 后续（本阶段不做）

- 自回归多 token 生成
- EOS 检测停止
- reply 通道：ML 结果返回 chat task 显示在 Command Output 区
- 多 slot 支持
- 远端 ML Thread 接入

---

## 人类评审

<!-- 在此区域写下评审意见 -->

