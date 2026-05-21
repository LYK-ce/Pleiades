-- programs/user/cpu_gpu_run.lua
-- Presented by KeJi
-- Date: 2026-05-21
-- CPU/GPU 混合推理：CPU 跑前半层 + embedding，GPU 跑后半层 + output
-- 使用方式: exec cpu_gpu_run model=xxx.pgguf split=16

COMMAND = "cpu_gpu_run"
DESCRIPTION = "CPU/GPU 混合推理 (embedding+前半→CPU, 后半+output→GPU)"

function execute(params)
    local model = params.model or "test.pgguf"
    local split = tonumber(params.split) or 16
    local cpu_device = params.cpu_device or "cpu"
    local gpu_device = params.gpu_device or "cuda"
    local temperature = tonumber(params.temperature) or 0.8
    local max_tokens = tonumber(params.max_tokens) or 120

    -- 0. 通过 Storage 获取模型路径
    local raw_path
    do
        local handle = caps.storage_acquire_read(model)
        raw_path = handle:path()
        handle:release()
    end
    caps.print("[cpu_gpu] 模型: " .. raw_path)

    -- 1. 分析模型获取总层数
    caps.print("[cpu_gpu] 分析模型...")
    local info = ml.analyze_model(raw_path)
    local N = info.num_layers
    local total = N + 1  -- layer 0=embedding, 1..N=blocks, N+1=output
    local mid = math.min(split, N)
    caps.print("  总层: " .. total .. " (embedding=0, blocks=1.." .. N .. ", output=" .. total .. ")")
    caps.print("  分段: CPU[0.." .. mid .. "]  GPU[" .. (mid + 1) .. ".." .. total .. "]")

    -- 2. 创建两个 session
    local cpu = ml.new(cpu_device)
    local gpu = ml.new(gpu_device)

    caps.print("[cpu_gpu] 加载 CPU 层 0.." .. mid .. " on " .. cpu_device)
    cpu:load_model(raw_path, 0, mid)

    caps.print("[cpu_gpu] 加载 GPU 层 " .. (mid + 1) .. ".." .. total .. " on " .. gpu_device)
    gpu:load_model(raw_path, mid + 1, total)

    caps.print("[cpu_gpu] 模型加载完成")

    -- 3. 编码 prompt
    local prompt = "你好，请介绍一下你自己。"
    caps.print("[cpu_gpu] Prompt: " .. prompt)
    local tokens = cpu:encode(prompt)
    caps.print("[cpu_gpu] 编码: " .. #tokens .. " tokens")

    -- 4. Prefill: CPU forward → to_device GPU → GPU forward → to_device CPU → sample
    local t = cpu:tensorize(tokens)
    local hidden = cpu:forward(t, 0)
    caps.print("[cpu_gpu] CPU forward 完成, hidden dims: [" .. table.concat(hidden:dims(), ", ") .. "]")

    local hidden_gpu = hidden:to_device(gpu_device)
    local logits = gpu:forward(hidden_gpu, 0)
    caps.print("[cpu_gpu] GPU forward 完成, logits dims: [" .. table.concat(logits:dims(), ", ") .. "]")

    local logits_cpu = logits:to_device(cpu_device)
    local tok = cpu:sample(logits_cpu, temperature)

    local eos = cpu:get_eos()
    local generated = 0
    local offset = #tokens

    if tok == eos then
        caps.print("<eos>")
        goto cleanup
    end

    local text = cpu:decode(tok)
    caps.print(text)
    generated = 1

    -- 5. 自回归循环
    for i = 2, max_tokens do
        local next_t = cpu:tensorize({tok})
        hidden = cpu:forward(next_t, offset)

        hidden_gpu = hidden:to_device(gpu_device)
        logits = gpu:forward(hidden_gpu, offset)

        logits_cpu = logits:to_device(cpu_device)
        tok = cpu:sample(logits_cpu, temperature)

        if tok == eos then
            caps.print("<eos>")
            break
        end

        text = cpu:decode(tok)
        caps.print(text)
        generated = generated + 1
        offset = offset + 1
    end

    caps.print("[cpu_gpu] 生成完成, 共 " .. generated .. " tokens")

    ::cleanup::
    cpu:unload()
    gpu:unload()
    caps.print("[cpu_gpu] 模型已卸载")
end
