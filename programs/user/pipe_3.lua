-- Presented by KeJi
-- Date ： 2026-05-29

COMMAND = "pipe_3"
DESCRIPTION = "CPU/GPU 混合流水线前半段 — 分片内 CPU→GPU，桥接 Session ↔ pipe_4"

-- 用法: session inference pipe_3 <session_id> <model_path>
--   pipe_3 将分片内部再拆分为 CPU 和 GPU 两部分：
--     CPU: split_start .. mid
--     GPU: mid+1 .. split_end
--   每轮: CPU forward → to_device("cuda") → GPU forward → 网络发送

-- 辅助函数：获取第一个远程节点 peer_id
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("pipe_3: 没有找到远程节点")
end

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    -- 1. 通过 Storage 获取模型路径 + 分析元数据
    caps.print("pipe_3: 读取模型 " .. model_path .. " ...")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    if not info.is_split then
        caps.print("pipe_3: 错误 — 模型不是 split PGGUF")
        handle:release()
        return
    end

    -- 计算分片内 CPU/GPU 分割点
    local range_size = info.split_end - info.split_start
    local mid = info.split_start + math.floor(range_size / 2)

    -- 计算远程模型名
    local stem = string.match(model_path, "^(.-)_split_")
    local total_layers = info.num_layers + 2
    local remote_start = info.split_end + 1
    local remote_end = total_layers - 1
    local remote_model = stem .. "_split_" .. remote_start .. "_" .. remote_end .. ".pgguf"

    caps.print(string.format("pipe_3: split_range=[%d,%d] total=%d cpu=[%d,%d] gpu=[%d,%d] remote=%s",
        info.split_start, info.split_end, total_layers,
        info.split_start, mid, mid + 1, info.split_end, remote_model))

    -- 2. 加载 CPU 部分（前半层）
    caps.print("pipe_3: 加载 CPU 层 " .. info.split_start .. ".." .. mid)
    local cpu_sess = ml.new("cpu")
    cpu_sess:load_model(full_path, info.split_start, mid)

    -- 3. 加载 GPU 部分（后半层）
    caps.print("pipe_3: 加载 GPU 层 " .. (mid + 1) .. ".." .. info.split_end)
    local gpu_sess = ml.new("cuda")
    gpu_sess:load_model(full_path, mid + 1, info.split_end)

    handle:release()
    caps.print("pipe_3: 模型加载完成 (CPU + GPU)")

    -- 4. 发现远程节点，通过 rexec 启动 pipe_4
    local peer_id = find_remote_peer()
    caps.print("pipe_3: 远程节点 " .. peer_id)

    local exec_payload = 'EXEC|pipe_4|{"model":"' .. remote_model ..
        '","inference_id":"' .. session_id .. '"}'
    caps.print("pipe_3: rexec → " .. exec_payload)
    local resp = caps.network.send_data(peer_id, "Command", exec_payload)
    caps.print("pipe_3: pipe_4 启动响应: " .. (resp.payload or "nil"))

    -- 5. 连接 Session
    local stream_id = "ml-" .. session_id
    caps.print("pipe_3: 连接 Session (stream: " .. stream_id .. ") ...")
    local session_stream = local_tensor.open_stream(stream_id)

    -- 6. 建立双向网络张量流
    local sid_num = tonumber(session_id)
    caps.print("pipe_3: 建立网络张量流 (inference_id=" .. session_id .. ") ...")
    local fwd = caps.network.open_tensor_stream(peer_id, sid_num)
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print("pipe_3: 网络张量流已建立")

    -- 7. 桥接循环: Session ↔ CPU ↔ GPU ↔ pipe_4
    caps.print("pipe_3: 桥接循环开始 (CPU→GPU→网络)")
    while true do
        -- 从 Session 收 tensor
        local tensor, offset = local_tensor.recv_tensor(session_stream, "cpu")
        if offset == 0 then
            cpu_sess:reset_kv_cache()
            gpu_sess:reset_kv_cache()
        end

        -- CPU 前向 → 转 GPU → GPU 前向
        local hidden_cpu = cpu_sess:forward(tensor, offset)
        local hidden_gpu = hidden_cpu:to_device("cuda")
        local hidden_out = gpu_sess:forward(hidden_gpu, offset)

        -- 发送到 pipe_4
        caps.network.send_tensor(fwd, hidden_out, offset)

        -- 接收 pipe_4 返回的 logits
        local logits, _ = caps.network.recv_tensor(bwd, "cpu")

        -- 回传 Session
        local_tensor.send_tensor(session_stream, logits, offset)
    end
end
