-- Presented by KeJi
-- Date: 2026-06-02
-- 实验用轻量 Worker: 加载模型分片 → recv → forward → send
-- 由 exp_qwen_N.lua 通过 rexec 远程启动

COMMAND = "exp_worker"
DESCRIPTION = "实验用 Worker: 加载模型, recv→forward→send"

function execute(params)
    local model = params.model
    local layer_start = tonumber(params.layer_start)
    local layer_end = tonumber(params.layer_end)
    local upstream_peer = (params.upstream and params.upstream ~= "") and params.upstream or nil
    local downstream_peer = (params.downstream and params.downstream ~= "") and params.downstream or nil
    local coordinator = (params.coordinator and params.coordinator ~= "") and params.coordinator or nil
    local stream_id = tonumber(params.stream_id)

    if not model or not layer_start or not layer_end or not stream_id then return end

    -- 加载模型
    local model_path
    do
        local handle = caps.storage_acquire_read(model)
        model_path = handle:path()
        handle:release()
    end

    local my_layers = layer_end - layer_start + 1
    local mid = layer_start + math.floor(my_layers / 2) - 1

    local sess_gpu0 = ml.new("cuda:0")
    sess_gpu0:load_model(model_path, layer_start, mid)

    local sess_gpu1 = ml.new("cuda:1")
    sess_gpu1:load_model(model_path, mid + 1, layer_end)

    -- 建立流
    local fwd = caps.network.accept_tensor_stream(stream_id, 300)
    local bwd = nil
    if downstream_peer then
        bwd = caps.network.open_tensor_stream(downstream_peer, stream_id)
    elseif coordinator then
        bwd = caps.network.open_tensor_stream(coordinator, stream_id)
    end

    -- 推理循环
    while true do
        local ok, hidden, recv_offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda:0")
        end)
        if not ok then break end

        local hidden0 = sess_gpu0:forward(hidden, recv_offset)
        local hidden_cpu = hidden0:to_device("cpu")
        local hidden1 = hidden_cpu:to_device("cuda:1")
        local hidden_out = sess_gpu1:forward(hidden1, recv_offset)

        caps.network.send_tensor(bwd, hidden_out, recv_offset)
    end

    sess_gpu0:unload()
    sess_gpu1:unload()
end
