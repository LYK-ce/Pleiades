-- Presented by KeJi
-- Date: 2026-06-02
-- 动态流水线 — 演示用 Worker (双卡, Session 模式)
-- 由 pipeline_coord 通过 rexec 远程启动，不直接调用
-- 每节点 2×GPU: GPU:0→CPU→GPU:1 桥接

COMMAND = "pipe_worker"
DESCRIPTION = "动态流水线工作端: GPU:0→CPU→GPU:1, recv→forward→send"

function execute(params)
    local model = params.model
    local layer_start = tonumber(params.layer_start)
    local layer_end = tonumber(params.layer_end)
    local upstream_peer = (params.upstream and params.upstream ~= "") and params.upstream or nil
    local downstream_peer = (params.downstream and params.downstream ~= "") and params.downstream or nil
    local coordinator = (params.coordinator and params.coordinator ~= "") and params.coordinator or nil
    local session_id = params.session_id

    if not model or not layer_start or not layer_end or not session_id then
        caps.print("[worker] 缺少必要参数")
        return
    end

    local sid_num = tonumber(session_id)

    caps.print(string.format(
        "[worker] ═══ 收到分片任务: 层 [%d → %d] (共 %d 层) ═══",
        layer_start, layer_end, layer_end - layer_start + 1))

    -- 1. 获取模型路径
    local model_path
    do
        local handle = caps.storage_acquire_read(model)
        model_path = handle:path()
        handle:release()
    end

    -- 2. 自动双 GPU 分片: 前半段 cuda:0, 后半段 cuda:1
    local my_layers = layer_end - layer_start + 1
    local mid = layer_start + math.floor(my_layers / 2) - 1

    caps.print(string.format("[worker] GPU:0 ← 层 [%d,%d]  |  GPU:1 ← 层 [%d,%d]",
        layer_start, mid, mid + 1, layer_end))

    -- 加载 GPU:0
    local t_load = os.clock()
    local sess_gpu0 = ml.new("cuda:0")
    sess_gpu0:load_model(model_path, layer_start, mid)
    caps.print(string.format("[worker] GPU:0 加载完成 (%.2fs)", os.clock() - t_load))

    -- 加载 GPU:1
    t_load = os.clock()
    local sess_gpu1 = ml.new("cuda:1")
    sess_gpu1:load_model(model_path, mid + 1, layer_end)
    caps.print(string.format("[worker] GPU:1 加载完成 (%.2fs)", os.clock() - t_load))

    -- 3. 建立流
    caps.print("[worker] 等待上游张量流 (timeout=120s)...")
    local fwd = caps.network.accept_tensor_stream(sid_num, 120)
    local upstream_name = upstream_peer or "coordinator"
    caps.print(string.format("[worker] ✓ 入站流已建立 (← %s)", upstream_name))

    local bwd = nil
    if downstream_peer then
        caps.print(string.format("[worker] 打开出站流 → %s ...", downstream_peer))
        bwd = caps.network.open_tensor_stream(downstream_peer, sid_num)
        caps.print(string.format("[worker] ✓ 出站流已打开 (→ %s)", downstream_peer))
    elseif coordinator then
        caps.print(string.format("[worker] 打开返回流 → coordinator ..."))
        bwd = caps.network.open_tensor_stream(coordinator, sid_num)
        caps.print("[worker] ✓ 返回流已打开 (→ coordinator)")
    end

    caps.print("[worker] ═══ Worker 就绪，等待推理任务 ═══")

    -- 4. 推理循环
    local iter = 0
    while true do
        local ok, hidden, recv_offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda:0")
        end)
        if not ok then
            caps.print(string.format("[worker] 流结束 (共 %d 轮)", iter))
            break
        end

        iter = iter + 1
        local t_iter = os.clock()

        -- GPU:0 forward
        local t0 = os.clock()
        local hidden0 = sess_gpu0:forward(hidden, recv_offset)
        local t_gpu0 = os.clock() - t0

        -- GPU:0 → CPU → GPU:1
        local hidden_cpu = hidden0:to_device("cpu")
        local hidden1 = hidden_cpu:to_device("cuda:1")

        -- GPU:1 forward
        t0 = os.clock()
        local hidden_out = sess_gpu1:forward(hidden1, recv_offset)
        local t_gpu1 = os.clock() - t0

        -- 发送到下游
        caps.network.send_tensor(bwd, hidden_out, recv_offset)

        local elapsed = os.clock() - t_iter
        if iter == 1 then
            caps.print(string.format(
                "[worker] ⚡ Prefill: GPU0=%.2fs GPU1=%.2fs 总=%.2fs",
                t_gpu0, t_gpu1, elapsed))
        elseif iter % 20 == 0 then
            caps.print(string.format(
                "[worker] %d tokens  GPU0=%.0fms GPU1=%.0fms",
                iter, t_gpu0 * 1000, t_gpu1 * 1000))
        end
    end

    sess_gpu0:unload()
    sess_gpu1:unload()
    caps.print("[worker] 任务完成")
end
