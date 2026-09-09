-- Presented by KeJi
-- Created Date: 2026-09-09
-- Modified Date: 2026-09-09

-- 预分片流水线 Coordinator（终端跑）
-- 用法: session inference llm <session_id> <model_path>
--   model_path 为完整模型文件名（coord 用它拿 model_id + tokenizer 做 encode/decode）
-- 流程: 发现持有该 model_id 的分片节点 → 分发 llm_worker → 桥接 Session

COMMAND = "llm"
DESCRIPTION = "预分片流水线协调者: 发现分片节点 → 分发 llm_worker → 桥接 Session"

-- ═══ 配置区 ═══
local device = "cpu"   -- coord 桥接 tensor 的中转设备: cpu / cuda:0

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    if not session_id or not model_path then
        caps.print("[llm] 缺少参数 session_id/model_path")
        return
    end

    local sid_num = tonumber(session_id)
    caps.print("")
    caps.print("╔══════════════════════════════════════════════╗")
    caps.print("║   Pleiades 预分片流水线（分布式推理）        ║")
    caps.print("╚══════════════════════════════════════════════╝")
    caps.print("")

    -- 阶段 1: 模型识别（完整模型）
    caps.print("┌─ 阶段 1/3: 模型识别 ─────────────────────────┐")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)
    local model_id = info.model_id
    local total_layers = info.num_layers + 2

    if not model_id then
        caps.print("│ ✗ 模型文件缺少 model_id")
        caps.print("└──")
        handle:release()
        return
    end
    caps.print(string.format("│ 模型: %s  架构: %s  总层: %d  ID: %d",
        model_path, info.architecture, total_layers, model_id))
    caps.print("└──")
    caps.print("")

    -- 阶段 2: 发现分片节点
    caps.print("┌─ 阶段 2/3: 发现分片节点 ────────────────────┐")
    local all_peers = caps.network.list_model_peers(model_id)
    local my_id = caps.network.get_local_peer_id()

    local remote_peers = {}
    for _, p in ipairs(all_peers) do
        if p.peer_id ~= my_id then table.insert(remote_peers, p) end
    end

    if #remote_peers == 0 then
        caps.print("│ ✗ 没有远程分片节点")
        caps.print("└──")
        handle:release()
        return
    end

    -- 按 peer_id 排序，确保链顺序确定
    table.sort(remote_peers, function(a, b) return a.peer_id < b.peer_id end)

    caps.print(string.format("│ 发现 %d 个远程分片节点:", #remote_peers))
    for i, p in ipairs(remote_peers) do
        caps.print(string.format("│   [%d] %s  %s", i, p.name, p.file_name))
    end
    caps.print("└──")
    caps.print("")

    -- 阶段 3: 分发 worker + 桥接 Session
    caps.print("┌─ 阶段 3/3: 分发 + 推理 ──────────────────────┐")

    local N = #remote_peers
    for i, p in ipairs(remote_peers) do
        local upstream = (i > 1) and remote_peers[i - 1].peer_id or nil
        local downstream = (i < N) and remote_peers[i + 1].peer_id or nil

        local fields = {}
        if upstream then fields[#fields+1] = string.format('"upstream":"%s"', upstream) end
        if downstream then fields[#fields+1] = string.format('"downstream":"%s"', downstream) end
        fields[#fields+1] = string.format('"coordinator":"%s"', my_id)
        fields[#fields+1] = string.format('"session_id":"%s"', session_id)
        local payload = "EXEC|llm_worker|{" .. table.concat(fields, ",") .. "}"

        caps.print(string.format("│   [%d/%d] → %s  llm_worker", i, N, p.name))
        local ok, resp = pcall(caps.network.send_data, p.peer_id, "Command", payload)
        if ok then
            caps.print(string.format("│         响应: %s", resp.payload or "nil"))
        else
            caps.print(string.format("│         失败: %s", tostring(resp)))
            caps.print("└──")
            handle:release()
            return
        end
    end

    caps.print("│")
    caps.print("│ 建立张量流 ...")
    local fwd = caps.network.open_tensor_stream(remote_peers[1].peer_id, sid_num)
    caps.print(string.format("│   fwd: coord → %s  ✓", remote_peers[1].name))
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print(string.format("│   bwd: %s → coord  ✓", remote_peers[N].name))
    caps.print("│")

    -- 连接 Session
    local stream_id = "ml-" .. session_id
    local session_stream = local_tensor.open_stream(stream_id)
    caps.print("│ Session ↔ coord  ✓")
    caps.print("│ ╔══════════════════════════════════╗")
    caps.print("│ ║  ✓ 流水线就绪，开始推理          ║")
    caps.print("│ ╚══════════════════════════════════╝")
    caps.print("└──")
    caps.print("")

    -- 桥接循环: Session → 链头 → ... → 链尾 → Session
    local iter = 0
    while true do
        local ok, tensor, recv_offset = pcall(function()
            return local_tensor.recv_tensor(session_stream, device)
        end)
        if not ok then caps.print("[llm] Session 流结束"); break end

        iter = iter + 1
        local t0 = os.clock()

        caps.network.send_tensor(fwd, tensor, recv_offset)
        local logits, _ = caps.network.recv_tensor(bwd, device)
        local_tensor.send_tensor(session_stream, logits, recv_offset)

        local elapsed = os.clock() - t0
        if iter == 1 then
            caps.print(string.format("[llm] Prefill 完成 (%.2fs)", elapsed))
        elseif iter % 10 == 0 then
            caps.print(string.format("[llm] %d tokens (%.2fs/tok)", iter, elapsed))
        end
    end

    caps.network.send_eof(fwd)
    handle:release()
    caps.print("[llm] 流水线演示结束")
end
