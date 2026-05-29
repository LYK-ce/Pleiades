-- Presented by KeJi
-- Date ： 2026-05-29

COMMAND = "split"
DESCRIPTION = "将 PGGUF 文件按层均分为 num 份，第一份保留 tokenizer"

-- 用法: exec split path=xxx.pgguf num=N
--   path: 模型文件名（Storage 扁平命名空间）
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
        caps.print("split: 用法: exec split path=<xxx.pgguf> num=<N>")
        return
    end

    -- 1. 通过 Storage 获取模型文件路径
    caps.print("split: 读取模型 " .. path .. " ...")
    local read_handle = caps.storage_acquire_read(path)
    local full_path = read_handle:path()

    -- 提取文件名 stem（如 Qwen14B）
    local stem = string.match(full_path, "([^/\\]+)%.[^.]+$")
    if not stem then
        caps.print("split: 错误 — 无法解析文件名: " .. full_path)
        read_handle:release()
        return
    end

    -- 2. 读取模型架构信息
    caps.print("split: 分析模型 " .. full_path .. " ...")
    local info = ml.analyze_model(full_path)
    read_handle:release()
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

        -- 通过 Storage 注册输出文件，获取写入目录
        local part_name = stem .. "_split_" .. start .. "_" .. end_idx .. ".pgguf"
        local write_handle = caps.storage_acquire_write(part_name)
        local write_path = write_handle:path()
        local write_dir = string.match(write_path, "^(.*)[/\\]")

        caps.print(string.format("split: part %d/%d  layers %d-%d → %s  (tokenizer=%s)",
            i + 1, num, start, end_idx, part_name, tostring(keep_tok)))

        ml.split_model(full_path, start, end_idx, write_dir, keep_tok)

        write_handle:release()

        start = end_idx + 1
    end

    -- 4. 刷新 Storage 索引，扫描新生成的 split 文件
    caps.storage_flush()
    caps.print("split: 完成 — 共输出 " .. num .. " 个文件")
end
