# Task 6 v2.1: MlContext 重构 — 拆分 model 和 tokenizer

> Presented by KeJi
> Date: 2026-05-25

---

## 背景

当前 `MlSession` 的 `model` 字段（`Option<GGUF_Model>`）同时持有模型权重和 tokenizer，两者被强行耦合。但实际上：

- `encode()` / `decode()` **只用 tokenizer**，不碰权重
- `forward()` **只用权重**，不碰 tokenizer
- `tensorize()` / `sample()` **两者都不需要**

这导致一个场景无法优雅支持：Session 只需要 tokenizer 做 encode/decode，ML Thread 只需要权重做 forward。目前无法只加载 tokenizer 而不加载数 GB 的权重。

## 目标

在 `MlContext` 中将 `model` 和 `tokenizer` 拆分为两个独立字段，新增 `load_tokenizer()` 方法，让调用方按需加载。

## 改动清单

### 1. `Src/ML_Engine/context.rs` — 核心改动

#### 1.1 MlContext 结构体

```diff
  struct MlContext {
-     model: Option<GGUF_Model>,
+     model: Option<GGUF_Model>,
+     tokenizer: Option<shimmytok::Tokenizer>,
      offset: usize,
      rng_state: u64,
      eos_token_id: u32,
      device: Device,
  }
```

#### 1.2 encode() / decode() 改为检查 tokenizer

```diff
  pub fn encode(&self, text: &str) -> Result<Vec<u32>, String> {
-     let model = self.ctx.model.as_ref()
-         .ok_or("encode: no model loaded. Call load_model() first.")?;
-     GGUF_Encode(model, text)
+     let tokenizer = self.ctx.tokenizer.as_ref()
+         .ok_or("encode: no tokenizer loaded. Call load_tokenizer() first.")?;
+     // 使用 tokenizer 直接 encode，不再依赖 GGUF_Model
+     let format_prompt = format!("<|im_start|>user\n{text}<|im_end|>\n<|im_start|>assistant\n");
+     tokenizer.encode(&format_prompt, true)
+         .map_err(|e| format!("Tokenization error: {e}"))
  }
```

> **注意**：encode() 不再调用 `GGUF_Encode(model, text)`，而是直接用 tokenizer。`GGUF_Encode` 函数保留不动，但 MlSession 不再依赖它。

decode() 同理：

```diff
  pub fn decode(&self, token_id: u32) -> Result<String, String> {
-     let model = self.ctx.model.as_ref()
-         .ok_or("decode: no model loaded. Call load_model() first.")?;
-     GGUF_Decode(model, &[token_id])
+     let tokenizer = self.ctx.tokenizer.as_ref()
+         .ok_or("decode: no tokenizer loaded. Call load_tokenizer() first.")?;
+     let eos = self.ctx.eos_token_id;
+     let tokens = if token_id == eos { &[] as &[u32] } else { std::slice::from_ref(&token_id) };
+     tokenizer.decode(tokens, true)
+         .map_err(|e| format!("Decoding error: {e}"))
  }
```

#### 1.3 load_model() — 不再加载 tokenizer

移除 `GGUF_Load_Model` 内部的 tokenizer 加载逻辑（或改为不加载）。`load_model()` 只负责权重。

```diff
  pub fn load_model(&mut self, path: &Path, start: usize, end: usize) -> Result<(), String> {
      // 新模型加载成功后卸载旧模型
      let model = GGUF_Load_Model(start, end, path, &self.ctx.device)
          .map_err(|e| format!("Failed to load model: {e}"))?;
      if let Some(old) = self.ctx.model.take() {
          GGUF_Unload_Model(old);
      }
      self.ctx.model = Some(model);
-     self.ctx.eos_token_id = model.inference_config.eos_token;  // 不再从 model 取 eos
      self.ctx.offset = 0;
      Ok(())
  }
```

> **设计决定**：`load_model()` 不再设置 `eos_token_id`。`eos_token_id` 由 `load_tokenizer()` 负责设置。

