-- Presented by KeJi
-- Date: 2026-06-03
-- 动态流水线 — 单卡 Worker
-- 由 coordinator 通过 rexec 远程启动，不直接调用
-- 仅使用 cuda:0，无 GPU→CPU→GPU 桥接

COMMAND = "pipe_worker_single"
DESCRIPTION = "动态流水线单卡工作端: cuda:0 only, recv→forward→send"

function execute(params)
    if type(params) == "string" then
        local ok, parsed = pcall(caps.json_decode, params)
        if ok then params = parsed end
    end

    local model = params.model
    caps.print("[worker_single] 收到远程命令, model=" .. (model or "nil"))
    local upstream_peer = (params.upstream and params.upstream ~= "") and params.upstream or nil
    local downstream_peer = (params.downstream and params.downstream ~= "") and params.downstream or nil
    local coordinator = (params.coordinator and params.coordinator ~= "") and params.coordinator or nil
    local session_id = params.session_id

    if not model or not session_id then
        caps.print("[worker_single] 缺少必要参数")
        return
    end

    local sid_num = tonumber(session_id)

    -- 1. 获取模型路径，优先用 coordinator 传来的层范围
    local layer_start = tonumber(params.layer_start)
    local layer_end = tonumber(params.layer_end)
    local model_path
    do
        local handle = caps.storage_acquire_read(model)
        model_path = handle:path()
        -- 如果没有传层范围，从 PGGUF 元数据读取 (fallback)
        if not layer_start or not layer_end then
            local info = ml.analyze_model(model_path)
            layer_start = info.split_start
            layer_end = info.split_end
        end
        handle:release()
    end

    caps.print(string.format(
        "[worker_single] ═══ 收到分片任务: 层 [%d → %d] (共 %d 层) ═══",
        layer_start, layer_end, layer_end - layer_start + 1))

    -- 2. 单卡加载所有层
    local t_load = caps.monotonic_time()
    local sess = ml.new("cuda:0")
    sess:load_model(model_path, layer_start, layer_end)
    caps.print(string.format("[worker_single] cuda:0 加载完成 (%.2fs)", caps.monotonic_time() - t_load))
    caps.print("[pipe_worker_single] 模型加载完毕, cuda:0 ready")

    -- 3. 建立流
    caps.print("[worker_single] 等待上游张量流 (timeout=300s)...")
    local fwd = caps.network.accept_tensor_stream(sid_num, 300)
    local upstream_name = upstream_peer or "coordinator"
    caps.print(string.format("[worker_single] ✓ 入站流已建立 (← %s)", upstream_name))

    local bwd = nil
    if downstream_peer then
        caps.print(string.format("[worker_single] 打开出站流 → %s ...", downstream_peer))
        bwd = caps.network.open_tensor_stream(downstream_peer, sid_num)
        caps.print(string.format("[worker_single] ✓ 出站流已打开 (→ %s)", downstream_peer))
    elseif coordinator then
        caps.print("[worker_single] 打开返回流 → coordinator ...")
        bwd = caps.network.open_tensor_stream(coordinator, sid_num)
        caps.print("[worker_single] ✓ 返回流已打开 (→ coordinator)")
    end

    caps.print("[worker_single] ═══ Worker 就绪，等待推理任务 ═══")

    -- 4. 推理循环（单卡，无 CPU 桥接）
    local iter = 0
    while true do
        local ok, hidden, recv_offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda:0")
        end)
        if not ok then
            caps.print(string.format("[worker_single] 流结束 (共 %d 轮)", iter))
            break
        end

        if recv_offset == 0 then
            sess:reset_kv_cache()
        end

        iter = iter + 1
        local t_iter = caps.monotonic_time()

        -- 单卡 forward
        local t_fwd = caps.monotonic_time()
        local hidden_out = sess:forward(hidden, recv_offset)
        local t_fwd_elapsed = caps.monotonic_time() - t_fwd

        -- 发送到下游
        caps.network.send_tensor(bwd, hidden_out, recv_offset)

        local elapsed = caps.monotonic_time() - t_iter
        if iter == 1 then
            caps.print(string.format(
                "[worker_single] ⚡ Prefill: FWD=%.2fs 总=%.2fs",
                t_fwd_elapsed, elapsed))
        elseif iter % 20 == 0 then
            caps.print(string.format(
                "[worker_single] %d tokens  FWD=%.0fms",
                iter, t_fwd_elapsed * 1000))
        end
    end

    sess:unload()
    caps.print("[worker_single] 任务完成")
end
