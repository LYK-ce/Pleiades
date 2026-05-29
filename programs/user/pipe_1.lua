-- Presented by KeJi
-- Date ： 2026-05-29

COMMAND = "pipe_1"
DESCRIPTION = "分布式流水线前半段 — 加载前半模型，桥接 Session ↔ pipe_2"

-- 用法: session inference pipe_1 <session_id> <model_path>
--   model_path: 前半模型文件名 (如 Qwen14B_split_0_20.pgguf)
--   pipe_1 自动通过 PGGUF split 元数据推导远程模型名，并 rexec 启动 pipe_2

-- 辅助函数：获取第一个远程节点 peer_id
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("pipe_1: 没有找到远程节点")
end

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    -- 1. 通过 Storage 获取前半模型路径 + 分析元数据
    caps.print("pipe_1: 读取模型 " .. model_path .. " ...")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    if not info.is_split then
        caps.print("pipe_1: 错误 — 模型不是 split PGGUF")
        handle:release()
        return
    end

    -- 计算远程模型名: Qwen14B_split_0_20.pgguf → Qwen14B_split_21_41.pgguf
    local stem = string.match(model_path, "^(.-)_split_")
    local total_layers = info.num_layers + 2
    local remote_start = info.split_end + 1
    local remote_end = total_layers - 1
    local remote_model = stem .. "_split_" .. remote_start .. "_" .. remote_end .. ".pgguf"

    caps.print(string.format("pipe_1: split_range=[%d,%d] total=%d remote=%s",
        info.split_start, info.split_end, total_layers, remote_model))

    -- 2. 加载前半模型 (embedding + transformer blocks)
    caps.print("pipe_1: 加载 " .. full_path .. " ...")
    local sess = ml.new("cuda")
    sess:load_model(full_path, info.split_start, info.split_end)
    handle:release()
    caps.print("pipe_1: 模型加载完成")

    -- 3. 发现远程节点，通过 rexec 启动 pipe_2
    local peer_id = find_remote_peer()
    caps.print("pipe_1: 远程节点 " .. peer_id)

    local exec_payload = 'EXEC|pipe_2|{"model":"' .. remote_model ..
        '","inference_id":"' .. session_id .. '"}'
    caps.print("pipe_1: rexec → " .. exec_payload)
    local resp = caps.network.send_data(peer_id, "Command", exec_payload)
    caps.print("pipe_1: pipe_2 启动响应: " .. (resp.payload or "nil"))

    -- 4. 连接 Session (local_tensor)
    local stream_id = "ml-" .. session_id
    caps.print("pipe_1: 连接 Session (stream: " .. stream_id .. ") ...")
    local session_stream = local_tensor.open_stream(stream_id)

    -- 5. 建立双向 tensor stream (inference_id = session_id)
    local sid_num = tonumber(session_id)
    caps.print("pipe_1: 建立网络张量流 (inference_id=" .. session_id .. ") ...")
    local fwd = caps.network.open_tensor_stream(peer_id, sid_num)
    local bwd = caps.network.accept_tensor_stream(sid_num, 300)
    caps.print("pipe_1: 网络张量流已建立")

    -- 6. 桥接循环: Session ↔ pipe_1 ↔ pipe_2
    caps.print("pipe_1: 桥接循环开始")
    while true do
        -- 从 Session 收 tensor（prefill 或单 token）
        local tensor, offset = local_tensor.recv_tensor(session_stream, "cuda")
        if offset == 0 then
            sess:reset_kv_cache()
        end

        -- 前向计算前半段
        local hidden = sess:forward(tensor, offset)

        -- 发送 hidden 到 pipe_2
        caps.network.send_tensor(fwd, hidden, offset)

        -- 接收 pipe_2 返回的 logits
        local logits, logits_offset = caps.network.recv_tensor(bwd, "cuda")

        -- 回传 logits 给 Session
        local_tensor.send_tensor(session_stream, logits, offset)
    end
end
