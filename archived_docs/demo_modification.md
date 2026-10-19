# Demo 分支修改文档

> Presented by KeJi
> Date: 2026-06-02

---

## 概述

对 `demo` 分支进行一系列修复，确保分布式流水线推理 (`pipeline_coord` + `pipe_worker`) 端到端可运行。

**修改范围**: 1 个 Rust 源文件 + 8 个 Lua 脚本，共 +387 行 / -83 行。

---

## 一、`Src/VM/capability_binding.rs` — `list_model_peers` 重写

### 问题

1. **coordinator 自身模型不可见**: local peer 的 `supported_models` 依赖异步 `flush` 填充，pipeline 脚本调用 `list_model_peers` 时可能尚未完成，导致 coordinator 持有的分片在链条中缺失
2. **层坐标偏移 (93 ≠ 95 误报)**: `layer_bitmap` 只编码 `blk.*` transformer blocks，不含 embedding (layer 0) 和 output (layer N+1)。pipeline_coord 使用 `total_layers = num_layers + 2` 校验，产生 2 层偏差
3. **同节点重叠分片未去重**: 同一设备上存在重叠的分片文件时（如 `_split_0_30` 和 `_split_0_47`），`list_model_peers` 全部返回，导致 pipeline 链条验证失败

### 修复

新增 3 个 helper 函数 + 4 步处理流程：

```
parse_split_range(name)        — 从文件名 _split_S_E.pgguf 解析 pipeline 坐标
bitmap_range(bitmap)           — 从 256-bit 位图提取 min/max blk 索引
adjust_range(min,max,file,nl)  — 修正层坐标：优先文件名解析 → 非split全覆盖 → 兜底bitmap

Step 1: 查询 Storage，收集匹配 model_id 的本地文件 → HashMap<file_name, num_layers>
Step 2: 遍历所有 peer 的 supported_models，按 model_id 过滤 → Vec<RawEntry>
Step 3: 补充 Storage 中未在 peer 条目出现的本地文件（解决 coord 不可见）
Step 3.5: 按 peer_id 分组，重叠条目取并集 + 保留最宽分片 file_name（互补条目保持独立）
Step 4: 构建 Lua 结果表（layer_start/layer_end 已为 pipeline 坐标）
```

---

## 二、`programs/user/pipeline_coord.lua` — Coordinator 脚本

### 问题

1. **向自己 `send_data`**: coordinator 在 peers 列表中时，对本地 peer 发送 `EXEC|pipe_worker|...`，导致 `DialFailure`
2. **不验证响应**: `send_data` 返回后未检查远程是否成功执行，失败时仍继续打开 tensor 流
3. **JSON 含空字符串**: `upstream or ""` 导致 JSON 中出现 `"upstream":""` 空串字段
4. **硬编码 `peers[1]` / `peers[N]`**: 当 coordinator 为第一/最后一个 peer 时，`open_tensor_stream` 向自己开流
5. **无 EOF 传播**: 推理结束后 coordinator 直接退出，worker 端 `accept_tensor_stream` 需等 120s 超时

### 修复

| 行号 | 修复内容 |
|------|----------|
| 148 | 阶段 4 获取 `my_id` |
| 151-153 | `if p.peer_id == my_id then goto continue` 跳过本机 |
| 156-167 | JSON 构造：只拼入有值的字段（省略空的 upstream/downstream），参照 `pipe_1.lua` 模式 |
| 170-186 | `pcall` 包裹 `send_data`，检查 `resp.payload == "OK"`，失败时 `handle:release()` + `return` |
| 189-200 | `fwd_peer` / `bwd_peer`：从 peers 中扫描第一个/最后一个远程节点 |
| 203-208 | `open_tensor_stream(fwd_peer)` / `accept_tensor_stream` |
| 266-268 | 桥接循环结束后 `caps.network.send_eof(fwd)` 通知链尾 |

---

## 三、`programs/user/pipe_worker.lua` — Worker 脚本

### 问题

Worker 收到 EOF 后退出循环，但未通知下游 worker，导致链上后续节点持续等待。

### 修复

| 行号 | 修复内容 |
|------|----------|
| 119-123 | 推理循环退出后 `if bwd then caps.network.send_eof(bwd)` 级联传播 EOF |

---

## 四、其余脚本（同类修复）

以下 6 个脚本进行了相同的防御性修复（跳过本机 + pcall + 不含空串 JSON + 找远程节点建流）：

| 脚本 | 说明 |
|------|------|
| `pipeline_coord_single.lua` | 单 GPU worker 版 coordinator |
| `exp_scale.lua` | 扩展性实验 coordinator |
| `exp_single.lua` | 单 GPU 实验 coordinator |
| `exp_qwen_3.lua` | 3 节点 Qwen 实验 |
| `exp_qwen_4.lua` | 4 节点 Qwen 实验 |
| `exp_qwen_5.lua` | 5 节点 Qwen 实验 |

---

## 五、已验证正确的逻辑

以下逻辑经审查确认无需修改：

| 组件 | 说明 |
|------|------|
| `split.lua` 均分计算 | 纯整数 `math.floor(total_layers/num)` + 余数前分配，无浮点精度问题 |
| `offsload_pingpong.lua` | `mid = floor(total/2)` 正确，无重叠无间隙 |
| `local_coord.lua` / `local_work.lua` | `mid = floor(N/2)` 正确 |
| pipeline 链条验证 | 首 0、尾 `total_layers-1`、中间 `curr_end+1 == next_start` 均正确 |
| 上游/下游 peer_id 链 | 含本机的排序列表中计算 upstream/downstream，即使 coordinator 被跳过，peer_id 仍然正确关联 |

---

## 六、修复链路

```
修复前:
  list_model_peers → 缺本地 + 坐标偏移(93≠95) + 重叠未去重
       ↓
  pipeline_coord → DialFailure(对自己发) + 不验响应 + 硬编码peers[1]
       ↓
  pipe_worker → 无限等待(无EOF) → 120s超时

修复后:
  list_model_peers → 含本地 + pipeline坐标 + 重叠自动合并
       ↓
  pipeline_coord → 跳本机 + 验OK + 动态找远程 + send_eof
       ↓
  pipe_worker → EOF级联传播 → 链上全员优雅退出
```
