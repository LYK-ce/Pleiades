-- programs/user/pipeline1.lua
-- Presented by KeJi
-- Date: 2026-05-19
-- 流水线并行 — 协调者 (embedding + 前半 transformer)

COMMAND = "pipeline1"
DESCRIPTION = "流水线并行前半段: embedding + transformer[0..mid-1], 负责采样解码"

function execute(params)
    local model = params.model or "test.pgguf"
    local device = params.device or "cpu"
    local temperature = tonumber(params.temperature) or 0.8
    local max_tokens = tonumber(params.max_tokens) or 60
    local peer = params.peer

    if not peer then
        caps.print("[pipeline1] 缺少 peer 参数")
        return
    end

    -- 0. 通过 Storage 获取模型路径
    local handle = caps.storage_acquire_read(model)
    local raw_path = handle:path()
    caps.print("[pipeline1] 模型文件: " .. raw_path)

    -- 1. 分析模型（自动处理 .gguf→.pgguf 转换）
    caps.print("[pipeline1] 分析模型...")
    local info = ml.analyze_model(raw_path)
    local N = info.num_layers
    local total = N + 1
    local mid = math.floor(N / 2)
    caps.print("  总层: " .. total .. " (embedding=0, blocks=1.." .. N .. ", output=" .. total .. ")")
    caps.print("  分段: A[0.." .. mid .. "]  B[" .. (mid+1) .. ".." .. total .. "]")

    -- 2. 确定 .pgguf 路径
    local pgguf_path = raw_path
    if raw_path:match("%.gguf$") then
        pgguf_path = raw_path:gsub("%.gguf$", ".pgguf")
    end

    -- 3. 创建会话，加载前半模型
    local sess = ml.new(device)
    caps.print("[pipeline1] 加载 0.." .. mid)
    sess:load_model(pgguf_path, 0, mid)
    caps.print("[pipeline1] 模型加载完成 (has_input_head=" .. tostring(sess:has_model()) .. ")")

    -- 4. 编码 prompt
    local prompt = "你好，请介绍一下你自己。"
    caps.print("[pipeline1] Prompt: " .. prompt)
    local tokens = sess:encode(prompt)
    caps.print("[pipeline1] 编码: " .. #tokens .. " tokens")

    -- 5. 建立双流
    caps.print("[pipeline1] 建立连接...")
    local stream1 = caps.network.open_tensor_stream(peer, 42)
    caps.print("[pipeline1] stream1 已打开")
    local stream2 = caps.network.accept_tensor_stream(42, 120)
    caps.print("[pipeline1] stream2 已建立")

    -- 6. 首次前向 (处理 prompt)
    caps.print("[pipeline1] 首次前向...")
    local t = sess:tensorize(tokens)
    local initial_offset = 0
    local hidden = sess:forward(t, initial_offset)
    caps.print("[pipeline1] hidden dims: [" .. table.concat(hidden:dims(), ", ") .. "]")
    caps.network.send_tensor(stream1, hidden, initial_offset)
    local offset = #tokens

    -- 7. 接收 logits, 采样, 解码
    local eos = sess:get_eos()

    local logits = caps.network.recv_tensor(stream2, device)
    local tok = sess:sample(logits, temperature)

    if tok == eos then
        caps.print("[pipeline1] <eos>")
        caps.network.send_eof(stream1)
        sess:unload()
        return
    end

    local text = sess:decode(tok)
    caps.print(text)
    local generated = 1

    -- 8. 自回归循环
    for i = 2, max_tokens do
        local next_t = sess:tensorize({tok})
        hidden = sess:forward(next_t, offset)
        caps.network.send_tensor(stream1, hidden, offset)
        offset = offset + 1

        logits = caps.network.recv_tensor(stream2, device)
        tok = sess:sample(logits, temperature)

        if tok == eos then
            caps.print("<eos>")
            break
        end

        text = sess:decode(tok)
        caps.print(text)
        generated = generated + 1
    end

    -- 9. 结束
    caps.network.send_eof(stream1)
    caps.print("[pipeline1] 完成, 共 " .. generated .. " tokens")
    sess:unload()
end
