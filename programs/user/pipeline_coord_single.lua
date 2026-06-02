-- Presented by KeJi
-- Date: 2026-06-02
-- 动态流水线 — 单卡 Coordinator (Session 模式)
-- 与 pipeline.lua 唯一区别: rexec 下发 pipe_worker_single
-- 用法: session inference pipeline_coord_single <session_id> <model_path>

COMMAND = "pipeline_coord_single"
DESCRIPTION = "动态流水线协调者(单卡): 发现→构建→桥接→推理"

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    if not session_id or not model_path then
        caps.print("[coord_single] 缺少参数")
        return
    end

    local sid_num = tonumber(session_id)
    caps.print("")
    caps.print("╔══════════════════════════════════════════════╗")
    caps.print("║   Pleiades 动态流水线 — 单卡模式演示         ║")
    caps.print("╚══════════════════════════════════════════════╝")
    caps.print("")

    -- 阶段 1: 模型识别
    caps.print("┌─ 阶段 1/4: 模型识别 ─────────────────────────┐")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)
    local model_id = info.model_id
    local total_layers = info.num_layers + 2

    if not model_id then
        caps.print("│ ✗ 错误: 模型文件缺少 model_id")
        caps.print("└──────────────────────────────────────────────┘")
        handle:release()
        return
    end

    caps.print(string.format("│ 模型: %s  id=%d  总层: %d", model_path, model_id, total_layers))
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- 阶段 2: 集群发现
    caps.print("┌─ 阶段 2/4: 集群发现 ─────────────────────────┐")
    local peers = caps.network.list_model_peers(model_id)
    if #peers == 0 then
        caps.print(string.format("│ ✗ 未找到 model_id=%d 的节点", model_id))
        caps.print("└──────────────────────────────────────────────┘")
        handle:release()
        return
    end
    caps.print(string.format("│ ✓ 发现 %d 个节点:", #peers))
    for i, p in ipairs(peers) do
        caps.print(string.format("│   [%d] %-20s  层 [%d,%d]  %s",
            i, p.name, p.layer_start, p.layer_end, p.file_name))
    end
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- 阶段 3: 构建链条
    caps.print("┌─ 阶段 3/4: 构建推理链条 ─────────────────────┐")
    table.sort(peers, function(a, b) return a.layer_start < b.layer_start end)

    local valid = true
    for i = 1, #peers - 1 do
        if peers[i].layer_end + 1 ~= peers[i + 1].layer_start then valid = false end
    end
    if peers[1].layer_start ~= 0 or peers[#peers].layer_end ~= total_layers - 1 then valid = false end

    if not valid then
        caps.print("│ ✗ 链条验证失败")
        caps.print("└──────────────────────────────────────────────┘")
        handle:release()
        return
    end
    caps.print(string.format("│ ✓ 链条: %d 节点 (单卡模式)", #peers))
    for i, p in ipairs(peers) do
        caps.print(string.format("│   [%d] %s  → 层 [%d,%d]", i, p.name, p.layer_start, p.layer_end))
    end
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- 阶段 4: 分发 + 推理
    caps.print("┌─ 阶段 4/4: 分发 + 推理 ──────────────────────┐")
    local N = #peers
    local my_id = caps.network.get_local_peer_id()

    caps.print("│ 分发 pipe_worker_single ...")
    for i, p in ipairs(peers) do
        local upstream = (i > 1) and peers[i - 1].peer_id or nil
        local downstream = (i < N) and peers[i + 1].peer_id or nil
        local payload = string.format(
            'EXEC|pipe_worker_single|{"model":"%s","layer_start":%d,"layer_end":%d,"upstream":"%s","downstream":"%s","coordinator":"%s","session_id":"%s"}',
            p.file_name, p.layer_start, p.layer_end,
            upstream or "", downstream or "", my_id, session_id)
        caps.print(string.format("│   [%d/%d] → %s", i, N, p.name))
        local resp = caps.network.send_data(p.peer_id, "Command", payload)
        caps.print(string.format("│         响应: %s", resp.payload or "nil"))
    end

    caps.print("│")
    local fwd = caps.network.open_tensor_stream(peers[1].peer_id, sid_num)
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print(string.format("│ fwd: → %s  ✓  bwd: ← %s  ✓", peers[1].name, peers[N].name))

    local stream_id = "ml-" .. session_id
    local session_stream = local_tensor.open_stream(stream_id)
    caps.print("│ Session ↔ coord  ✓")
    caps.print("│ ╔══════════════════════════════════╗")
    caps.print("│ ║  ✓ 流水线就绪，开始推理          ║")
    caps.print("│ ╚══════════════════════════════════╝")
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- 桥接循环
    local iter = 0
    while true do
        local ok, tensor, recv_offset = pcall(function()
            return local_tensor.recv_tensor(session_stream, "cpu")
        end)
        if not ok then break end

        iter = iter + 1
        caps.network.send_tensor(fwd, tensor, recv_offset)
        local logits, _ = caps.network.recv_tensor(bwd, "cpu")
        local_tensor.send_tensor(session_stream, logits, recv_offset)

        if iter % 10 == 0 then
            caps.print(string.format("[coord_single] %d tokens", iter))
        end
    end

    handle:release()
    caps.print("[coord_single] 演示结束")
end
