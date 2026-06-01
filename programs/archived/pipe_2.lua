-- Presented by KeJi
-- Date ： 2026-05-29

COMMAND = "pipe_2"
DESCRIPTION = "分布式流水线后半段 — 加载后半模型，接收 hidden 返回 logits"

-- 由 pipe_1 通过 rexec 远程启动: EXEC|pipe_2|{"model":"Qwen14B_split_21_41.pgguf","inference_id":"1"}

-- 辅助函数：获取第一个远程节点 peer_id (即 pipe_1)
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    error("pipe_2: 没有找到远程节点")
end

function execute(params)
    local model_path = params.model
    local inference_id = tonumber(params.inference_id)

    -- 1. 通过 Storage 获取模型路径
    caps.print("pipe_2: 读取模型 " .. model_path .. " ...")
    local handle = caps.storage_acquire_read(model_path)
    local full_path = handle:path()
    local info = ml.analyze_model(full_path)

    -- 2. 加载后半模型 (transformer blocks + LM head)
    caps.print("pipe_2: 加载 " .. full_path .. " ...")
    local sess = ml.new("cuda")
    sess:load_model(full_path, info.split_start, info.split_end)
    handle:release()
    caps.print("pipe_2: 模型加载完成 (layers " .. info.split_start .. "-" .. info.split_end .. ")")

    -- 3. 发现 pipe_1 节点
    local peer_id = find_remote_peer()
    caps.print("pipe_2: 连接 pipe_1 (" .. peer_id .. ")")

    -- 4. 建立双向 tensor stream
    caps.print("pipe_2: 建立网络张量流 (inference_id=" .. inference_id .. ") ...")
    local fwd = caps.network.accept_tensor_stream(inference_id, 300)
    local bwd = caps.network.open_tensor_stream(peer_id, inference_id)
    caps.print("pipe_2: 网络张量流已建立")

    -- 5. 循环: 收 hidden → forward → 发 logits
    caps.print("pipe_2: 推理循环开始")
    while true do
        local ok, hidden, offset = pcall(function()
            return caps.network.recv_tensor(fwd, "cuda")
        end)
        if not ok then
            caps.print("pipe_2: 流结束，退出")
            break
        end

        if offset == 0 then
            sess:reset_kv_cache()
        end

        local logits = sess:forward(hidden, offset)
        caps.network.send_tensor(bwd, logits, sess:get_offset())
    end

    sess:unload()
    caps.print("pipe_2: 完成")
end
