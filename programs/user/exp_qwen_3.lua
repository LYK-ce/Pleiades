-- Presented by KeJi
-- Date: 2026-06-02
-- 实验: 三节点流水线 (Qwen3-235B)
-- 用法: exec exp_qwen_3 model=xxx.pgguf tokens=128

COMMAND = "exp_qwen_3"
DESCRIPTION = "三节点流水线实验"

function execute(params)
    local model = params.model
    local max_tokens = tonumber(params.tokens) or 128
    local prompt = params.prompt or "你好，请介绍一下你自己"
    local temperature = tonumber(params.temperature) or 0.8

    if not model then caps.print("EXP_ERROR: missing model"); return end

    -- 1. 获取 model_id
    local handle = caps.storage_acquire_read(model)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)
    local model_id = info.model_id
    if not model_id then caps.print("EXP_ERROR: no model_id"); handle:release(); return end

    -- 2. 发现集群
    local all_peers = caps.network.list_model_peers(model_id)
    if #all_peers < 3 then
        caps.print(string.format("EXP_ERROR: need 3 nodes, only %d", #all_peers)); handle:release(); return
    end
    table.sort(all_peers, function(a, b) return a.layer_start < b.layer_start end)
    local peers = {all_peers[1], all_peers[2], all_peers[3]}

    caps.print(string.format("EXP_QWEN_3 model=%s nodes=3 tokens=%d", model, max_tokens))

    -- 3. 启动 worker
    local my_id = caps.network.get_local_peer_id()
    local stream_id = os.time()
    local N = 3

    for i, p in ipairs(peers) do
        local upstream = (i > 1) and peers[i - 1].peer_id or nil
        local downstream = (i < N) and peers[i + 1].peer_id or nil
        local payload = string.format(
            'EXEC|exp_worker|{"model":"%s","layer_start":%d,"layer_end":%d,"upstream":"%s","downstream":"%s","coordinator":"%s","stream_id":"%d"}',
            p.file_name, p.layer_start, p.layer_end,
            upstream or "", downstream or "", my_id, stream_id)
        caps.network.send_data(p.peer_id, "Command", payload)
    end

    -- 4. 建立 tensor stream
    local fwd = caps.network.open_tensor_stream(peers[1].peer_id, stream_id)
    local bwd = caps.network.accept_tensor_stream(stream_id, 300)

    -- 5. Tokenizer (CPU)
    local sess = ml.new("cpu")
    sess:load_tokenizer(full_path)
    local eos = sess:get_eos()

    -- 6. Encode
    local t_total_start = os.clock()
    local t0 = os.clock()
    local tokens = sess:encode(prompt)
    local t_encode = os.clock() - t0

    -- Prefill
    t0 = os.clock()
    local hidden = sess:tensorize(tokens)
    caps.network.send_tensor(fwd, hidden, 0)
    local logits, _ = caps.network.recv_tensor(bwd, "cpu")
    local t_prefill = os.clock() - t0

    -- Sample first
    local tok = sess:sample(logits, temperature)
    local first_text = ""
    if tok ~= eos then first_text = sess:decode(tok) end

    -- 7. Decode loop
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
        sess:decode(tok)
        generated = generated + 1
        offset = offset + 1
        t_decode_total = t_decode_total + t_fwd
    end

    local t_total = os.clock() - t_total_start
    caps.network.send_eof(fwd)

    caps.print("")
    caps.print("==== EXP_QWEN_3 RESULT ====")
    caps.print(string.format("NODES:3"))
    caps.print(string.format("TOKENS:%d", generated))
    caps.print(string.format("TOTAL_S:%.3f", t_total))
    caps.print(string.format("ENCODE_S:%.3f", t_encode))
    caps.print(string.format("PREFILL_S:%.3f", t_prefill))
    caps.print(string.format("DECODE_S:%.3f", t_decode_total))
    caps.print(string.format("TOK_S:%.3f", generated / t_decode_total))
    caps.print(string.format("TOK_S_E2E:%.3f", generated / t_total))
    caps.print("==== EXP_QWEN_3 END ====")

    handle:release()
    sess:unload()
end
