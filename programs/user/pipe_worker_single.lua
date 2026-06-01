-- Presented by KeJi
-- Date: 2026-06-01
-- 动态流水线 — 单卡 Worker
-- 由 pipeline_coord_single 通过 rexec 远程启动，不直接调用
-- 与 pipe_worker.lua 的区别: 只用 cuda:0, 无本机 GPU→CPU→GPU 桥接

COMMAND = "pipe_worker_single"
DESCRIPTION = "动态流水线工作端 (单卡): 仅 cuda:0, recv hidden→forward→send hidden"

function execute(params)
    local model = params.model
    local layer_start = tonumber(params.layer_start)
    local layer_end = tonumber(params.layer_end)
    local upstream_peer = (params.upstream and params.upstream ~= "") and params.upstream or nil
    local downstream_peer = (params.downstream and params.downstream ~= "") and params.downstream or nil
    local coordinator = (params.coordinator and params.coordinator ~= "") and params.coordinator or nil
    local inference_id = tonumber(params.inference_id)

    if not model or not layer_start or not layer_end or not inference_id then
        caps.print("[worker_single] 缺少必要参数 (model/layer_start/layer_end/inference_id)")
        return
    end

    caps.print(string.format("[worker_single] layers=[%d,%d] up=%s down=%s coord=%s id=%d",
        layer_start, layer_end, upstream_peer or "nil", downstream_peer or "nil",
        coordinator or "nil", inference_id))

    -- 1. 通过 Storage 获取模型路径
    local model_path
    do
        local handle = caps.storage_acquire_read(model)
        model_path = handle:path()
        handle:release()
    end
    caps.print("[worker_single] 模型: " .. model_path)

    -- 2. 单卡加载: 所有层放在 cuda:0
    caps.print(string.format("[worker_single] cuda:0 ← [%d,%d]", layer_start, layer_end))
    local sess = ml.new("cuda:0")
    sess:load_model(model_path, layer_start, layer_end)
    caps.print("[worker_single] 模型加载完成")

    -- 3. 建立流
    caps.print("[worker_single] 等待入站 stream (timeout=120s)...")
    local fwd = caps.network.accept_tensor_stream(inference_id, 120)
    caps.print("[worker_single] 入站 stream 已建立")

    local bwd = nil
    if downstream_peer then
        caps.print("[worker_single] 打开出站 stream → " .. downstream_peer)
        bwd = caps.network.open_tensor_stream(downstream_peer, inference_id)
        caps.print("[worker_single] 出站 stream 已打开")
    elseif coordinator then
        caps.print("[worker_single] 打开返回 stream → coordinator " .. coordinator)
        bwd = caps.network.open_tensor_stream(coordinator, inference_id)
        caps.print("[worker_single] 返回 stream 已打开")
    end

    -- 4. 推理循环: 收 → forward → 发 (无 CPU 中转)
    local iter = 0
    while true do
        iter = iter + 1

        local ok, hidden, recv_offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda:0")
        end)
        if not ok then
            caps.print("[worker_single] 流结束, 退出")
            break
        end

        local t0 = os.clock()
        local hidden_out = sess:forward(hidden, recv_offset)
        caps.print(string.format("[worker_single] forward done (%.3fs)", os.clock() - t0))

        caps.network.send_tensor(bwd, hidden_out, recv_offset)
        caps.print(string.format("[worker_single] iter %d 完成", iter))
    end

    sess:unload()
    caps.print("[worker_single] 完成")
end
