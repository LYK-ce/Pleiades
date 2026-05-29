-- programs/pipeline.lua
-- Presented by KeJi
-- Date: 2026-05-17
-- 分布式流水线推理脚本
COMMAND = "pipeline"
DESCRIPTION = "分布式流水线推理 (可选 strategy: uniform/weighted)"

function execute(params, caps)
    -- Phase 1: 模型分析
    local arch = caps.analyze_model(params.model)

    -- Phase 2: 节点发现 + 调度
    local peers = caps.get_available_peers()
    local plan
    if params.strategy == "weighted" then
        plan = caps.plan_weighted(arch, peers)
    else
        plan = caps.plan_uniform(arch, peers)
    end

    -- Phase 3: 分发模型分片
    for _, w in ipairs(plan.workers) do
        local shard = caps.split_model(params.model, w.layer_start, w.layer_end)
        caps.send_file(w.peer_id, shard)
    end

    -- Phase 4: 建立张量流
    caps.establish_streams(plan)

    -- Phase 5: Coordinator 推理
    local sess = caps.create_session(params.model, params.device,
        plan.coord_layer_start, plan.coord_layer_end)
    local io = caps.allocate_io()

    -- 推理循环
    local prompt = io.input()
    local tokens = sess:encode(prompt)
    sess:forward(sess:tensorize(tokens), 0)

    for i = 1, 120 do
        local hidden = sess:get_output_tensor()
        caps.send_tensor(hidden, plan.workers[1].peer_id)

        local result = caps.receive_tensor()
        sess:forward(result, nil)

        local tok = sess:sample(0.8)
        io.output(sess:decode(tok))
        if tok == sess:get_eos() then break end

        sess:forward(sess:tensorize({tok}), nil)
    end

    io.end_output()
    caps.send_eof(plan)
    sess:unload()
end