#### 1.4 新增 load_tokenizer()

```rust
/// 仅加载 tokenizer，不加载模型权重。
///
/// 适合只需要 encode/decode 能力的场景（如 Session）。
pub fn load_tokenizer(&mut self, path: &Path) -> Result<(), String> {
    let tokenizer = shimmytok::Tokenizer::from_gguf_file(path)
        .map_err(|e| format!("Failed to load tokenizer: {e}"))?;

    // 同时从文件中解析 eos_token_id
    let arch_info = GGUF_Analyze_From_Content(&content)?; // 或调用已有方法
    self.ctx.eos_token_id = arch_info.eos_token_id;

    self.ctx.tokenizer = Some(tokenizer);
    Ok(())
}
```

> **实际实现**：需要读取 GGUF 文件的 metadata 获取 eos_token_id。可以复用 `GGUF_Analyze_And_Convert` 或直接读文件头。细节在实施时确定。

#### 1.5 unload() — 清理 tokenizer

```diff
  pub fn unload(&mut self) {
      if let Some(model) = self.ctx.model.take() {
          GGUF_Unload_Model(model);
      }
+     self.ctx.tokenizer = None;
      self.ctx.offset = 0;
      self.ctx.eos_token_id = 151645;
  }
```

#### 1.6 Lua 绑定 — 新增 load_tokenizer 方法

```rust
methods.add_method_mut(
    "load_tokenizer",
    |_, sess, path: String| {
        sess.load_tokenizer(std::path::Path::new(&path))
            .map_err(|e| mlua::Error::runtime(e))
    },
);
```

#### 1.7 测试更新

- 所有现有测试 -- `load_model()` 之后需要额外调 `load_tokenizer()` 才能使用 encode/decode
- 新增测试：`load_tokenizer` 后 encode/decode 可用且 forward 不可用
- 新增测试：只 `load_model` 后 forward 可用但 encode/decode 不可用

### 2. `Src/ML_Engine/gguf_model.rs`

- `GGUF_Load_Model` 不再加载 tokenizer（移除 `shimmytok::Tokenizer::from_gguf_file()` 调用）
- `GGUF_Model` 结构体中 `tokenizer` 字段可保留但不再填充（向后兼容，后续 Task 可清理）
- `GGUF_Encode` / `GGUF_Decode` 保持不变（仍有其他调用方如直接使用 `GGUF_Model` 的代码）

### 3. `Src/VM/capability_binding.rs`

- 文档注释更新：`load_model` 说明改为「只加载权重」，补充 `load_tokenizer` 说明

### 4. Lua 脚本（如存在）

- 调用 `sess:load_model(...)` 后需要追加 `sess:load_tokenizer(path)` 才能使用 encode/decode

---

## 实施计划

| 步骤 | 内容 | 文件 |
|:--:|------|------|
| 1 | 新建分支 `ml-context-reforge` | — |
| 2 | `MlContext` 加 `tokenizer` 字段 | `context.rs` |
| 3 | 重写 `encode()` / `decode()` 检查 `self.ctx.tokenizer` | `context.rs` |
| 4 | `load_model()` 移除 tokenizer + eos 设置 | `context.rs` |
| 5 | 新增 `load_tokenizer(path)` 方法 | `context.rs` |
| 6 | `unload()` 清理 `tokenizer` | `context.rs` |
| 7 | `GGUF_Load_Model` 不再加载 tokenizer | `gguf_model.rs` |
| 8 | Lua 绑定：新增 `load_tokenizer` 方法 | `context.rs` (add_methods) |
| 9 | 更新 Lua 绑定文档 | `capability_binding.rs` |
| 10 | 更新/新增测试 | `context.rs` |
| 11 | `cargo test` 全量通过 | — |
| 12 | 合并回 `session-manager-reforge` | — |

---

## 人类评审

<!-- 在此区域写下评审意见 -->

