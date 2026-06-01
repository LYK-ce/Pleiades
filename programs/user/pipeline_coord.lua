-- Presented by KeJi
-- Date: 2026-06-01
-- 动态流水线 — 双卡 Coordinator
-- 发现集群节点 → 排序验证 → 分发 worker → 编码/采样/解码

COMMAND = "pipeline"
DESCRIPTION = "动态流水线协调者 (双卡): 发现节点、分发 pipe_worker、编码/采样/解码"

function execute(params)
    local model = params.model
    local prompt = params.prompt or "你好"
    local temperature = tonumber(params.temperature) or 0.8
    local max_tokens = tonumber(params.max_tokens) or 60

    if not model then
        caps.print("[coord] 缺少 model 参数")
        return
    end

    -- 1. 通过 Storage 获取模型路径 + 分析元数据
    local model_path
    local total_layers
    do
        local handle = caps.storage_acquire_read(model)
        model_path = handle:path()
        local info = ml.analyze_model(model_path)
        total_layers = info.num_layers + 2   -- embedding(0) + blocks(1..N) + output(N+1)
        handle:release()
    end
    caps.print(string.format("[coord] 模型: %s, 总层: %d", model_path, total_layers))

    -- 2. 发现集群中持有该模型的节点
    local peers = caps.network.list_model_peers(model)
    if #peers == 0 then
        caps.print("[coord] 错误: 没有节点持有模型 " .. model)
        return
    end

    -- 按 layer_start 排序
    table.sort(peers, function(a, b) return a.layer_start < b.layer_start end)

    -- 验证链: 无间隙、首尾覆盖完整
    for i = 1, #peers - 1 do
        if peers[i].layer_end + 1 ~= peers[i + 1].layer_start then
            caps.print(string.format("[coord] 错误: 层范围不连续 [%d,%d] → [%d,%d]",
                peers[i].layer_start, peers[i].layer_end,
                peers[i + 1].layer_start, peers[i + 1].layer_end))
            return
        end
    end
    if peers[1].layer_start ~= 0 then
        caps.print(string.format("[coord] 错误: 第一段 start=%d, 需要 0", peers[1].layer_start))
        return
    end
    if peers[#peers].layer_end ~= total_layers - 1 then
        caps.print(string.format("[coord] 错误: 最后一段 end=%d, 需要 %d",
            peers[#peers].layer_end, total_layers - 1))
        return
    end

    caps.print(string.format("[coord] 链条: %d 个节点", #peers))
    for i, p in ipairs(peers) do
        caps.print(string.format("  [%d] %s: layers [%d,%d] file=%s",
            i, p.name, p.layer_start, p.layer_end, p.file_name))
    end

    -- 3. 分发 worker 到每个节点
    local inference_id = os.time()
    local N = #peers
    local my_id = caps.network.get_local_peer_id()

    for i, p in ipairs(peers) do
        local upstream = (i > 1) and peers[i - 1].peer_id or nil
        local downstream = (i < N) and peers[i + 1].peer_id or nil

        local payload = string.format(
            'EXEC|pipe_worker|{"model":"%s","layer_start":%d,"layer_end":%d,"upstream":"%s","downstream":"%s","coordinator":"%s","inference_id":%d}',
            model, p.layer_start, p.layer_end,
            upstream or "", downstream or "", my_id, inference_id)

        caps.print(string.format("[coord] rexec → %s: %s", p.name, payload))
        local resp = caps.network.send_data(p.peer_id, "Command", payload)
        caps.print(string.format("[coord] %s 响应: %s", p.name, resp.payload or "nil"))
    end

    -- 4. 打开与首尾 worker 的 tensor stream
    caps.print("[coord] 建立 tensor stream ...")
    local fwd = caps.network.open_tensor_stream(peers[1].peer_id, inference_id)
    caps.print("[coord] fwd stream 已打开 (→ " .. peers[1].name .. ")")

    local bwd = caps.network.accept_tensor_stream(inference_id, 300)
    caps.print("[coord] bwd stream 已建立 (← " .. peers[N].name .. ")")

    -- 5. 加载 tokenizer (coordinator 只做 encode/sample/decode)
    local sess = ml.new("cpu")
    sess:load_tokenizer(model_path)
    caps.print("[coord] tokenizer 已加载")

    -- 6. 编码 prompt → tensorize → 发送 hidden
    caps.print("[coord] Prompt: " .. prompt)
    local tokens = sess:encode(prompt)
    caps.print("[coord] 编码: " .. #tokens .. " tokens")

    local hidden = sess:tensorize(tokens)
    caps.network.send_tensor(fwd, hidden, 0)
    caps.print("[coord] hidden 已发送")

    -- 7. 推理循环: 收 logits → sample → decode → tensorize → 发 hidden
    local eos = sess:get_eos()
    local generated = 0
    local offset = #tokens

    for _ = 1, max_tokens do
        local logits, _ = caps.network.recv_tensor(bwd, "cpu")
        local tok = sess:sample(logits, temperature)

        if tok == eos then
            caps.print("<eos>")
            break
        end

        local text = sess:decode(tok)
        caps.print(text)
        generated = generated + 1

        local next_hidden = sess:tensorize({tok})
        caps.network.send_tensor(fwd, next_hidden, offset)
        offset = offset + 1
    end

    -- 8. 结束
    caps.network.send_eof(fwd)
    caps.print(string.format("[coord] 完成, 共 %d tokens", generated))
    sess:unload()
end
