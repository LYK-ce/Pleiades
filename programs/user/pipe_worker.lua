-- Presented by KeJi
-- Date: 2026-06-01
-- 动态流水线 — 双卡 Worker
-- 由 pipeline_coord 通过 rexec 远程启动，不直接调用

COMMAND = "pipe_worker"
DESCRIPTION = "动态流水线工作端 (双卡): GPU:0→CPU→GPU:1, recv hidden→forward→send hidden"

function execute(params)
    local model = params.model
    local layer_start = tonumber(params.layer_start)
    local layer_end = tonumber(params.layer_end)
    local upstream_peer = (params.upstream and params.upstream ~= "") and params.upstream or nil
    local downstream_peer = (params.downstream and params.downstream ~= "") and params.downstream or nil
    local coordinator = (params.coordinator and params.coordinator ~= "") and params.coordinator or nil
    local inference_id = tonumber(params.inference_id)

    if not model or not layer_start or not layer_end or not inference_id then
        caps.print("[worker] 缺少必要参数 (model/layer_start/layer_end/inference_id)")
        return
    end

    caps.print(string.format("[worker] layers=[%d,%d] up=%s down=%s coord=%s id=%d",
        layer_start, layer_end, upstream_peer or "nil", downstream_peer or "nil",
        coordinator or "nil", inference_id))

    -- 1. 通过 Storage 获取模型路径
    local model_path
    do
        local handle = caps.storage_acquire_read(model)
        model_path = handle:path()
        handle:release()
    end
    caps.print("[worker] 模型: " .. model_path)

    -- 2. 自动双 GPU 分片: 前半段 cuda:0, 后半段 cuda:1
    local my_layers = layer_end - layer_start + 1
    local mid = layer_start + math.floor(my_layers / 2) - 1

    caps.print(string.format("[worker] cuda:0 ← [%d,%d]  cuda:1 ← [%d,%d]",
        layer_start, mid, mid + 1, layer_end))

    local sess_gpu0 = ml.new("cuda:0")
    sess_gpu0:load_model(model_path, layer_start, mid)
    caps.print("[worker] GPU:0 模型加载完成")

    local sess_gpu1 = ml.new("cuda:1")
    sess_gpu1:load_model(model_path, mid + 1, layer_end)
    caps.print("[worker] GPU:1 模型加载完成")

    -- 3. 建立流: 总是 accept 入站，有下游则 open 出站，否则 open 回 coordinator
    caps.print("[worker] 等待入站 stream (timeout=120s)...")
    local fwd = caps.network.accept_tensor_stream(inference_id, 120)
    caps.print("[worker] 入站 stream 已建立")

    local bwd = nil
    if downstream_peer then
        caps.print("[worker] 打开出站 stream → " .. downstream_peer)
        bwd = caps.network.open_tensor_stream(downstream_peer, inference_id)
        caps.print("[worker] 出站 stream 已打开")
    elseif coordinator then
        caps.print("[worker] 打开返回 stream → coordinator " .. coordinator)
        bwd = caps.network.open_tensor_stream(coordinator, inference_id)
        caps.print("[worker] 返回 stream 已打开")
    end

    -- 4. 推理循环: 收 → GPU:0 forward → CPU 中转 → GPU:1 forward → 发
    local iter = 0
    while true do
        iter = iter + 1

        -- 接收 hidden (放在 GPU:0)
        local ok, hidden, recv_offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda:0")
        end)
        if not ok then
            caps.print("[worker] 流结束, 退出")
            break
        end

        -- GPU:0 forward
        local t0 = os.clock()
        local hidden0 = sess_gpu0:forward(hidden, recv_offset)
        caps.print(string.format("[worker] GPU:0 forward done (%.3fs)", os.clock() - t0))

        -- GPU:0 → CPU → GPU:1
        local hidden_cpu = hidden0:to_device("cpu")
        local hidden1 = hidden_cpu:to_device("cuda:1")

        -- GPU:1 forward
        t0 = os.clock()
        local hidden_out = sess_gpu1:forward(hidden1, recv_offset)
        caps.print(string.format("[worker] GPU:1 forward done (%.3fs)", os.clock() - t0))

        -- 发送到下游 (或 coordinator)
        caps.network.send_tensor(bwd, hidden_out, recv_offset)

        caps.print(string.format("[worker] iter %d 完成", iter))
    end

    sess_gpu0:unload()
    sess_gpu1:unload()
    caps.print("[worker] 完成")
end
