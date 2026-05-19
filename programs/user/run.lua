-- programs/user/run.lua
-- Presented by KeJi
-- Date: 2026-05-18
-- 单机推理脚本

COMMAND = "run"
DESCRIPTION = "单机推理 (硬编码 prompt, 模型默认 Pleiades_Workspace/test.gguf)"

function execute(params)
    local model_name = params.model or "test.gguf"
    local model_path = "Pleiades_Workspace/" .. model_name
    local device = params.device or "cpu"
    local temperature = tonumber(params.temperature) or 0.8
    local max_tokens = tonumber(params.max_tokens) or 120

    caps.print("设备: " .. device)
    caps.print("模型: " .. model_path)

    -- 1. 创建会话
    local sess = ml.new(device)
    caps.print("加载模型中...")
    sess:load_model(model_path, 0, 999999)
    caps.print("模型加载完成")

    -- 2. 硬编码 prompt
    local prompt = "你好，请介绍一下你自己。"
    caps.print("Prompt: " .. prompt)

    -- 3. 编码
    local tokens = sess:encode(prompt)
    caps.print("编码完成, token 数: " .. #tokens)

    -- 4. 首次推理
    local t = sess:tensorize(tokens)
    local logits = sess:forward(t, 0)

    -- 5. 自回归生成
    local eos = sess:get_eos()
    local generated = 0
    for i = 1, 120 do
        local tok = sess:sample(logits, temperature)
        if tok == eos then
            caps.print("<eos>")
            break
        end

        local text = sess:decode(tok)
        caps.print(text)
        generated = generated + 1

        local next_t = sess:tensorize({tok})
        logits = sess:forward(next_t, nil)  -- nil = 自动 offset
    end

    caps.print("生成完成, 共 " .. generated .. " tokens")

    -- 6. 卸载模型
    sess:unload()
    caps.print("模型已卸载")
end
