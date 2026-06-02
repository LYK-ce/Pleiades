-- Presented by KeJi
-- Date: 2026-06-02
-- 动态流水线 — 单卡 Worker (Session 模式)
-- 由 pipeline_coord_single 通过 rexec 远程启动
-- 与 pipe_worker.lua 的区别: 只用 cuda:0, 无本机 GPU→CPU→GPU 桥接

COMMAND = "pipe_worker_single"
DESCRIPTION = "动态流水线工作端(单卡): 仅 cuda:0, recv→forward→send"

function execute(params)
    local model = params.model
    local layer_start = tonumber(params.layer_start)
    local layer_end = tonumber(params.layer_end)
    local upstream_peer = (params.upstream and params.upstream ~= "") and params.upstream or nil
    local downstream_peer = (params.downstream and params.downstream ~= "") and params.downstream or nil
    local coordinator = (params.coordinator and params.coordinator ~= "") and params.coordinator or nil
    local session_id = params.session_id

    if not model or not layer_start or not layer_end or not session_id then
        caps.print("[worker_single] 缺少必要参数")
        return
    end

    local sid_num = tonumber(session_id)

    caps.print(string.format(
        "[worker_single] ═══ 收到分片任务: 层 [%d → %d] (共 %d 层, 单卡) ═══",
        layer_start, layer_end, layer_end - layer_start + 1))

    -- 1. 获取模型路径
    local model_path
    do
        local handle = caps.storage_acquire_read(model)
        model_path = handle:path()
        handle:release()
    end

    -- 2. 单卡加载: 所有层放在 cuda:0
    local t_load = os.clock()
    local sess = ml.new("cuda:0")
    sess:load_model(model_path, layer_start, layer_end)
    caps.print(string.format("[worker_single] ✓ 模型加载完成 (%.2fs)", os.clock() - t_load))

    -- 3. 建立流
    caps.print("[worker_single] 等待上游张量流 (timeout=120s)...")
    local fwd = caps.network.accept_tensor_stream(sid_num, 120)
    local upstream_name = upstream_peer or "coordinator"
    caps.print(string.format("[worker_single] ✓ 入站流已建立 (← %s)", upstream_name))

    local bwd = nil
    if downstream_peer then
        bwd = caps.network.open_tensor_stream(downstream_peer, sid_num)
        caps.print(string.format("[worker_single] ✓ 出站流已打开 (→ %s)", downstream_peer))
    elseif coordinator then
        bwd = caps.network.open_tensor_stream(coordinator, sid_num)
        caps.print("[worker_single] ✓ 返回流已打开 (→ coordinator)")
    end

    caps.print("[worker_single] ═══ Worker 就绪 ═══")

    -- 4. 推理循环 (无 CPU 中转)
    local iter = 0
    while true do
        local ok, hidden, recv_offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda:0")
        end)
        if not ok then
            caps.print(string.format("[worker_single] 流结束 (共 %d 轮)", iter))
            break
        end

        iter = iter + 1
        local t0 = os.clock()
        local hidden_out = sess:forward(hidden, recv_offset)
        local t_fwd = os.clock() - t0

        caps.network.send_tensor(bwd, hidden_out, recv_offset)

        if iter == 1 then
            caps.print(string.format("[worker_single] ⚡ Prefill: %.2fs", t_fwd))
        elseif iter % 20 == 0 then
            caps.print(string.format("[worker_single] %d tokens  fwd=%.0fms", iter, t_fwd * 1000))
        end
    end

    sess:unload()
    caps.print("[worker_single] 任务完成")
end
