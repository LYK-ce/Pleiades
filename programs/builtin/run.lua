-- programs/run.lua
-- Presented by KeJi
-- Date: 2026-05-17
-- 单机推理脚本
COMMAND = "run"
DESCRIPTION = "单机推理 (本地模型加载 + 自回归生成)"

function execute(params, caps)
    local sess = caps.create_session(params.model, params.device, 0, 999999)
    local io = caps.allocate_io()

    local prompt = io.input()
    local tokens = sess:encode(prompt)
    sess:forward(sess:tensorize(tokens), 0)

    for i = 1, tonumber(params.max_tokens) or 120 do
        local tok = sess:sample(tonumber(params.temperature) or 0.8)
        io.output(sess:decode(tok))
        if tok == sess:get_eos() then break end
        sess:forward(sess:tensorize({tok}), nil)
    end

    io.end_output()
    sess:unload()
end
