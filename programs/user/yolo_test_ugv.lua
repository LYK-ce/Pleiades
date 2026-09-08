-- programs/user/yolo_test_ugv.lua
-- Presented by KeJi
-- Created Date ： 2026-09-08
-- UGV 端：accept 流 → 收图剥字节 → YOLO detect → 打印 bbox
--
-- 前置：UGV 的 Pleiades_Workspace/ 放好模型权重

COMMAND = "yolo_test_ugv"
DESCRIPTION = "accept 流 → 收图 → YOLO detect → 打印 bbox"

-- ==================== 配置（直接改这里）====================
local TASK_ID = 1001                       -- tensor stream 配对 id，两端要一致
local MODEL   = "yolov8n.safetensors"      -- 模型权重文件名（放在 Pleiades_Workspace/）
local CONF    = 0.25                       -- 置信度阈值
local NMS     = 0.45                       -- NMS 阈值
local DEVICE  = "cpu"                      -- YOLO 推理设备（cpu / cuda:0）
-- ==========================================================

function execute(params)
    caps.print("[ugv] 等待 UAV 图流 (task_id=" .. TASK_ID .. ", 60s) ...")
    local s = caps.network.accept_tensor_stream(TASK_ID, 60)

    caps.print("[ugv] 加载模型 " .. MODEL .. " (" .. DEVICE .. ") ...")
    local det = ml.yolo_new(MODEL, DEVICE)

    caps.print("[ugv] 开始收帧 ...")
    while true do
        local ok, img_t, frame = pcall(function()
            return caps.network.recv_tensor(s, "cpu")
        end)
        if not ok then
            caps.print("[ugv] 流结束（EOF），退出")
            break
        end

        local jpeg = ml.tensor_to_u8_bytes(img_t)
        caps.print("[ugv] 收到第 " .. frame .. " 帧，" .. #jpeg .. " 字节")

        local dets = det:detect(jpeg, CONF, NMS)
        caps.print("[ugv] 检测到 " .. #dets .. " 个目标")
        for _, d in ipairs(dets) do
            caps.print(string.format("  %-12s conf=%.4f  [%.0f, %.0f, %.0f, %.0f]",
                d.class_name, d.confidence, d.xmin, d.ymin, d.xmax, d.ymax))
        end
    end
    caps.print("[ugv] 完成")
    return "ok"
end
