-- Presented by KeJi
-- Date ： 2026-05-29

COMMAND = "pipe_4"
DESCRIPTION = "CPU/GPU 混合流水线后半段 — 分片内 CPU→GPU，接收 hidden 返回 logits"

-- 由 pipe_3 通过 rexec 远程启动: EXEC|pipe_4|{"model":"Qwen14B_split_21_41.pgguf","inference_id":"1"}

-- 辅助函数：获取第一个远程节点 peer_id (即 pipe_3)
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("pipe_4: 没有找到远程节点")
end

function execute(params)
    local model_path = params.model
    local inference_id = tonumber(params.inference_id)

    -- 1. 通过 Storage 获取模型路径 + 分析元数据
    caps.print("pipe_4: 读取模型 " .. model_path .. " ...")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    -- 计算分片内 CPU/GPU 分割点
    local range_size = info.split_end - info.split_start
    local mid = info.split_start + math.floor(range_size / 2)

    caps.print(string.format("pipe_4: split_range=[%d,%d] cpu=[%d,%d] gpu=[%d,%d]",
        info.split_start, info.split_end,
        info.split_start, mid, mid + 1, info.split_end))

    -- 2. 加载 CPU 部分
    caps.print("pipe_4: 加载 CPU 层 " .. info.split_start .. ".." .. mid)
    local cpu_sess = ml.new("cpu")
    cpu_sess:load_model(full_path, info.split_start, mid)

    -- 3. 加载 GPU 部分
    caps.print("pipe_4: 加载 GPU 层 " .. (mid + 1) .. ".." .. info.split_end)
    local gpu_sess = ml.new("cuda")
    gpu_sess:load_model(full_path, mid + 1, info.split_end)

    handle:release()
    caps.print("pipe_4: 模型加载完成 (CPU + GPU)")

    -- 4. 发现 pipe_3 节点
    local peer_id = find_remote_peer()
    caps.print("pipe_4: 连接 pipe_3 (" .. peer_id .. ")")

    -- 5. 建立双向网络张量流
    caps.print("pipe_4: 建立网络张量流 (inference_id=" .. inference_id .. ") ...")
    local fwd = caps.network.accept_tensor_stream(inference_id, 300)
    local bwd = caps.network.open_tensor_stream(peer_id, inference_id)
    caps.print("pipe_4: 网络张量流已建立")

    -- 6. 循环: 收 hidden → CPU forward → GPU forward → 发 logits
    caps.print("pipe_4: 推理循环开始 (网络→CPU→GPU)")
    while true do
        local ok, hidden, offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cpu")
        end)
        if not ok then
            caps.print("pipe_4: 流结束，退出")
            break
        end

        -- CPU 前向 → 转 GPU → GPU 前向
        local hidden_cpu = cpu_sess:forward(hidden, offset)
        local hidden_gpu = hidden_cpu:to_device("cuda")
        local logits = gpu_sess:forward(hidden_gpu, offset)

        caps.network.send_tensor(bwd, logits, gpu_sess:get_offset())
    end

    cpu_sess:unload()
    gpu_sess:unload()
    caps.print("pipe_4: 完成")
end
