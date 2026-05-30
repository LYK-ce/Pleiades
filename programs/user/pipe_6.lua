-- Presented by KeJi
-- Date ： 2026-05-30

COMMAND = "pipe_6"
DESCRIPTION = "2节点×2GPU流水线后半段 — 接收 hidden，GPU:0→GPU:1 内部流水，返回 logits"

-- 由 pipe_5 通过 rexec 远程启动:
--   EXEC|pipe_6|{"model":"DeepSeek-V3.2-UD-TQ1_0_split_31_61.pgguf","inference_id":"1"}

-- 辅助函数：获取第一个远程节点 peer_id (即 pipe_5)
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("pipe_6: 没有找到远程节点")
end

function execute(params)
    local model_path = params.model
    local inference_id = tonumber(params.inference_id)

    -- 1. 通过 Storage 获取模型路径 + 分析元数据
    caps.print("pipe_6: 读取模型 " .. model_path .. " ...")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    -- 将分片内层对半分给 GPU:0 和 GPU:1
    local range_size = info.split_end - info.split_start
    local mid = info.split_start + math.floor(range_size / 2)
    -- GPU:0 负责 [split_start, mid]
    -- GPU:1 负责 [mid+1, split_end]

    caps.print(string.format(
        "pipe_6: split_range=[%d,%d] gpu0=[%d,%d] gpu1=[%d,%d]",
        info.split_start, info.split_end,
        info.split_start, mid, mid + 1, info.split_end))

    -- 2. 加载 GPU:0 部分
    caps.print("pipe_6: 加载 GPU:0 层 " .. info.split_start .. ".." .. mid)
    local gpu0_sess = ml.new("cuda:0")
    gpu0_sess:load_model(full_path, info.split_start, mid)

    -- 3. 加载 GPU:1 部分
    caps.print("pipe_6: 加载 GPU:1 层 " .. (mid + 1) .. ".." .. info.split_end)
    local gpu1_sess = ml.new("cuda:1")
    gpu1_sess:load_model(full_path, mid + 1, info.split_end)

    handle:release()
    caps.print("pipe_6: 模型加载完成 (GPU:0 + GPU:1)")

    -- 4. 发现 pipe_5 节点
    local peer_id = find_remote_peer()
    caps.print("pipe_6: 连接 pipe_5 (" .. peer_id .. ")")

    -- 5. 建立双向网络张量流
    caps.print("pipe_6: 建立网络张量流 (inference_id=" .. inference_id .. ") ...")
    local fwd = caps.network.accept_tensor_stream(inference_id, 300)
    local bwd = caps.network.open_tensor_stream(peer_id, inference_id)
    caps.print("pipe_6: 网络张量流已建立")

    -- 6. 循环: 收 hidden → GPU:0 forward → GPU:1 forward → 发 logits
    caps.print("pipe_6: 推理循环开始 (网络→GPU:0→GPU:1)")
    while true do
        local ok, hidden, offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda:0")
        end)
        if not ok then
            caps.print("pipe_6: 流结束，退出")
            break
        end

        -- GPU:0 前向
        local hidden0 = gpu0_sess:forward(hidden, offset)

        -- 跨 GPU 搬运: GPU:0 → GPU:1
        local hidden1 = hidden0:to_device("cuda:1")

        -- GPU:1 前向
        local logits = gpu1_sess:forward(hidden1, offset)

        -- 发送 logits 回 pipe_5
        caps.network.send_tensor(bwd, logits, gpu1_sess:get_offset())
    end

    gpu0_sess:unload()
    gpu1_sess:unload()
    caps.print("pipe_6: 完成")
end
