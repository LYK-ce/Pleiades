-- Presented by KeJi
-- Date ： 2026-06-01

COMMAND = "pipe_8"
DESCRIPTION = "2节点×2GPU流水线后半段（带 offload）— 接收 hidden，GPU:0→GPU:1 按 chunk offload，返回 logits"

-- 由 pipe_7 通过 rexec 远程启动:
--   EXEC|pipe_8|{"model":"model_split_31_61.pgguf","inference_id":"1","chunks":"2"}

-- 辅助函数：获取第一个远程节点 (即 pipe_7)
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("pipe_8: 没有找到远程节点")
end

-- GPU chunk 迭代 forward（同 pipe_7）
-- offset==0 时用 load_model（新对话），offset>0 时用 offload_to_cuda（恢复 KV）
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
    local model_path = params.model
    local inference_id = tonumber(params.inference_id)
    local chunks_per_gpu = tonumber(params.chunks) or 2

    caps.print("pipe_8: chunks_per_gpu=" .. chunks_per_gpu)

    -- 1. 通过 Storage 获取模型路径 + 分析元数据
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    -- 将分片内层对半分给 GPU:0 和 GPU:1
    local range_size = info.split_end - info.split_start
    local mid = info.split_start + math.floor(range_size / 2)

    caps.print(string.format(
        "pipe_8: split=[%d,%d] gpu0=[%d,%d] gpu1=[%d,%d]",
        info.split_start, info.split_end,
        info.split_start, mid, mid + 1, info.split_end))

    -- 2. 为每张 GPU 创建 chunks_per_gpu 个空壳 MlSession
    local gpu0_sessions = {}
    local gpu1_sessions = {}
    local gpu0_starts = {}
    local gpu0_ends = {}
    local gpu1_starts = {}
    local gpu1_ends = {}

    -- GPU:0 各 chunk 范围
    local gpu0_range = mid - info.split_start
    for i = 0, chunks_per_gpu - 1 do
        local cs = info.split_start + math.floor(i * gpu0_range / chunks_per_gpu)
        local ce = info.split_start + math.floor((i + 1) * gpu0_range / chunks_per_gpu) - 1
        if i == chunks_per_gpu - 1 then ce = mid end
        table.insert(gpu0_starts, cs)
        table.insert(gpu0_ends, ce)
        table.insert(gpu0_sessions, ml.new("cuda:0"))
        caps.print(string.format("  GPU:0 chunk %d: [%d, %d]", i + 1, cs, ce))
    end

    -- GPU:1 各 chunk 范围
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
    caps.print("pipe_8: MlSession 创建完成 (" ..
        (chunks_per_gpu * 2) .. " 个)")

    -- 3. 发现 pipe_7 节点
    local peer_id = find_remote_peer()
    caps.print("pipe_8: 连接 pipe_7 (" .. peer_id .. ")")

    -- 4. 建立双向网络张量流
    caps.print("pipe_8: 建立网络张量流 (inference_id=" .. inference_id .. ") ...")
    local fwd = caps.network.accept_tensor_stream(inference_id, 300)
    local bwd = caps.network.open_tensor_stream(peer_id, inference_id)
    caps.print("pipe_8: 网络张量流已建立")

    -- 5. 循环: 收 hidden → GPU:0{chunks} → GPU:1{chunks} → 发 logits
    caps.print("pipe_8: 推理循环开始 (网络→GPU:0{chunks}→GPU:1{chunks})")
    while true do
        local ok, hidden, offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda:0")
        end)
        if not ok then
            caps.print("pipe_8: 流结束，退出")
            break
        end

        if offset == 0 then
            for _, s in ipairs(gpu0_sessions) do s:reset_kv_cache() end
            for _, s in ipairs(gpu1_sessions) do s:reset_kv_cache() end
        end

        -- GPU:0 chunked forward
        local hidden0 = gpu_chunked_forward(
            gpu0_sessions, hidden, offset,
            full_path, gpu0_starts, gpu0_ends)

        -- 跨 GPU 搬运: GPU:0 → CPU → GPU:1
        local hidden_cpu = hidden0:to_device("cpu")
        local hidden1 = hidden_cpu:to_device("cuda:1")

        -- GPU:1 chunked forward
        local logits = gpu_chunked_forward(
            gpu1_sessions, hidden1, offset,
            full_path, gpu1_starts, gpu1_ends)

        -- 发送 logits 回 pipe_7
        caps.network.send_tensor(bwd, logits, offset)
    end

    -- 清理
    for _, s in ipairs(gpu0_sessions) do s:unload() end
    for _, s in ipairs(gpu1_sessions) do s:unload() end
    caps.print("pipe_8: 完成")
end
