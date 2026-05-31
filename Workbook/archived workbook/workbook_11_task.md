# Workbook — Task 11: Branch and ML Refine

## 2026-05-29

### 11.1 Split Model PGGUF ✅
- GGUF_Split_Model metadata: skip old layer_bitmap, recompute from selected_tensor_names
- commit: bbbbf2b

### 11.2 Network Command Branch ✅
- command.rs: +ExecRemote (NetworkProtocol + UserCommand) + parse/serialize + tests
- branch_user.rs: +B1 ExecRemote dispatch
- branch_command.rs: B2 DataType::Command full implementation (replaced stub)
- TUI: +rexec <peer> <cmd> [k=v]
- commit: bbbbf2b (original), fafad5f (refactored to fire-and-forget)

### 11.3 Session Inference Custom ML Thread ✅
- inference.lua → user/single_inf.lua (COMMAND=single_inf)
- SessionInference: +command field
- B1: get("inference") → get(&command)
- TUI: 3-param + 2-param fallback
- commit: 9ef33fa

### 11.4 Split.lua + keep_tokenizer ✅
- GGUF_Split_Model: +keep_tokenizer: bool
- capability::split_model + Lua binding: +keep_tokenizer param
- programs/user/split.lua: even split with tokenizer on part 0
- Fix: stem regex for backslash paths (0cf598a, fed7e7c)
- Fix: storage_acquire_write for output registration + flush (19875a4)
- commit: 97302f0 (base), 19875a4 (storage fix)

### 11.5 pipe_1/pipe_2 (distributed pipeline) ✅
- capability_binding.rs: +caps.network.get_all_peers()
- pipe_1.lua: load first half, find remote peer, rexec pipe_2, bridge Session ↔ pipe_2
- pipe_2.lua: load second half, receive hidden, forward, return logits
- commit: 706b87c

### 11.6 pipe_3/pipe_4 (CPU/GPU hybrid) ✅
- pipe_3.lua: CPU(split_start..mid) + GPU(mid+1..split_end) → network
- pipe_4.lua: network → CPU forward → GPU forward → return
- commit: c183279

## Fixes
- TUI peer name: name#XXXX format (47feb2a)
- B2 rexec: fire-and-forget instead of 60s wait (fafad5f)
- pipe_2/pipe_4: KV cache reset on offset==0 (0076709)
- split.lua: backslash path compatibility (0cf598a, fed7e7c)

## New Risks
- #22: ml.new("cuda") hardcoded cuda:0, no multi-GPU
- #23: qwen3.rs Dense only, no MoE support

## 2026-05-31
- Created docs/speedup.md (mistral.rs + Crane optimization strategies)
