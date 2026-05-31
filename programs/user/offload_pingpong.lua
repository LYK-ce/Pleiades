-- Presented by KeJi
-- Date ： 2026-06-01

COMMAND = "offload_pingpong"
DESCRIPTION = "单机双 Session 轮流 offload ML Thread — 一个 ML 线程管理两个 Session，通过 offload 交替推理"

-- 用法:
--   session create <model>        → 创建 Session A (得到 id=1)
--   session create <model>        → 创建 Session B (得到 id=2)
--   session inference offload_pingpong 1 <model>  → 启动 ML Thread
--     （session_B 参数由 session_id+1 自动推导）
--
-- 或者显式指定两个 session:
--   exec offload_pingpong session_A=1 session_B=2 model_path=<model>

function execute(params)
    -- 支持两种参数方式：
    -- 1. session inference 调用: params.session_id (字符串), params.model_path
    -- 2. exec 调用: params.session_A, params.session_B (都是字符串), params.model_path
    local session_id_A = params.session_A or params.session_id
    local session_id_B = params.session_B or tostring(tonumber(session_id_A) + 1)
    local model_path = params.model_path

    caps.print("===== Offload Ping-Pong ML Thread =====")
    caps.print("Session A: " .. session_id_A)
    caps.print("Session B: " .. session_id_B)

    -- 1. 通过 Storage 获取模型路径
    local handle = caps.storage_acquire_read(model_path)
    local path = handle:path()
    caps.print("模型路径: " .. path)

    -- 2. 创建 MlSession（在 GPU 上）
    local sess = ml.new("cuda")
    local eos = sess:get_eos()

    -- 3. 打开两个 Session 的 stream（Session 的 tokio task 在 session create 时就已启动并在 accept_async 等待）
    local stream_A = local_tensor.open_stream("ml-" .. session_id_A)
    caps.print("Session A stream 已连接 (ml-" .. session_id_A .. ")")

    local stream_B = local_tensor.open_stream("ml-" .. session_id_B)
    caps.print("Session B stream 已连接 (ml-" .. session_id_B .. ")")

    handle:release()
    caps.print("ML Thread 就绪，开始乒乓推理 ...\n")

    -- ================================================================
    -- 乒乓循环：在 A 和 B 之间交替
    -- ================================================================
    local active = "A"  -- 当前活跃的 session
    local round = 0

    -- 每个 session 的上下文状态
    local ctx = {
        A = { stream = stream_A, offloaded = false, offset = 0 },
        B = { stream = stream_B, offloaded = false, offset = 0 },
    }

    -- 加载模型到 GPU（首次）
    sess:load_model(path, 0, 999999)
    caps.print("模型已加载到 GPU")

    while true do
        round = round + 1
        local cur = ctx[active]
        caps.print(string.format("\n--- Round %d: Session %s ---", round, active))

        -- 如果当前 session 之前被 offload 了，先恢复到 GPU
        if cur.offloaded then
            caps.print("Session " .. active .. ": 从 CPU 恢复到 GPU ...")
            sess:offload_to_cuda()
            cur.offloaded = false
        end

        -- 处理来自 Session 的推理请求（最多处理 3 个 request，然后切换）
        local requests_handled = 0
        while requests_handled < 3 do
            -- 接收 tensor（带超时，超时后切换 session）
            local ok, result = pcall(function()
                return local_tensor.recv_tensor(cur.stream, "cuda")
            end)

            if not ok then
                caps.print("Session " .. active .. ": 无更多请求，准备切换")
                break
            end

            local tensor, offset = result

            -- offset=0 表示新一轮对话
            if offset == 0 then
                sess:reset_kv_cache()
                cur.offset = 0
            end

            -- Forward
            local logits = sess:forward(tensor, offset)
            local_tensor.send_tensor(cur.stream, logits, offset)
            cur.offset = offset + 1
            requests_handled = requests_handled + 1
        end

        -- 收到 EOF（offset=u64::MAX, length=0），退出循环
        if requests_handled == 0 and cur.offloaded == false then
            -- 可能 session 已关闭
        end

        -- Offload 当前 session 到 CPU，切换到另一个
        caps.print("Session " .. active .. ": offload to CPU ...")
        sess:offload_to_cpu()
        cur.offloaded = true

        -- 切换
        active = (active == "A") and "B" or "A"
    end
end
