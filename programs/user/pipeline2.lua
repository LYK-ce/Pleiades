-- programs/user/pipeline2.lua
-- Presented by KeJi
-- Date: 2026-05-19
-- 流水线并行 — 工作者 (后半 transformer + output)

COMMAND = "pipeline2"
DESCRIPTION = "流水线并行后半段: transformer[mid..N] + output, 产出 logits"

function execute(params)
    local model = params.model or "test.pgguf"
    local device = params.device or "cpu"
    local peer = params.peer

    if not peer then
        caps.print("[pipeline2] 缺少 peer 参数")
        return
    end

    -- 0. 通过 Storage 获取模型路径
    local handle = caps.storage_acquire_read(model)
    local raw_path = handle:path()
    caps.print("[pipeline2] 模型文件: " .. raw_path)

    -- 1. 分析模型
    caps.print("[pipeline2] 分析模型...")
    local info = ml.analyze_model(raw_path)
    local N = info.num_layers
    local total = N + 1
    local mid = math.floor(N / 2)
    caps.print("  总层: " .. total .. " (embedding=0, blocks=1.." .. N .. ", output=" .. total .. ")")
    caps.print("  分段: A[0.." .. mid .. "]  B[" .. (mid+1) .. ".." .. total .. "]")

    -- 2. 确定 .pgguf 路径
    local pgguf_path = raw_path
    if raw_path:match("%.gguf$") then
        pgguf_path = raw_path:gsub("%.gguf$", ".pgguf")
    end

    -- 3. 创建会话，加载后半模型
    local sess = ml.new(device)
    caps.print("[pipeline2] 加载 " .. (mid+1) .. ".." .. total)
    sess:load_model(pgguf_path, mid + 1, total)
    caps.print("[pipeline2] 模型加载完成")

    -- 4. 建立双流
    caps.print("[pipeline2] 建立连接...")
    local stream1 = caps.network.accept_tensor_stream(42, 120)
    caps.print("[pipeline2] stream1 已建立")
    local stream2 = caps.network.open_tensor_stream(peer, 42)
    caps.print("[pipeline2] stream2 已打开")

    -- 5. 循环: 收 hidden → forward → 发 logits
    while true do
        local ok, hidden, offset_a = pcall(function()
            return caps.network.recv_tensor(stream1, device)
        end)
        if not ok then
            caps.print("[pipeline2] 流结束, 退出")
            break
        end

        caps.print("[pipeline2] 收到 hidden dims: [" .. table.concat(hidden:dims(), ", ") .. "] offset=" .. offset_a)

        local logits = sess:forward(hidden, offset_a)
        caps.print("[pipeline2] 前向完成, logits dims: [" .. table.concat(logits:dims(), ", ") .. "]")

        caps.network.send_tensor(stream2, logits, sess:get_offset())
    end

    sess:unload()
    caps.print("[pipeline2] 完成")
end
