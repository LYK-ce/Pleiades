-- programs/user/local_coord.lua
-- Presented by KeJi
-- Date: 2026-05-21
-- 本地流协调端: embedding + 前半 transformer, 负责采样解码
-- 配合 local_work.lua 使用: 先 exec local_work, 再 exec local_coord

COMMAND = "local_coord"
DESCRIPTION = "本地流协调端 (embedding+前半→send, recv logits→sample)"

function execute(params)
    local model = params.model or "test.pgguf"
    local device = params.device or "cpu"
    local temperature = tonumber(params.temperature) or 0.8
    local max_tokens = tonumber(params.max_tokens) or 60
    local timeout = tonumber(params.timeout) or 120000

    -- 0. 获取模型路径
    local raw_path
    do
        local handle = caps.storage_acquire_read(model)
        raw_path = handle:path()
        handle:release()
    end
    caps.print("[coord] 模型: " .. raw_path)

    -- 1. 分析模型
    local info = ml.analyze_model(raw_path)
    local N = info.num_layers
    local total = N + 1
    local mid = math.floor(N / 2)
    caps.print("[coord] 总层: " .. total .. ", 分段: [0.." .. mid .. "] [" .. (mid+1) .. ".." .. total .. "]")

    -- 2. 加载前半模型
    local sess = ml.new(device)
    caps.print("[coord] 加载 0.." .. mid)
    sess:load_model(raw_path, 0, mid)
    caps.print("[coord] 模型加载完成")

    -- 3. 建立本地流: coord 打开 fwd 出站，等待 bwd 入站
    caps.print("[coord] 打开 fwd 流...")
    local fwd = local_tensor.open_stream("fwd")
    caps.print("[coord] 等待 bwd 流 (timeout=" .. timeout .. "ms)...")
    local bwd = local_tensor.accept_stream("bwd", timeout)
    caps.print("[coord] 双流已建立")

    -- 4. 编码 prompt
    local prompt = "你好，请介绍一下你自己。"
    caps.print("[coord] Prompt: " .. prompt)
    local tokens = sess:encode(prompt)
    caps.print("[coord] 编码: " .. #tokens .. " tokens")

    -- 5. 首次前向 (prefill)
    local t = sess:tensorize(tokens)
    local hidden = sess:forward(t, 0)
    caps.print("[coord] hidden dims: [" .. table.concat(hidden:dims(), ", ") .. "]")

    -- 发送 hidden → worker
    local_tensor.send_tensor(fwd, hidden:to_bytes(), 0)
    local offset = #tokens

    -- 6. 接收 logits, 采样, 解码
    local eos = sess:get_eos()

    local result = local_tensor.recv_tensor(bwd)
    local logits = ml.tensor_from_bytes(result.data, device)
    local tok = sess:sample(logits, temperature)

    if tok == eos then
        caps.print("<eos>")
        local_tensor.send_eof(fwd)
        sess:unload()
        return
    end

    local text = sess:decode(tok)
    caps.print(text)
    local generated = 1

    -- 7. 自回归循环
    for i = 2, max_tokens do
        local next_t = sess:tensorize({tok})
        hidden = sess:forward(next_t, offset)
        local_tensor.send_tensor(fwd, hidden:to_bytes(), offset)
        offset = offset + 1

        result = local_tensor.recv_tensor(bwd)
        logits = ml.tensor_from_bytes(result.data, device)
        tok = sess:sample(logits, temperature)

        if tok == eos then
            caps.print("<eos>")
            break
        end

        text = sess:decode(tok)
        caps.print(text)
        generated = generated + 1
    end

    -- 8. 结束
    local_tensor.send_eof(fwd)
    caps.print("[coord] 完成, 共 " .. generated .. " tokens")
    sess:unload()
end
