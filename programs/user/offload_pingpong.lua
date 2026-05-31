-- Presented by KeJi
-- Date ： 2026-06-01

COMMAND = "offload_pingpong"
DESCRIPTION = "分层 offload 推理 — 两个 MlContext 轮流加载模型前后半段，服务一个 Session"

-- 场景:
--   模型太大，单 GPU 放不下完整模型。
--   sess_first: 加载前半层 (0..mid)，forward → hidden → offload_to_cpu
--   sess_second: 加载后半层 (mid+1..end)，hidden → forward → logits → 送回 Session
--   两个 context 轮换使用 GPU，通过 offload 暂存对方状态

-- 用法:
--   session create <model>              → 创建 Session
--   session inference offload_pingpong <sid> <model>  → 启动分层 ML Thread

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    caps.print("===== 分层 Offload ML Thread =====")
    caps.print("Session: " .. session_id)

    -- 1. 通过 Storage 获取模型路径 + 分析层数
    local handle = caps.storage_acquire_read(model_path)
    local path = handle:path()
    caps.print("模型路径: " .. path)

    local arch = ml.analyze_model(path)
    local total_layers = arch.num_layers + 2  -- embedding + blocks + output
    local mid = math.floor(total_layers / 2)
    caps.print(string.format("总层数: %d, 前半: 0..%d, 后半: %d..%d",
        total_layers, mid - 1, mid, total_layers - 1))

    -- 2. 创建两个空壳 MlSession（GPU）
    local sess_first = ml.new("cuda")
    local sess_second = ml.new("cuda")
    local eos = sess_first:get_eos()

    -- 3. 连接 Session 的 stream
    local stream = local_tensor.open_stream("ml-" .. session_id)
    caps.print("Stream 已连接 (ml-" .. session_id .. ")")
    handle:release()
    caps.print("ML Thread 就绪，等待 tensor ...\n")

    -- ================================================================
    -- 推理循环
    -- ================================================================

    -- 预加载前半段到 GPU（首次 prefill 用）
    caps.print("预加载前半段模型 (0.." .. (mid-1) .. ") 到 GPU ...")
    sess_first:load_model(path, 0, mid - 1)

    while true do
        -- ── 接收 tensor ──────────────────────────────────────────
        local tensor, offset = local_tensor.recv_tensor(stream, "cuda")

        -- offset=0 表示新一轮对话
        if offset == 0 then
            caps.print("\n--- 新一轮对话 ---")
            sess_first:reset_kv_cache()
            sess_second:reset_kv_cache()
        end

        -- ── 前半段 forward ───────────────────────────────────────
        local hidden = sess_first:forward(tensor, offset)
        caps.print(string.format("前半段 forward 完成 (offset=%d)", offset))

        -- Offload 前半段 → CPU，释放 GPU
        sess_first:offload_to_cpu()

        -- ── 后半段 forward ───────────────────────────────────────
        -- 如果是第一轮，先加载后半段模型
        if offset == 0 then
            caps.print(string.format("加载后半段模型 (%d..%d) 到 GPU ...", mid, total_layers - 1))
            sess_second:load_model(path, mid, total_layers - 1)
        else
            -- 不是第一轮：后半段从 CPU 恢复到 GPU
            sess_second:offload_to_cuda()
        end

        -- 后半段 forward（hidden 需要转到 GPU 上）
        -- hidden 在 sess_first 的 device 上（现在是 CPU offload 状态）
        -- 但 offload_to_cpu 时 hidden tensor 引用已被 clone，仍在 GPU
        -- 实际上 forward 返回的 hidden 是在 forward 调用时 sess_first 还在 GPU 上，
        -- 所以 hidden 在 GPU 上，可以直接给 sess_second 用
        local logits = sess_second:forward(hidden, offset)
        caps.print(string.format("后半段 forward 完成 (offset=%d)", offset))

        -- Offload 后半段 → CPU，释放 GPU
        sess_second:offload_to_cpu()

        -- ── 发送结果 ─────────────────────────────────────────────
        local_tensor.send_tensor(stream, logits, offset)

        -- ── 恢复前半段到 GPU，准备下一轮 ─────────────────────────
        sess_first:offload_to_cuda()
        caps.print("前半段已恢复到 GPU，准备下一轮")
    end
end
