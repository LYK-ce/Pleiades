-- Presented by KeJi
-- Date: 2026-06-02
-- 实验 E3: 单卡模式 vs 双卡模式
-- 用法: exec exp_single model=xxx.pgguf nodes=N tokens=M
-- 与 exp_scale 唯一区别: rexec pipe_worker_single

COMMAND = "exp_single"
DESCRIPTION = "E3 单卡实验: 测量单卡模式下的吞吐"

function execute(params)
    local model = params.model
    local nodes = tonumber(params.nodes) or 2
    local max_tokens = tonumber(params.tokens) or 128
    local prompt = params.prompt or "你好，请介绍一下你自己"
    local temperature = tonumber(params.temperature) or 0.8

    if not model then
        caps.print("EXP_ERROR: missing model")
        return
    end

    local handle = caps.storage_acquire_read(model)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)
    local model_id = info.model_id

    if not model_id then
        caps.print("EXP_ERROR: no model_id")
        handle:release()
        return
    end

    local all_peers = caps.network.list_model_peers(model_id)
    if #all_peers < nodes then
        caps.print(string.format("EXP_ERROR: need %d nodes, only %d found", nodes, #all_peers))
        handle:release()
        return
    end

    table.sort(all_peers, function(a, b) return a.layer_start < b.layer_start end)
    local peers = {}
    for i = 1, nodes do table.insert(peers, all_peers[i]) end

    caps.print(string.format("EXP_SINGLE model=%s nodes=%d max_tokens=%d", model, nodes, max_tokens))

    local my_id = caps.network.get_local_peer_id()
    local inference_id = os.time()
    local N = #peers

    -- rexec pipe_worker_single (不是 pipe_worker)
    for i, p in ipairs(peers) do
        if p.peer_id == my_id then goto continue end
        local upstream = (i > 1) and peers[i - 1].peer_id or nil
        local downstream = (i < N) and peers[i + 1].peer_id or nil
        local fields = {}
        fields[#fields+1] = string.format('"model":"%s"', p.file_name)
        fields[#fields+1] = string.format('"layer_start":%d', p.layer_start)
        fields[#fields+1] = string.format('"layer_end":%d', p.layer_end)
        if upstream and upstream ~= "" then
            fields[#fields+1] = string.format('"upstream":"%s"', upstream)
        end
        if downstream and downstream ~= "" then
            fields[#fields+1] = string.format('"downstream":"%s"', downstream)
        end
        fields[#fields+1] = string.format('"coordinator":"%s"', my_id)
        fields[#fields+1] = string.format('"session_id":"%d"', inference_id)
        local payload = "EXEC|pipe_worker_single|{" .. table.concat(fields, ",") .. "}"
        pcall(function() caps.network.send_data(p.peer_id, "Command", payload) end)
        ::continue::
    end

    local fwd_peer, bwd_peer = nil, nil
    for i = 1, N do if peers[i].peer_id ~= my_id then fwd_peer = peers[i]; break end end
    for i = N, 1, -1 do if peers[i].peer_id ~= my_id then bwd_peer = peers[i]; break end end
    if not fwd_peer or not bwd_peer then caps.print("EXP_ERROR: no remote peers"); return end
    local fwd = caps.network.open_tensor_stream(fwd_peer.peer_id, inference_id)
    local bwd = caps.network.accept_tensor_stream(inference_id, 300)

    local sess = ml.new("cpu")
    sess:load_tokenizer(full_path)
    local eos = sess:get_eos()

    local t_total_start = os.clock()
    local t0 = os.clock()
    local tokens = sess:encode(prompt)
    local t_encode = os.clock() - t0

    t0 = os.clock()
    local hidden = sess:tensorize(tokens)
    caps.network.send_tensor(fwd, hidden, 0)
    local logits, _ = caps.network.recv_tensor(bwd, "cpu")
    local t_prefill = os.clock() - t0

    local tok = sess:sample(logits, temperature)
    local first_text = ""
    if tok ~= eos then first_text = sess:decode(tok) end

    local offset = #tokens
    local t_decode_total = 0
    local generated = 1

    for i = 2, max_tokens do
        t0 = os.clock()
        local next_hidden = sess:tensorize({tok})
        caps.network.send_tensor(fwd, next_hidden, offset)
        logits, _ = caps.network.recv_tensor(bwd, "cpu")
        local t_fwd = os.clock() - t0

        tok = sess:sample(logits, temperature)
        if tok == eos then break end
        local _text = sess:decode(tok)
        generated = generated + 1
        offset = offset + 1
        t_decode_total = t_decode_total + t_fwd
    end

    local t_total = os.clock() - t_total_start

    caps.network.send_eof(fwd)
    caps.print(first_text:sub(1, 50))

    caps.print("")
    caps.print("==== EXP_SINGLE RESULT ====")
    caps.print(string.format("MODE:single"))
    caps.print(string.format("NODES:%d", nodes))
    caps.print(string.format("TOKENS:%d", generated))
    caps.print(string.format("TOTAL_S:%.3f", t_total))
    caps.print(string.format("ENCODE_S:%.3f", t_encode))
    caps.print(string.format("PREFILL_S:%.3f", t_prefill))
    caps.print(string.format("DECODE_S:%.3f", t_decode_total))
    caps.print(string.format("TOK_S:%.3f", generated / t_decode_total))
    caps.print(string.format("TOK_S_E2E:%.3f", generated / t_total))
    caps.print("==== EXP_SINGLE END ====")

    handle:release()
    sess:unload()
end
