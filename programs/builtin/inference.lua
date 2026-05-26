-- Presented by KeJi
-- Date ： 2026-05-25

COMMAND = "inference"
DESCRIPTION = "启动 ML Thread，连接到指定 Session 并执行 forward"

-- 内置命令: session inference <session_id> <model_path>
-- 由 Core 直接查找并调用，用户不可直接 exec

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    -- 1. 通过 Storage 获取模型文件路径
    caps.print("inference: 读取模型 " .. model_path .. " ...")
    local handle = caps.storage_acquire_read(model_path)
    local path = handle:path()

    -- 2. 加载模型权重
    caps.print("inference: 加载模型 " .. path .. " ...")
    local sess = ml.new("cuda")
    sess:load_model(path, 0, 999999)
    handle:release()

    -- 3. 连接 Session
    local stream_id = "ml-" .. session_id
    caps.print("inference: 连接 Session (stream: " .. stream_id .. ") ...")
    local stream = local_tensor.open_stream(stream_id)

    caps.print("inference: ML Thread 就绪，等待 tensor ...")

    -- 4. forward loop
    while true do
        local tensor, offset = local_tensor.recv_tensor(stream, "cuda")
        -- offset = u64::MAX - 1: 清理 KV Cache 控制帧
        if offset == 18446744073709551614 then
            sess:reset_kv_cache()
            goto continue
        end
        local logits = sess:forward(tensor, offset)
        local_tensor.send_tensor(stream, logits, offset)
        ::continue::
    end
end
