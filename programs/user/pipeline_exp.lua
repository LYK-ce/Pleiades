-- Presented by KeJi
-- Date: 2026-06-03
-- 分布式推理实验脚本 — 不依赖 session/API
-- 用法: exec pipeline_exp path=<model>.pgguf prompt="..." max_tokens=100 mode=single|dual

COMMAND = "pipeline_exp"
DESCRIPTION = "分布式推理实验: 发现集群→构建链条→加载模型→推理→计时"

function execute(params)
    local model_path = params.path
    local prompt_text = params.prompt or "What is the meaning of life?"
    local max_tokens = tonumber(params.max_tokens) or 100
    local mode = params.mode or "dual"  -- "single" or "dual"

    if not model_path then
        caps.print("[exp] 缺少参数 path=<model.pgguf>")
        return
    end

    -- ═══════════════════════════════════════════════
    -- 阶段 1: 模型识别 + 加载 tokenizer
    -- ═══════════════════════════════════════════════
    caps.print("[exp] 阶段 1: 模型识别...")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)
    local model_id = info.model_id
    local total_layers = info.num_layers

    if not model_id then
        caps.print("[exp] 错误: 模型缺少 model_id")
        handle:release()
        return
    end

    caps.print(string.format("[exp] %s | layers=%d | model_id=%d",
        info.architecture, total_layers, model_id))

    -- 加载 tokenizer
    local sess = ml.new("cuda:0")
    sess:load_tokenizer(full_path)
    caps.print("[exp] Tokenizer 加载完成")

    -- ═══════════════════════════════════════════════
    -- 阶段 2: 集群发现
    -- ═══════════════════════════════════════════════
    caps.print("[exp] 阶段 2: 集群发现...")
    local peers = caps.network.list_model_peers(model_id)

    -- 过滤本机
    local my_id = caps.network.get_local_peer_id()
    local remote_peers = {}
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            table.insert(remote_peers, p)
        end
    end
    peers = remote_peers

    caps.print(string.format("[exp] 发现 %d 个远程节点", #peers))
    for i, p in ipairs(peers) do
        caps.print(string.format("[exp]   [%d] %s  layers [%d-%d]  %s",
            i, p.name, p.layer_start, p.layer_end, p.file_name))
    end

    -- ═══════════════════════════════════════════════
    -- 阶段 3: 构建推理链条
    -- ═══════════════════════════════════════════════
    caps.print("[exp] 阶段 3: 构建推理链条...")

    -- 把本机作为链条第一个节点（必须持有 embedding 层 [0, N]）
    -- 远程节点按 peer_id 排序保证确定性
    table.sort(peers, function(a, b) return a.peer_id < b.peer_id end)

    local coord_peer = {
        name = "coord(本机)",
        peer_id = my_id,
        file_name = model_path,
    }
    table.insert(peers, 1, coord_peer)

    -- 均匀分配层范围（coordinator 固定第一个，拿 [0, N-1]）
    -- pipeline 坐标: 0=embedding, 1..total_layers=blocks, total_layers+1=output head
    local N = #peers
    local pipeline_slots = total_layers + 2  -- embedding + blocks + output
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

    caps.print(string.format("[exp] 链条构建: %d 节点, %d 层/节点", N, layers_per_node))
    for i, p in ipairs(peers) do
        caps.print(string.format("[exp]   [%d] %s  → 层 [%d,%d]",
            i, p.name, p.layer_start, p.layer_end))
    end

    -- ═══════════════════════════════════════════════
    -- 阶段 4: 分发 worker
    -- ═══════════════════════════════════════════════
    caps.print("[exp] 阶段 4: 分发 worker...")
    local sid_num = 42  -- 固定实验用 session_id

    local worker_cmd = (mode == "single") and "pipe_worker_single" or "pipe_worker"

    for i, p in ipairs(peers) do
        if p.peer_id == my_id then
            goto continue
        end

        local upstream = (i > 1) and peers[i - 1].peer_id or nil
        local downstream = (i < N) and peers[i + 1].peer_id or nil

        local payload = string.format(
            'EXEC|%s|{"model":"%s","layer_start":%d,"layer_end":%d,"upstream":"%s","downstream":"%s","coordinator":"%s","session_id":%d}',
            worker_cmd, p.file_name, p.layer_start, p.layer_end,
            upstream or "", downstream or "", my_id, sid_num)

        caps.print(string.format("[exp]   → %s 启动 %s", p.name, worker_cmd))
        local ok, resp = pcall(caps.network.send_data, p.peer_id, "Command", payload)
        if ok then
            caps.print(string.format("[exp]     响应: %s", resp.payload or "nil"))
        else
            caps.print(string.format("[exp]     发送失败: %s", tostring(resp)))
        end
        ::continue::
    end

    -- 找到第一个和最后一个远程 worker
    local first_worker, last_worker = nil, nil
    for i = 1, N do
        if peers[i].peer_id ~= my_id then
            if not first_worker then first_worker = peers[i] end
            last_worker = peers[i]
        end
    end

    if not first_worker then
        caps.print("[exp] 错误: 没有远程 worker")
        handle:release()
        sess:unload()
        return
    end

    -- ═══════════════════════════════════════════════
    -- 阶段 5: 建立网络张量流
    -- ═══════════════════════════════════════════════
    caps.print("[exp] 阶段 5: 建立张量流...")
    local fwd = caps.network.open_tensor_stream(first_worker.peer_id, sid_num)
    caps.print(string.format("[exp]   fwd: coord → %s ✓", first_worker.name))

    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print(string.format("[exp]   bwd: %s → coord ✓", last_worker.name))

    -- ═══════════════════════════════════════════════
    -- 阶段 6: 加载本地模型
    -- ═══════════════════════════════════════════════
    caps.print("[exp] 阶段 6: 加载本地模型...")
    local coord_entry = nil
    for _, p in ipairs(peers) do
        if p.peer_id == my_id then coord_entry = p; break end
    end

    local coord_ls = coord_entry.layer_start
    local coord_le = coord_entry.layer_end
    local coord_layers = coord_le - coord_ls + 1

    local t_load_start = caps.monotonic_time()
    local sess_gpu1 = nil  -- 双卡模式下的第二 GPU

    if mode == "single" then
        caps.print(string.format("[exp]   单卡模式: cuda:0 ← layers [%d,%d]", coord_ls, coord_le))
        sess:load_model(full_path, coord_ls, coord_le)
    else
        local coord_mid = coord_ls + math.floor(coord_layers / 2) - 1
        caps.print(string.format("[exp]   双卡: GPU:0 ← [%d,%d] | GPU:1 ← [%d,%d]",
            coord_ls, coord_mid, coord_mid + 1, coord_le))
        sess:load_model(full_path, coord_ls, coord_mid)

        sess_gpu1 = ml.new("cuda:1")
        sess_gpu1:load_model(full_path, coord_mid + 1, coord_le)
    end

    local t_load = caps.monotonic_time() - t_load_start
    caps.print(string.format("[exp]   模型加载完成 (%.2fs)", t_load))

    -- ═══════════════════════════════════════════════
    -- 阶段 7: Tokenize + Prefill
    -- ═══════════════════════════════════════════════
    caps.print("[exp] 阶段 7: Tokenize...")
    local tokens = sess:encode(prompt_text)
    caps.print(string.format("[exp]   Prompt tokens: %d", #tokens))

    -- Tensorize
    caps.print("[exp] Prefill...")
    local t_prefill_start = caps.monotonic_time()

    local input_tensor = sess:tensorize(tokens)

    -- 本地 forward（单卡或双卡桥接）
    local hidden
    if mode == "single" then
        hidden = sess:forward(input_tensor, 0)
    else
        local h0 = sess:forward(input_tensor, 0)
        local h_cpu = h0:to_device("cpu")
        local h1 = h_cpu:to_device("cuda:1")
        hidden = sess_gpu1:forward(h1, 0)
    end

    caps.network.send_tensor(fwd, hidden, 0)

    -- 接收第一个 logits
    local logits, _ = caps.network.recv_tensor(bwd, "cuda:0")

    local first_token = sess:sample(logits, 0.8)
    local first_text = sess:decode(first_token)
    caps.print(string.format("[exp]   First token: %d '%s'", first_token, first_text))

    local t_prefill = caps.monotonic_time() - t_prefill_start
    caps.print(string.format("[exp] Prefill 完成: %.2fms", t_prefill * 1000))

    -- ═══════════════════════════════════════════════
    -- 阶段 8: 自回归 Decode
    -- ═══════════════════════════════════════════════
    caps.print(string.format("[exp] Decode (max %d tokens)...", max_tokens))

    local eos = sess:get_eos() or 151645
    local gen_count = 0
    local decode_text = ""
    local t_decode_start = caps.monotonic_time()
    local total_forward_time = 0.0
    local total_cpu_time = 0.0
    local total_network_time = 0.0

    local current_token = first_token
    local current_offset = #tokens  -- offset after prefill

    while gen_count < max_tokens - 1 do
        if current_token == eos then
            caps.print("[exp]   EOS reached")
            break
        end

        -- 将当前 token tensorize 并 forward（单卡或双卡桥接）
        local t_iter_start = caps.monotonic_time()

        local t_fwd_start = caps.monotonic_time()
        local next_tensor = sess:tensorize({current_token})

        local next_hidden
        if mode == "single" then
            next_hidden = sess:forward(next_tensor, current_offset)
        else
            local h0 = sess:forward(next_tensor, current_offset)
            local h_cpu = h0:to_device("cpu")
            local h1 = h_cpu:to_device("cuda:1")
            next_hidden = sess_gpu1:forward(h1, current_offset)
        end
        total_forward_time = total_forward_time + (caps.monotonic_time() - t_fwd_start)

        -- 发送到 chain
        local t_net_start = caps.monotonic_time()
        caps.network.send_tensor(fwd, next_hidden, current_offset)
        local next_logits, _ = caps.network.recv_tensor(bwd, "cuda:0")
        total_network_time = total_network_time + (caps.monotonic_time() - t_net_start)

        -- Sample
        current_token = sess:sample(next_logits, 0.8)
        local token_text = sess:decode(current_token)
        decode_text = decode_text .. token_text

        gen_count = gen_count + 1
        current_offset = current_offset + 1

        local t_iter = caps.monotonic_time() - t_iter_start

        if gen_count == 1 then
            caps.print(string.format("[exp]   Token 1: %.0fms", t_iter * 1000))
        elseif gen_count % 10 == 0 then
            caps.print(string.format("[exp]   %d tokens | avg: %.2fs/tok",
                gen_count, t_iter))
        end
    end

    local t_decode = caps.monotonic_time() - t_decode_start

    -- ═══════════════════════════════════════════════
    -- 结果输出
    -- ═══════════════════════════════════════════════
    caps.print("")
    caps.print("=== EXPERIMENT RESULT ===")
    caps.print(string.format("Model:     %s (%s)", model_path, info.architecture))
    caps.print(string.format("Nodes:     %d (%s mode)", N, mode))
    caps.print(string.format("Prompt:    %d tokens", #tokens))
    caps.print(string.format("Generated: %d tokens", gen_count))
    caps.print("---")
    caps.print(string.format("Model load: %.2fs", t_load))
    caps.print(string.format("Prefill:   %.2fms (%d tokens)", t_prefill * 1000, #tokens))
    caps.print(string.format("Decode:    %.2fs (%d tokens)", t_decode, gen_count))
    if gen_count > 0 then
        caps.print(string.format("  tok/s:   %.2f", gen_count / t_decode))
    end
    caps.print("---")
    caps.print(string.format("Output: \"%s\"", decode_text:sub(1, 200)))
    caps.print("=== END ===")

    -- 清理
    if sess_gpu1 then
        sess_gpu1:unload()
    end
    sess:unload()
    handle:release()
    caps.print("[exp] 完成")
end
