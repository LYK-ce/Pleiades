-- Presented by KeJi
-- Date ： 2026-06-01

COMMAND = "pipe_7"
DESCRIPTION = "2节点×2GPU流水线前半段（带 offload）— GPU:0→GPU:1 内部按 chunk offload，桥接 Session ↔ pipe_8"

-- 用法: session inference pipe_7 <session_id> <model_path> [chunks=N]
--   model_path: 前半模型文件名 (如 model_split_0_30.pgguf)
--   chunks: 每张 GPU 切几个 chunk（默认 2，即模型分成 8 块跑）

-- 辅助函数：获取第一个远程节点
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("pipe_7: 没有找到远程节点")
end

-- 辅助函数：GPU chunk 迭代 forward
-- offset==0 时用 load_model（新对话，不用旧 KV），offset>0 时用 offload_to_cuda（恢复 KV）
local function gpu_chunked_forward(sessions, input, offset, path, starts, ends)
    local hidden = input
    for i = 1, #sessions do
        if offset == 0 then
            sessions[i]:load_model(path, starts[i], ends[i])
        else
            sessions[i]:offload_to_cuda()
        end
        hidden = sessions[i]:forward(hidden, offset)
        sessions[i]:offload_to_cpu()
    end
    return hidden
end

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path
    local chunks_per_gpu = tonumber(params.chunks) or 2

    caps.print("pipe_7: chunks_per_gpu=" .. chunks_per_gpu)

    -- 1. 通过 Storage 获取模型路径 + 分析元数据
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    if not info.is_split then
        caps.print("pipe_7: 错误 — 模型不是 split PGGUF")
        handle:release()
        return
    end

    -- 将分片内层对半分给 GPU:0 和 GPU:1
    local range_size = info.split_end - info.split_start
    local mid = info.split_start + math.floor(range_size / 2)

    -- 计算远程模型名（pipe_8 用）
    local stem = string.match(model_path, "^(.-)_split_")
    local total_layers = info.num_layers + 2
    local remote_start = info.split_end + 1
    local remote_end = total_layers - 1
    local remote_model = stem .. "_split_" .. remote_start .. "_" .. remote_end .. ".pgguf"

    caps.print(string.format(
        "pipe_7: split=[%d,%d] total=%d gpu0=[%d,%d] gpu1=[%d,%d] remote=%s",
        info.split_start, info.split_end, total_layers,
        info.split_start, mid, mid + 1, info.split_end, remote_model))

    -- 2. 为每张 GPU 创建 chunks_per_gpu 个空壳 MlSession
    local gpu0_sessions = {}
    local gpu1_sessions = {}

    -- 计算 GPU:0 的各 chunk 范围
    local gpu0_starts = {}
    local gpu0_ends = {}
    local gpu0_range = mid - info.split_start
    for i = 0, chunks_per_gpu - 1 do
        local cs = info.split_start + math.floor(i * gpu0_range / chunks_per_gpu)
        local ce = info.split_start + math.floor((i + 1) * gpu0_range / chunks_per_gpu) - 1
        -- 最后一个 chunk 包含 end
        if i == chunks_per_gpu - 1 then ce = mid end
        table.insert(gpu0_starts, cs)
        table.insert(gpu0_ends, ce)
        table.insert(gpu0_sessions, ml.new("cuda:0"))
        caps.print(string.format("  GPU:0 chunk %d: [%d, %d]", i + 1, cs, ce))
    end

    -- 计算 GPU:1 的各 chunk 范围
    local gpu1_starts = {}
    local gpu1_ends = {}
    local gpu1_range = info.split_end - mid
    for i = 0, chunks_per_gpu - 1 do
        local cs = mid + 1 + math.floor(i * gpu1_range / chunks_per_gpu)
        local ce = mid + 1 + math.floor((i + 1) * gpu1_range / chunks_per_gpu) - 1
        if i == chunks_per_gpu - 1 then ce = info.split_end end
        table.insert(gpu1_starts, cs)
        table.insert(gpu1_ends, ce)
        table.insert(gpu1_sessions, ml.new("cuda:1"))
        caps.print(string.format("  GPU:1 chunk %d: [%d, %d]", i + 1, cs, ce))
    end

    handle:release()
    caps.print("pipe_7: MlSession 创建完成 (" ..
        (chunks_per_gpu * 2) .. " 个)")

    -- 3. 连接 Session
    local stream_id = "ml-" .. session_id
    caps.print("pipe_7: 连接 Session (stream: " .. stream_id .. ") ...")
    local session_stream = local_tensor.open_stream(stream_id)

    -- 4. 发现远程节点，通过 rexec 启动 pipe_8
    local peer_id = find_remote_peer()
    caps.print("pipe_7: 远程节点 " .. peer_id)

    local exec_payload = 'EXEC|pipe_8|{"model":"' .. remote_model ..
        '","inference_id":"' .. session_id ..
        '","chunks":"' .. chunks_per_gpu .. '"}'
    caps.print("pipe_7: rexec → " .. exec_payload)
    local resp = caps.network.send_data(peer_id, "Command", exec_payload)
    caps.print("pipe_7: pipe_8 启动响应: " .. (resp.payload or "nil"))

    -- 5. 建立双向网络张量流
    local sid_num = tonumber(session_id)
    caps.print("pipe_7: 建立网络张量流 (inference_id=" .. session_id .. ") ...")
    local fwd = caps.network.open_tensor_stream(peer_id, sid_num)
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print("pipe_7: 网络张量流已建立")

    -- 6. 桥接循环
    caps.print("pipe_7: 桥接循环开始 (GPU:0{chunks}→GPU:1{chunks}→网络)")
    while true do
        -- 从 Session 收 tensor (放在 GPU:0)
        local tensor, offset = local_tensor.recv_tensor(session_stream, "cuda:0")

        if offset == 0 then
            -- 新一轮对话：重置所有 session 的 KV Cache
            -- （next offload_to_cuda 时 load_model 重建，无需恢复旧 KV）
            for _, s in ipairs(gpu0_sessions) do s:reset_kv_cache() end
            for _, s in ipairs(gpu1_sessions) do s:reset_kv_cache() end
        end

        -- GPU:0 chunked forward
        local hidden0 = gpu_chunked_forward(
            gpu0_sessions, tensor, offset,
            full_path, gpu0_starts, gpu0_ends)

        -- 跨 GPU 搬运: GPU:0 → CPU → GPU:1
        local hidden_cpu = hidden0:to_device("cpu")
        local hidden1 = hidden_cpu:to_device("cuda:1")

        -- GPU:1 chunked forward
        local hidden_out = gpu_chunked_forward(
            gpu1_sessions, hidden1, offset,
            full_path, gpu1_starts, gpu1_ends)

        -- 发送到 pipe_8
        caps.network.send_tensor(fwd, hidden_out, offset)

        -- 接收 pipe_8 返回的 logits
        local logits, _ = caps.network.recv_tensor(bwd, "cuda:0")

        -- 回传 logits 给 Session
        local_tensor.send_tensor(session_stream, logits, offset)
    end
end
