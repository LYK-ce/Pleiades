-- Presented by KeJi
-- Date ： 2026-05-25

COMMAND = "inference"
DESCRIPTION = "启动 ML Thread，连接到指定 Session 并执行 forward"

-- 用法: exec inference session_id=<id> model_path=<path>
-- 示例: exec inference session_id=1 model_path=test.pgguf

function execute(params)
    local session_id = params.session_id
    local model_path = params.model_path

    if not session_id or not model_path then
        caps.print("inference: 缺少参数 session_id 或 model_path")
        return
    end

    -- 1. 通过 Storage 获取模型文件路径
    caps.print("inference: 读取模型 " .. model_path .. " ...")
    local handle = caps.storage_acquire_read(model_path)
    local path = handle:path()

    -- 2. 加载模型权重
    caps.print("inference: 加载模型 " .. path .. " ...")
    local sess = ml.new("cpu")
    sess:load_model(path, 0, 999999)
    handle:release()

    -- 3. 连接 Session
    local stream_id = "ml-" .. session_id
    caps.print("inference: 连接 Session (stream: " .. stream_id .. ") ...")
    local stream = local_tensor.open_stream(stream_id)

    caps.print("inference: ML Thread 就绪，等待 tensor ...")

    -- 4. forward loop
    while true do
        local tensor, offset = local_tensor.recv_tensor(stream, "cpu")
        local logits = sess:forward(tensor, offset)
        local_tensor.send_tensor(stream, logits, offset)
    end
end
