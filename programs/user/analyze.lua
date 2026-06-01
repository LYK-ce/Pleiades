-- Presented by KeJi
-- Date ： 2026-06-01

COMMAND = "analyze"
DESCRIPTION = "分析 PGGUF/GGUF 模型文件，打印架构元数据"

-- 用法: exec analyze model_path=xxx.pgguf

function execute(params)
    local model_path = params.model_path

    -- 通过 Storage 获取文件路径
    local handle = caps.storage_acquire_read(model_path)
    local path = handle:path()

    caps.print("===== 模型分析: " .. model_path .. " =====")
    caps.print("完整路径: " .. path)

    -- 分析模型
    local info = ml.analyze_model(path)
    handle:release()

    -- 打印所有字段
    local fields = {
        {"architecture",     "架构"},
        {"num_layers",       "总层数 (transformer blocks)"},
        {"embedding_length", "隐藏维度"},
        {"head_count",       "注意力头数"},
        {"head_count_kv",    "KV 头数"},
        {"head_dim",         "每头维度"},
        {"feed_forward_length", "FFN 中间维度"},
        {"context_length",   "上下文长度"},
        {"rms_norm_eps",     "RMS Norm epsilon"},
        {"rope_freq_base",   "RoPE 频率基数"},
        {"vocab_size",       "词表大小"},
        {"eos_token_id",     "EOS token ID"},
        {"is_split",         "是否切分模型"},
        {"split_start",      "切分起始层"},
        {"split_end",        "切分结束层"},
    }

    for _, f in ipairs(fields) do
        local key = f[1]
        local label = f[2]
        local val = info[key]
        if val ~= nil then
            caps.print(string.format("  %-25s = %s", label, tostring(val)))
        end
    end

    -- 额外信息
    local total_layers = info.num_layers + 2
    caps.print(string.format("  %-25s = %d (embedding + %d blocks + output)",
        "总层范围", total_layers, info.num_layers))

    caps.print("===== 分析完成 =====")

    -- 打印原始 metadata（GGUF 所有 key-value）
    if info.metadata_raw then
        caps.print("\n--- GGUF Metadata Raw ---")
        local keys = {}
        for k, _ in pairs(info.metadata_raw) do
            table.insert(keys, k)
        end
        table.sort(keys)
        for _, k in ipairs(keys) do
            caps.print(string.format("  %s = %s", k, info.metadata_raw[k]))
        end
    end

    caps.print("===== 分析完成 =====")
end
