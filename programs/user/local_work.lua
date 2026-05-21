-- programs/user/local_work.lua
-- Presented by KeJi
-- Date: 2026-05-21
-- 本地流工作端: 后半 transformer + output, 产出 logits
-- 配合 local_coord.lua 使用: 先 exec local_work, 再 exec local_coord

COMMAND = "local_work"
DESCRIPTION = "本地流工作端 (recv hidden→后半 forward→send logits)"

function execute(params)
    local model = params.model or "test.pgguf"
    local device = params.device or "cpu"
    local timeout = tonumber(params.timeout) or 120000

    -- 0. 获取模型路径
    local raw_path
    do
        local handle = caps.storage_acquire_read(model)
        raw_path = handle:path()
        handle:release()
    end
    caps.print("[work] 模型: " .. raw_path)

    -- 1. 分析模型
    local info = ml.analyze_model(raw_path)
    local N = info.num_layers
    local total = N + 1
    local mid = math.floor(N / 2)
    caps.print("[work] 总层: " .. total .. ", 分段: [0.." .. mid .. "] [" .. (mid+1) .. ".." .. total .. "]")

    -- 2. 加载后半模型
    local sess = ml.new(device)
    caps.print("[work] 加载 " .. (mid+1) .. ".." .. total)
    sess:load_model(raw_path, mid + 1, total)
    caps.print("[work] 模型加载完成")

    -- 3. 建立本地流: work 等待 fwd 入站，打开 bwd 出站
    caps.print("[work] 等待 fwd 流 (timeout=" .. timeout .. "ms)...")
    local fwd = local_tensor.accept_stream("fwd", timeout)
    caps.print("[work] 打开 bwd 流...")
    local bwd = local_tensor.open_stream("bwd")
    caps.print("[work] 双流已建立")

    -- 4. 循环: 收 hidden → forward → 发 logits
    while true do
        local ok, result = pcall(function()
            return local_tensor.recv_tensor(fwd)
        end)
        if not ok then
            caps.print("[work] 流结束 (收到 EOF), 退出")
            break
        end

        caps.print("[work] 收到 hidden offset=" .. result.offset)

        local hidden = ml.tensor_from_bytes(result.data, device)
        local logits = sess:forward(hidden, result.offset)

        local_tensor.send_tensor(bwd, logits:to_bytes(), sess:get_offset())
    end

    sess:unload()
    caps.print("[work] 完成")
end
