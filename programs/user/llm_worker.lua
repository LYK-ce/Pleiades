-- Presented by KeJi
-- Created Date: 2026-09-09
-- Modified Date: 2026-09-09

-- 预分片流水线 Worker（小车跑）
-- 由 llm（coord）通过 rexec 远程启动，不直接调用
-- 加载本车持有的分片（读 split_start/split_end），recv hidden → forward → send logits

COMMAND = "llm_worker"
DESCRIPTION = "预分片流水线工作端: 读分片范围 → forward → send logits"

-- ═══ 配置区（在小车上按实际修改）═══
local device = "cpu"                              -- 推理设备: cpu / cuda:0 / cuda:1
local model = "Qwen3-8B-Q4_K_M_split_0_18.pgguf"  -- 本车实际持有的分片文件名

function execute(params)
    -- 如果 params 是 JSON 字符串，先解析
    if type(params) == "string" then
        local ok, parsed = pcall(caps.json_decode, params)
        if ok then params = parsed end
    end

    caps.print("[llm_worker] 收到远程命令, model=" .. model)
    local upstream_peer = (params.upstream and params.upstream ~= "") and params.upstream or nil
    local downstream_peer = (params.downstream and params.downstream ~= "") and params.downstream or nil
    local coordinator = (params.coordinator and params.coordinator ~= "") and params.coordinator or nil
    local session_id = params.session_id

    if not session_id then
        caps.print("[llm_worker] 缺少必要参数 session_id")
        return
    end

    local sid_num = tonumber(session_id)

    -- 1. 读分片范围（从分片 metadata 的 split_start/split_end）
    local model_path
    local layer_start, layer_end
    do
        local handle = caps.storage_acquire_read(model)
        model_path = handle:path()
        local info = ml.analyze_model(model_path)
        layer_start = info.split_start
        layer_end = info.split_end
        handle:release()
    end

    caps.print(string.format(
        "[llm_worker] ═══ 分片任务: 层 [%d → %d] (共 %d 层) ═══",
        layer_start, layer_end, layer_end - layer_start + 1))

    -- 2. 加载分片
    local t_load = caps.monotonic_time()
    local sess = ml.new(device)
    sess:load_model(model_path, layer_start, layer_end)
    caps.print(string.format("[llm_worker] %s 加载完成 (%.2fs)", device, caps.monotonic_time() - t_load))

    -- 3. 建立流
    caps.print("[llm_worker] 等待上游张量流 (timeout=300s)...")
    local fwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print("[llm_worker] ✓ 入站流已建立")

    local bwd = nil
    if downstream_peer then
        caps.print(string.format("[llm_worker] 打开出站流 → %s ...", downstream_peer))
        bwd = caps.network.open_tensor_stream(downstream_peer, sid_num)
        caps.print("[llm_worker] ✓ 出站流已打开")
    elseif coordinator then
        caps.print("[llm_worker] 打开返回流 → coordinator ...")
        bwd = caps.network.open_tensor_stream(coordinator, sid_num)
        caps.print("[llm_worker] ✓ 返回流已打开")
    end

    caps.print("[llm_worker] ═══ Worker 就绪，等待推理任务 ═══")

    -- 4. 推理循环
    local iter = 0
    while true do
        local ok, hidden, recv_offset = pcall(function()
            return caps.network.recv_tensor(fwd, device)
        end)
        if not ok then
            caps.print(string.format("[llm_worker] 流结束 (共 %d 轮)", iter))
            break
        end

        if recv_offset == 0 then
            sess:reset_kv_cache()
        end

        iter = iter + 1
        local t_fwd = caps.monotonic_time()
        local hidden_out = sess:forward(hidden, recv_offset)
        local t_fwd_elapsed = caps.monotonic_time() - t_fwd

        caps.network.send_tensor(bwd, hidden_out, recv_offset)

        if iter == 1 then
            caps.print(string.format("[llm_worker] ⚡ Prefill: FWD=%.2fs", t_fwd_elapsed))
        elseif iter % 20 == 0 then
            caps.print(string.format("[llm_worker] %d tokens  FWD=%.0fms", iter, t_fwd_elapsed * 1000))
        end
    end

    sess:unload()
    caps.print("[llm_worker] 任务完成")
end
