# Task 6.1.i: Session Stream ACK 补全 + 死代码清理

> Presented by KeJi
> Date: 2026-05-24

---

## 目标

补全 Session Stream 协议中缺失的 ACK 环节，清理 Core 中 TensorStreamArrived 死代码分支。

---

## 问题

Session Stream 协议定义了 `[1B ACK]` 握手响应，但两端均未使用：
- **发起方** `open_session_stream` 写 handshake 后直接返回 stream，不读 ACK
- **接收方** Core 分配 slot 后直接桥接，不写 ACK

远端无法得知 slot 申请成功还是失败。

---

## 改动

### 1. 发起方 — `open_session_stream` 读 ACK

**文件**: `Src/Network/capability.rs`

写 handshake 后，读 1 字节 ACK。REJECT 则返回错误。

### 2. 接收方 — Core 写 ACK

**文件**: `Src/Orchestrator/core/branch_stream.rs`

收到 slot 分配结果后，写 ACK 给远端：
- `Ok(Ok(handle))` → 写 `ACCEPT(0x01)` → 桥接
- `Ok(Err(_))` / `Err(_)` → 写 `REJECT(0x00)`

### 3. 清理死代码

**文件**: `Src/Network/capability.rs`
删除 `TensorStreamArrived` 变体（从未被构造，纯死代码）

**文件**: `Src/Orchestrator/core/branch_stream.rs`
删除 `TensorStreamArrived` match 分支

### 4. 测试

- [ ] `cargo test` 全量通过

---

## 不做的

- 不重构 StreamHandler 统一抽象（2 种流不值得）
- 不改动 Bandwidth / Tensor Stream 流程

---

## 人类评审

<!-- 在此区域写下评审意见 -->

