-- Presented by KeJi
-- Date: 2026-06-13
-- 动态流水线 — 单卡 Coordinator (Session 模式, 自动分配层)
-- 用法: session inference pipeline_coord_single <session_id> <model_path>
-- Coordinator 不加载层，只做桥接。所有层分配给远程 worker（单卡）。

COMMAND = "pipeline_coord_single"
DESCRIPTION = "动态流水线单卡协调者: 发现集群→动态分层→分发worker→桥接Session→推理"

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
        caps.print("│ ✗ 模型文件缺少 model_id")
        caps.print("└──")
        handle:release()
        return
    end
    caps.print(string.format("│ 模型: %s  id=%d  总层: %d", model_path, model_id, total_layers))
    caps.print("└──")
    caps.print("")

    -- 阶段 2: 集群发现
    caps.print("┌─ 阶段 2/4: 集群发现 ─────────────────────────┐")
    local all_peers = caps.network.list_model_peers(model_id)
    local my_id = caps.network.get_local_peer_id()

    local remote_peers = {}
    for _, p in ipairs(all_peers) do
        if p.peer_id ~= my_id then table.insert(remote_peers, p) end
    end

    if #remote_peers == 0 then
        caps.print("│ ✗ 没有远程节点持有该模型")
        caps.print("└──")
        handle:release()
        return
    end

    caps.print(string.format("│ 发现 %d 个远程节点:", #remote_peers))
    for i, p in ipairs(remote_peers) do
        caps.print(string.format("│   [%d] %s  %s", i, p.name, p.file_name))
    end
    caps.print("└──")
    caps.print("")

    -- 阶段 3: 动态分配层
    caps.print("┌─ 阶段 3/4: 分配层范围 ───────────────────────┐")
    table.sort(remote_peers, function(a, b) return a.peer_id < b.peer_id end)

    local N = #remote_peers
    local layers_per_node = math.floor(total_layers / N)
    local remainder = total_layers % N
    local current_start = 0

    for i, p in ipairs(remote_peers) do
        local count = layers_per_node
        if i <= remainder then count = count + 1 end
        p.layer_start = current_start
        p.layer_end = current_start + count - 1
        current_start = current_start + count
    end

    caps.print(string.format("│ Coordinator (本机) ← 桥接，不加载层"))
    caps.print(string.format("│ %d 远程节点 (单卡):", N))
    for i, p in ipairs(remote_peers) do
        caps.print(string.format("│   [%d] %s  层 [%d - %d]", i, p.name, p.layer_start, p.layer_end))
    end
    caps.print("└──")
    caps.print("")

    -- 阶段 4: 分发 + 推理
    caps.print("┌─ 阶段 4/4: 分发 + 推理 ──────────────────────┐")

    -- 分发 worker
    for i, p in ipairs(remote_peers) do
        local upstream = (i > 1) and remote_peers[i - 1].peer_id or nil
        local downstream = (i < N) and remote_peers[i + 1].peer_id or nil

        local fields = {}
        fields[#fields+1] = string.format('"model":"%s"', p.file_name)
        fields[#fields+1] = string.format('"layer_start":"%d"', p.layer_start)
        fields[#fields+1] = string.format('"layer_end":"%d"', p.layer_end)
        if upstream then fields[#fields+1] = string.format('"upstream":"%s"', upstream) end
        if downstream then fields[#fields+1] = string.format('"downstream":"%s"', downstream) end
        fields[#fields+1] = string.format('"coordinator":"%s"', my_id)
        fields[#fields+1] = string.format('"session_id":"%s"', session_id)
        local payload = "EXEC|pipe_worker_single|{" .. table.concat(fields, ",") .. "}"

        caps.print(string.format("│   [%d/%d] → %s", i, N, p.name))
        local ok, resp = pcall(caps.network.send_data, p.peer_id, "Command", payload)
        if ok then
            caps.print(string.format("│         响应: %s", resp.payload or "nil"))
        else
            caps.print(string.format("│         失败: %s", tostring(resp)))
        end
    end

    caps.print("│")
    local fwd = caps.network.open_tensor_stream(remote_peers[1].peer_id, sid_num)
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print(string.format("│ fwd: → %s  ✓  bwd: ← %s  ✓", remote_peers[1].name, remote_peers[N].name))

    local stream_id = "ml-" .. session_id
    local session_stream = local_tensor.open_stream(stream_id)
    caps.print("│ Session ↔ coord  ✓")
    caps.print("│ ╔══════════════════════════════════╗")
    caps.print("│ ║  ✓ 流水线就绪，开始推理          ║")
    caps.print("│ ╚══════════════════════════════════╝")
    caps.print("└──")
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

    caps.network.send_eof(fwd)
    handle:release()
    caps.print("[coord_single] 演示结束")
end
