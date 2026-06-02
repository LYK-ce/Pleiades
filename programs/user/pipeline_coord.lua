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
    -- 阶段 3: 构建推理链条
    -- ═══════════════════════════════════════════════
    caps.print("┌─ 阶段 3/4: 构建推理链条 ─────────────────────┐")

    -- 按 layer_start 排序
    table.sort(peers, function(a, b) return a.layer_start < b.layer_start end)

    -- 验证链: 无间隙、首尾覆盖完整
    local valid = true
    for i = 1, #peers - 1 do
        if peers[i].layer_end + 1 ~= peers[i + 1].layer_start then
            caps.print(string.format("│ ✗ 层范围不连续: [%d,%d] → [%d,%d]",
                peers[i].layer_start, peers[i].layer_end,
                peers[i + 1].layer_start, peers[i + 1].layer_end))
            valid = false
        end
    end
    if peers[1].layer_start ~= 0 then
        caps.print(string.format("│ ✗ 链条未从 layer 0 开始 (start=%d)", peers[1].layer_start))
        valid = false
    end
    if peers[#peers].layer_end ~= total_layers - 1 then
        caps.print(string.format("│ ✗ 链条未覆盖到 layer %d (end=%d)", total_layers - 1, peers[#peers].layer_end))
        valid = false
    end

    if not valid then
        caps.print("│ 请检查模型分片是否完整覆盖了所有层")
        caps.print("└──────────────────────────────────────────────┘")
        handle:release()
        return
    end

    caps.print("│ ✓ 链条验证通过 — 层覆盖完整无间隙")
    caps.print("│")
    caps.print(string.format("│ 流水线拓扑 (%d 节点, 每节点 2×GPU):", #peers))
    caps.print("│")
    local my_id = caps.network.get_local_peer_id()
    for i, p in ipairs(peers) do
        local role = ""
        if p.peer_id == my_id then role = " ← 本机(协调者)" end
        if i == 1 then
            caps.print(string.format("│   ┌─ [%d] %s", i, p.name))
            caps.print(string.format("│   │   层 %d→%d  GPU:0→CPU→GPU:1%s", p.layer_start, p.layer_end, role))
        elseif i == #peers then
            caps.print(string.format("│   └─ [%d] %s", i, p.name))
            caps.print(string.format("│       层 %d→%d  GPU:0→CPU→GPU:1%s", p.layer_start, p.layer_end, role))
        else
            caps.print(string.format("│   ├─ [%d] %s", i, p.name))
            caps.print(string.format("│   │   层 %d→%d  GPU:0→CPU→GPU:1%s", p.layer_start, p.layer_end, role))
        end
    end
    caps.print(string.format("│   Session ←→ 节点[1] ←→ ... ←→ 节点[%d] ←→ Session", #peers))
    caps.print("└──────────────────────────────────────────────┘")
    caps.print("")

    -- ═══════════════════════════════════════════════
    -- 阶段 4: 分发 Worker + 启动推理
    -- ═══════════════════════════════════════════════
    caps.print("┌─ 阶段 4/4: 分发 + 推理 ──────────────────────┐")

    local N = #peers

    caps.print(string.format("│ 推理会话 ID: %d", sid_num))
    caps.print("│")
    caps.print("│ 正在向各节点分发 pipe_worker ...")
    caps.print("│")

    for i, p in ipairs(peers) do
        local upstream = (i > 1) and peers[i - 1].peer_id or nil
        local downstream = (i < N) and peers[i + 1].peer_id or nil

        local payload = string.format(
            'EXEC|pipe_worker|{"model":"%s","layer_start":%d,"layer_end":%d,"upstream":"%s","downstream":"%s","coordinator":"%s","session_id":"%s"}',
            p.file_name, p.layer_start, p.layer_end,
            upstream or "", downstream or "", my_id, session_id)

        caps.print(string.format("│   [%d/%d] → %s  rexec pipe_worker", i, N, p.name))
        local resp = caps.network.send_data(p.peer_id, "Command", payload)
        caps.print(string.format("│         响应: %s", resp.payload or "nil"))
    end

    caps.print("│")
    caps.print("│ 建立网络张量流 ...")

    -- 打开前向流 (→ 第一个 worker)
    local fwd = caps.network.open_tensor_stream(peers[1].peer_id, sid_num)
    caps.print(string.format("│   fwd: coord → %s  ✓", peers[1].name))

    -- 接受返回流 (← 最后一个 worker)
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print(string.format("│   bwd: %s → coord  ✓", peers[N].name))

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

    handle:release()
    caps.print("[coord] 流水线演示结束")
end
