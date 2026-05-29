-- Presented by KeJi
-- Date ： 2026-05-29

COMMAND = "split"
DESCRIPTION = "将 PGGUF 文件按层均分为 num 份，第一份保留 tokenizer"

-- 用法: exec split xxx.pgguf num
--   path: PGGUF 文件路径
--   num:  均分份数

-- 层编号:
--   Layer 0          = Embedding
--   Layer 1 .. N     = Transformer blocks
--   Layer N+1        = LM Head
--   total = num_layers + 2

function execute(params)
    local path = params.path
    local num = tonumber(params.num)

    if not path or not num or num < 1 then
        caps.print("split: 用法: exec split <path> <num>")
        return
    end

    -- 1. 通过 Storage 获取模型文件路径
    caps.print("split: 读取模型 " .. path .. " ...")
    local handle = caps.storage_acquire_read(path)
    local full_path = handle:path()

    -- 2. 读取模型架构信息
    caps.print("split: 分析模型 " .. full_path .. " ...")
    local info = ml.analyze_model(full_path)
    handle:release()
    local total_layers = info.num_layers + 2   -- N blocks + embedding + LM head

    caps.print(string.format("split: %s, %d blocks → %d total layers, 分成 %d 份",
        info.architecture, info.num_layers, total_layers, num))

    if num > total_layers then
        caps.print(string.format("split: 错误 — 份数 %d 超过总层数 %d", num, total_layers))
        return
    end

    -- 3. 计算均分范围（余数分配给前几份，每份多1层）
    local base = math.floor(total_layers / num)
    local remainder = total_layers % num

    local start = 0
    for i = 0, num - 1 do
        local group_size = base
        if i < remainder then
            group_size = group_size + 1
        end
        local end_idx = start + group_size - 1

        local keep_tok = (i == 0)
        caps.print(string.format("split: part %d/%d  layers %d-%d  (tokenizer=%s)",
            i + 1, num, start, end_idx, tostring(keep_tok)))

        ml.split_model(full_path, start, end_idx, ".", keep_tok)

        start = end_idx + 1
    end

    caps.print("split: 完成 — 共输出 " .. num .. " 个文件")
end
