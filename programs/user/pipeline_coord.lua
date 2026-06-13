-- Presented by KeJi
-- Date: 2026-06-02
-- 动态流水线 — 演示用 Coordinator (Session 模式)
-- 用法: session inference pipeline <session_id> <model_path>
--    coord 节点: session create model.pgguf → 得到 session_id
--    coord 节点: session inference pipeline <sid> model.pgguf

COMMAND = "pipeline"
DESCRIPTION = "动态流水线协调者: 发现集群→构建链条→桥接Session→推理"

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    if not session_id or not model_path then
        caps.print("[coord] 缺少参数 session_id/model_path")
        return
    end

    local sid_num = tonumber(session_id)
    caps.print("")
    caps.print("╔══════════════════════════════════════════════╗")
    caps.print("║   Pleiades 动态流水线 — 分布式推理演示       ║")
    caps.print("╚══════════════════════════════════════════════╝")
    caps.print("")

    -- ═══════════════════════════════════════════════
    -- 阶段 1: 模型识别
    -- ═══════════════════════════════════════════════
    caps.print("┌─ 阶段 1/4: 模型识别 ─────────────────────────┐")

    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)
    local model_id = info.model_id
    local total_layers = info.num_layers + 2   -- embedding(0) + blocks(1..N) + output(N+1)

    if not model_id then
        caps.print("│ ✗ 错误: 模型文件缺少 model_id")
        caps.print("└──────────────────────────────────────────────┘")
        handle:release()
        return
    end

    caps.print(string.format("│ 模型文件 : %s", model_path))
    caps.print(string.format("│ 架构     : %s", info.architecture))
    caps.print(string.format("│ 总层数   : %d (embedding + %d blocks + output)", total_layers, info.num_layers))
    caps.print(string.format("│ Model ID : %d (xxhash32, 跨分片唯一)", model_id))
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- ═══════════════════════════════════════════════
    -- 阶段 2: 集群发现
    -- ═══════════════════════════════════════════════
    caps.print("┌─ 阶段 2/4: 集群发现 ─────────────────────────┐")
    caps.print(string.format("│ 搜索 Model ID = %d 的节点...", model_id))

    local peers = caps.network.list_model_peers(model_id)

    if #peers == 0 then
        caps.print(string.format("│ ✗ 错误: 未找到持有 model_id=%d 的节点", model_id))
        caps.print("└──────────────────────────────────────────────┘")
        handle:release()
        return
    end

    caps.print(string.format("│ ✓ 发现 %d 个节点持有该模型:", #peers))
    for i, p in ipairs(peers) do
        caps.print(string.format("│   [%d] %-20s  层 [%2d - %2d]  %s",
            i, p.name, p.layer_start, p.layer_end, p.file_name))
    end
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- ═══════════════════════════════════════════════
    -- 阶段 3: 构建推理链条（动态分配层范围）
    -- ═══════════════════════════════════════════════
    caps.print("┌─ 阶段 3/4: 构建推理链条 ─────────────────────┐")

    local my_id = caps.network.get_local_peer_id()

    -- 按 peer_id 排序保证确定性
    table.sort(peers, function(a, b) return a.peer_id < b.peer_id end)

    -- 均匀分配 pipeline 层范围 (0=embedding, 1..N=blocks, N+1=output)
    local N = #peers
    local pipeline_slots = total_layers  -- = num_layers + 2
    local layers_per_node = math.floor(pipeline_slots / N)
    local remainder = pipeline_slots % N
    local current_start = 0

    for i, p in ipairs(peers) do
        local count = layers_per_node
        if i <= remainder then count = count + 1 end
        p.layer_start = current_start
        p.layer_end = current_start + count - 1
        current_start = current_start + count
    end

    caps.print(string.format("│ 流水线拓扑 (%d 节点, 每节点 2×GPU, 自动分配):", N))
    caps.print("│")
    for i, p in ipairs(peers) do
        local role = ""
        if p.peer_id == my_id then role = " ← 本机(协调者)" end
        caps.print(string.format("│   [%d] %-20s  层 [%2d - %2d]%s",
            i, p.name, p.layer_start, p.layer_end, role))
    end
    caps.print("│")
    caps.print(string.format("│   Session ←→ 节点[1] ←→ ... ←→ 节点[%d] ←→ Session", N))
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- ═══════════════════════════════════════════════
    -- 阶段 4: 分发 Worker + 启动推理
    -- ═══════════════════════════════════════════════
    caps.print("┌─ 阶段 4/4: 分发 + 推理 ──────────────────────┐")

    local N = #peers

    caps.print(string.format("│ 推理会话 ID: %d", sid_num))
    caps.print("│")
    caps.print("│ 正在向各节点启动 pipe_worker ...");
    caps.print("│")

    local my_id = caps.network.get_local_peer_id()

    for i, p in ipairs(peers) do
        -- 跳过本机：coordinator 不向自己发网络指令
        if p.peer_id == my_id then
            caps.print(string.format("│   [%d/%d] %s  跳过 (本机)", i, N, p.name))
            goto continue
        end

        local upstream = (i > 1) and peers[i - 1].peer_id or nil
        local downstream = (i < N) and peers[i + 1].peer_id or nil

        -- 构造 JSON: 不发送空字符串字段
        local fields = {}
        fields[#fields+1] = string.format('"model":"%s"', p.file_name)
        fields[#fields+1] = string.format('"layer_start":"%d"', p.layer_start)
        fields[#fields+1] = string.format('"layer_end":"%d"', p.layer_end)
        if upstream and upstream ~= "" then
            fields[#fields+1] = string.format('"upstream":"%s"', upstream)
        end
        if downstream and downstream ~= "" then
            fields[#fields+1] = string.format('"downstream":"%s"', downstream)
        end
        fields[#fields+1] = string.format('"coordinator":"%s"', my_id)
        fields[#fields+1] = string.format('"session_id":"%s"', session_id)
        local payload = "EXEC|pipe_worker|{" .. table.concat(fields, ",") .. "}"

        caps.print(string.format("│   [%d/%d] → %s  启动 pipe_worker", i, N, p.name))
        local ok, resp = pcall(function()
            return caps.network.send_data(p.peer_id, "Command", payload)
        end)
        if ok and resp and resp.payload == "OK" then
            caps.print(string.format("│         响应: OK"))
        elseif ok and resp then
            caps.print(string.format("│         ✗ 远程拒绝: %s", resp.payload or "nil"))
            caps.print("│ 请确认远程节点已部署 pipe_worker 脚本")
            caps.print("└──────────────────────────────────────────────┘")
            handle:release()
            return
        else
            caps.print(string.format("│         ✗ 网络失败: %s", tostring(resp)))
            caps.print("└──────────────────────────────────────────────┘")
            handle:release()
            return
        end
        ::continue::
    end

    caps.print("│")
    caps.print("│ 建立网络张量流 ...")

    -- 找第一个 / 最后一个远程节点（跳过本机）
    local fwd_peer, bwd_peer = nil, nil
    for i = 1, N do
        if peers[i].peer_id ~= my_id then fwd_peer = peers[i]; break end
    end
    for i = N, 1, -1 do
        if peers[i].peer_id ~= my_id then bwd_peer = peers[i]; break end
    end
    if not fwd_peer or not bwd_peer then
        caps.print("│ ✗ 错误: 所有节点均为本机，无法建立远程流水线")
        caps.print("└──────────────────────────────────────────────┘")
        handle:release()
        return
    end

    -- 打开前向流 (→ 第一个远程 worker)
    local fwd = caps.network.open_tensor_stream(fwd_peer.peer_id, sid_num)
    caps.print(string.format("│   fwd: coord → %s  ✓", fwd_peer.name))

    -- 接受返回流 (← 最后一个远程 worker)
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print(string.format("│   bwd: %s → coord  ✓", bwd_peer.name))

    caps.print("│")
    caps.print("│ 连接 Session ...")

    -- 连接 Session 的 local_tensor 流
    local stream_id = "ml-" .. session_id
    local session_stream = local_tensor.open_stream(stream_id)
    caps.print(string.format("│   local_tensor: session ↔ coord  ✓"))
    caps.print("│")
    caps.print("│ ╔══════════════════════════════════╗")
    caps.print("│ ║  ✓ 流水线就绪，开始推理          ║")
    caps.print("│ ╚══════════════════════════════════╝")
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- ═══════════════════════════════════════════════
    -- 桥接循环: Session ←→ 流水线链条
    -- ═══════════════════════════════════════════════
    local iter = 0
    while true do
        -- 从 Session 接收 tensor (encoder 输出或下一个 token embedding)
        local ok, tensor, recv_offset = pcall(function()
            return local_tensor.recv_tensor(session_stream, "cpu")
        end)
        if not ok then
            caps.print("[coord] Session 流结束")
            break
        end

        iter = iter + 1
        local t0 = os.clock()

        -- 前向: Session → 流水线链头
        caps.network.send_tensor(fwd, tensor, recv_offset)

        -- 反向: 链尾 → Session
        local logits, _ = caps.network.recv_tensor(bwd, "cpu")

        -- 返回 logits 给 Session (Session 负责 sample/decode)
        local_tensor.send_tensor(session_stream, logits, recv_offset)

        local elapsed = os.clock() - t0
        if iter == 1 then
            caps.print(string.format("[coord] Prefill 完成 (%.2fs)", elapsed))
        elseif iter % 10 == 0 then
            caps.print(string.format("[coord] %d tokens (%.2fs/tok)", iter, elapsed))
        end
    end

    -- 通知链尾 worker 流结束
    caps.network.send_eof(fwd)
    caps.print("[coord] 已发送 EOF → 流水线链头")

    handle:release()
    caps.print("[coord] 流水线演示结束")
end
