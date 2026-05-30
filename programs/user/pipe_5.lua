-- Presented by KeJi
-- Date ： 2026-05-30

COMMAND = "pipe_5"
DESCRIPTION = "2节点×2GPU流水线前半段 — GPU:0→GPU:1 内部流水，桥接 Session ↔ pipe_6"

-- 用法: session inference pipe_5 <session_id> <model_path>
--   model_path: 前半模型文件名 (如 DeepSeek-V3.2-UD-TQ1_0_split_0_30.pgguf)
--   pipe_5 自动通过 PGGUF split 元数据推导远程模型名，并 rexec 启动 pipe_6

-- 辅助函数：获取第一个远程节点 peer_id
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("pipe_5: 没有找到远程节点")
end

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    -- 1. 通过 Storage 获取前半模型路径 + 分析元数据
    caps.print("pipe_5: 读取模型 " .. model_path .. " ...")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    if not info.is_split then
        caps.print("pipe_5: 错误 — 模型不是 split PGGUF")
        handle:release()
        return
    end

    -- 将分片内层对半分给 GPU:0 和 GPU:1
    local range_size = info.split_end - info.split_start
    local mid = info.split_start + math.floor(range_size / 2)
    -- GPU:0 负责 [split_start, mid]
    -- GPU:1 负责 [mid+1, split_end]

    -- 计算远程模型名
    local stem = string.match(model_path, "^(.-)_split_")
    local total_layers = info.num_layers + 2
    local remote_start = info.split_end + 1
    local remote_end = total_layers - 1
    local remote_model = stem .. "_split_" .. remote_start .. "_" .. remote_end .. ".pgguf"

    caps.print(string.format(
        "pipe_5: split_range=[%d,%d] total=%d gpu0=[%d,%d] gpu1=[%d,%d] remote=%s",
        info.split_start, info.split_end, total_layers,
        info.split_start, mid, mid + 1, info.split_end, remote_model))

    -- 2. 加载 GPU:0 部分
    caps.print("pipe_5: 加载 GPU:0 层 " .. info.split_start .. ".." .. mid)
    local gpu0_sess = ml.new("cuda:0")
    gpu0_sess:load_model(full_path, info.split_start, mid)

    -- 3. 加载 GPU:1 部分
    caps.print("pipe_5: 加载 GPU:1 层 " .. (mid + 1) .. ".." .. info.split_end)
    local gpu1_sess = ml.new("cuda:1")
    gpu1_sess:load_model(full_path, mid + 1, info.split_end)

    handle:release()
    caps.print("pipe_5: 模型加载完成 (GPU:0 + GPU:1)")

    -- 4. 连接 Session (local_tensor)
    local stream_id = "ml-" .. session_id
    caps.print("pipe_5: 连接 Session (stream: " .. stream_id .. ") ...")
    local session_stream = local_tensor.open_stream(stream_id)

    -- 5. 发现远程节点，通过 rexec 启动 pipe_6
    local peer_id = find_remote_peer()
    caps.print("pipe_5: 远程节点 " .. peer_id)

    local exec_payload = 'EXEC|pipe_6|{"model":"' .. remote_model ..
        '","inference_id":"' .. session_id .. '"}'
    caps.print("pipe_5: rexec → " .. exec_payload)
    local resp = caps.network.send_data(peer_id, "Command", exec_payload)
    caps.print("pipe_5: pipe_6 启动响应: " .. (resp.payload or "nil"))

    -- 6. 建立双向网络张量流
    local sid_num = tonumber(session_id)
    caps.print("pipe_5: 建立网络张量流 (inference_id=" .. session_id .. ") ...")
    local fwd = caps.network.open_tensor_stream(peer_id, sid_num)
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print("pipe_5: 网络张量流已建立")

    -- 7. 桥接循环: Session ↔ GPU:0 ↔ GPU:1 ↔ pipe_6
    caps.print("pipe_5: 桥接循环开始 (GPU:0→GPU:1→网络)")
    while true do
        -- 从 Session 收 tensor (放在 GPU:0)
        local tensor, offset = local_tensor.recv_tensor(session_stream, "cuda:0")
        if offset == 0 then
            gpu0_sess:reset_kv_cache()
            gpu1_sess:reset_kv_cache()
        end

        -- GPU:0 前向
        local hidden0 = gpu0_sess:forward(tensor, offset)

        -- 跨 GPU 搬运: GPU:0 → GPU:1
        local hidden1 = hidden0:to_device("cuda:1")

        -- GPU:1 前向
        local hidden_out = gpu1_sess:forward(hidden1, offset)

        -- 发送到 pipe_6
        caps.network.send_tensor(fwd, hidden_out, offset)

        -- 接收 pipe_6 返回的 logits
        local logits, _ = caps.network.recv_tensor(bwd, "cuda:0")

        -- 回传 logits 给 Session
        local_tensor.send_tensor(session_stream, logits, offset)
    end
end
