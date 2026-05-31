-- Presented by KeJi
-- Date ： 2026-06-01

COMMAND = "offload_pingpong"
DESCRIPTION = "单机双 Session 轮流 offload 演示 — 两个 MlContext 交替在 GPU 推理，验证 KV Cache 保留"

-- 用法:
--   exec offload_pingpong
--   或通过 session inference 调用

-- 场景:
--   Session A: 问 "What is 1+1?" → 得到回答 → offload 到 CPU
--   Session B: 问 "What is 2+2?" → 得到回答 → offload 到 CPU
--   Session A: 恢复到 GPU，继续问 "Now what is 3+3?" → 验证 KV Cache 保留

function execute(params)
    local model_path = params.model_path

    -- 1. 通过 Storage 获取模型文件路径
    caps.print("===== Offload Ping-Pong 演示 =====")
    local handle = caps.storage_acquire_read(model_path)
    local path = handle:path()
    caps.print("模型路径: " .. path)

    -- 2. 加载 tokenizer（两个 session 共用同一个 tokenizer 信息，但独立加载）
    local arch_info = ml.analyze_model(path)
    caps.print("架构: " .. arch_info.architecture .. ", 层数: " .. tostring(arch_info.num_layers))

    -- ================================================================
    -- Round 1: Session A — 第一轮推理
    -- ================================================================
    caps.print("\n--- Round 1: Session A 加载 & 推理 ---")

    local sess_A = ml.new("cuda")
    sess_A:load_model(path, 0, 999999)
    sess_A:load_tokenizer(path)
    caps.print("Session A: 模型已加载到 GPU")

    -- Prefill
    local prompt_A1 = "<|im_start|>user\nWhat is 1+1?\n<|im_end|>\n<|im_start|>assistant\n"
    local tokens_A1 = sess_A:encode(prompt_A1)
    caps.print("Session A: 编码完成, token 数: " .. tostring(#tokens_A1))

    local tensor_A1 = sess_A:tensorize(tokens_A1)
    local logits_A1 = sess_A:forward(tensor_A1, 0)

    -- Decode a few tokens
    local eos = sess_A:get_eos()
    local response_A1 = ""
    for i = 1, 5 do
        local tok = sess_A:sample(logits_A1, 0.0)  -- greedy
        if tok == eos then break end
        local text = sess_A:decode(tok)
        response_A1 = response_A1 .. text
        local next_t = sess_A:tensorize({tok})
        logits_A1 = sess_A:forward(next_t)
    end
    caps.print("Session A 回答: " .. response_A1)

    -- Offload A → CPU
    caps.print("Session A: offload to CPU ...")
    sess_A:offload_to_cpu()
    caps.print("Session A: GPU 显存已释放, KV Cache 保留在 CPU")

    -- ================================================================
    -- Round 2: Session B — 第一轮推理
    -- ================================================================
    caps.print("\n--- Round 2: Session B 加载 & 推理 ---")

    local sess_B = ml.new("cuda")
    sess_B:load_model(path, 0, 999999)
    sess_B:load_tokenizer(path)
    caps.print("Session B: 模型已加载到 GPU")

    -- Offload B 后立即测试 save
    local prompt_B1 = "<|im_start|>user\nWhat is 2+2?\n<|im_end|>\n<|im_start|>assistant\n"
    local tokens_B1 = sess_B:encode(prompt_B1)
    caps.print("Session B: 编码完成, token 数: " .. tostring(#tokens_B1))

    local tensor_B1 = sess_B:tensorize(tokens_B1)
    local logits_B1 = sess_B:forward(tensor_B1, 0)

    local response_B1 = ""
    for i = 1, 5 do
        local tok = sess_B:sample(logits_B1, 0.0)
        if tok == eos then break end
        local text = sess_B:decode(tok)
        response_B1 = response_B1 .. text
        local next_t = sess_B:tensorize({tok})
        logits_B1 = sess_B:forward(next_t)
    end
    caps.print("Session B 回答: " .. response_B1)

    -- Offload B → CPU
    caps.print("Session B: offload to CPU ...")
    sess_B:offload_to_cpu()
    caps.print("Session B: GPU 显存已释放, KV Cache 保留在 CPU")

    -- ================================================================
    -- Round 3: Session A — 恢复并继续推理（验证 KV Cache 保留）
    -- ================================================================
    caps.print("\n--- Round 3: Session A 恢复, 验证 KV Cache ---")

    sess_A:offload_to_cuda()
    caps.print("Session A: 已恢复到 GPU")

    -- 继续对话（KV Cache 保留了之前的上下文）
    local prompt_A2 = "<|im_start|>user\nNow what is 3+3?\n<|im_end|>\n<|im_start|>assistant\n"
    local tokens_A2 = sess_A:encode(prompt_A2)
    local tensor_A2 = sess_A:tensorize(tokens_A2)
    local logits_A2 = sess_A:forward(tensor_A2)  -- offset 自动继续

    local response_A2 = ""
    for i = 1, 10 do
        local tok = sess_A:sample(logits_A2, 0.0)
        if tok == eos then break end
        local text = sess_A:decode(tok)
        response_A2 = response_A2 .. text
        local next_t = sess_A:tensorize({tok})
        logits_A2 = sess_A:forward(next_t)
    end
    caps.print("Session A 回答 (基于之前上下文): " .. response_A2)

    -- Offload A → CPU, then save to disk
    caps.print("Session A: offload to CPU ...")
    sess_A:offload_to_cpu()
    caps.print("Session A: offload_save 到磁盘 ...")
    sess_A:offload_save("pingpong_A")

    -- ================================================================
    -- Round 4: Session B — 恢复并继续
    -- ================================================================
    caps.print("\n--- Round 4: Session B 恢复 ---")

    sess_B:offload_to_cuda()
    caps.print("Session B: 已恢复到 GPU")

    -- 继续 B 的对话
    local prompt_B2 = "<|im_start|>user\nNow what is 4+4?\n<|im_end|>\n<|im_start|>assistant\n"
    local tokens_B2 = sess_B:encode(prompt_B2)
    local tensor_B2 = sess_B:tensorize(tokens_B2)
    local logits_B2 = sess_B:forward(tensor_B2)

    local response_B2 = ""
    for i = 1, 10 do
        local tok = sess_B:sample(logits_B2, 0.0)
        if tok == eos then break end
        local text = sess_B:decode(tok)
        response_B2 = response_B2 .. text
        local next_t = sess_B:tensorize({tok})
        logits_B2 = sess_B:forward(next_t)
    end
    caps.print("Session B 回答: " .. response_B2)

    -- Save B to disk too
    caps.print("Session B: offload_save 到磁盘 ...")
    sess_B:offload_save("pingpong_B")

    -- ================================================================
    -- Round 5: 从磁盘恢复 Session A（验证 save/load）
    -- ================================================================
    caps.print("\n--- Round 5: 从磁盘恢复 Session A（验证 offload_load）---")

    sess_A = nil  -- 释放旧引用
    local sess_A2 = ml.offload_load("pingpong_A", "cuda")
    caps.print("Session A: 从磁盘恢复成功")

    -- 继续 A 的对话
    local prompt_A3 = "<|im_start|>user\nWhat is 5+5?\n<|im_end|>\n<|im_start|>assistant\n"
    local tokens_A3 = sess_A2:encode(prompt_A3)
    local tensor_A3 = sess_A2:tensorize(tokens_A3)
    local logits_A3 = sess_A2:forward(tensor_A3)

    local response_A3 = ""
    for i = 1, 10 do
        local tok = sess_A2:sample(logits_A3, 0.0)
        if tok == eos then break end
        local text = sess_A2:decode(tok)
        response_A3 = response_A3 .. text
        local next_t = sess_A2:tensorize({tok})
        logits_A3 = sess_A2:forward(next_t)
    end
    caps.print("Session A 回答 (磁盘恢复后): " .. response_A3)

    -- 清理
    sess_A2:unload()
    sess_B:unload()

    handle:release()
    caps.print("\n===== Offload Ping-Pong 演示完成 =====")
end
