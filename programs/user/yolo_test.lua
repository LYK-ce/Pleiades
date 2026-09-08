-- programs/user/yolo_test.lua
-- Presented by KeJi
-- Created Date ： 2026-09-08
-- 测试 YOLO 目标检测：读本地图 → detect → 打印结果
--
-- 前置：把 bike.jpg 和 yolov8n.safetensors 放到 Pleiades_Workspace/

COMMAND = "yolo_test"
DESCRIPTION = "测试 YOLO：读 Pleiades_Workspace/bike.jpg → detect → 打印 bbox"

function execute(params)
    -- 1. 读图片字节（storage 读锁机制，handle:read() 返回二进制字符串）
    caps.print("[yolo] 读取图片 bike.jpg ...")
    local handle = caps.storage_acquire_read("bike.jpg")
    local jpeg = handle:read()
    handle:release()
    caps.print("[yolo] 读到图片 " .. #jpeg .. " 字节")

    -- 2. 加载检测器（型号/权重写死，cpu 设备）
    caps.print("[yolo] 加载 yolov8n ...")
    local det = ml.yolo_new("cpu")

    -- 3. 检测（conf=0.25, nms=0.45，与 rust_yolo 基准一致）
    caps.print("[yolo] 检测中 ...")
    local dets = det:detect(jpeg, 0.25, 0.45)
    caps.print("[yolo] 检测到 " .. #dets .. " 个目标")

    -- 4. 打印结果（类别 + 置信度 + 检测尺度坐标）
    for i, d in ipairs(dets) do
        caps.print(string.format(
            "%s  conf=%.4f  [%.2f, %.2f, %.2f, %.2f]",
            d.class_name, d.confidence, d.xmin, d.ymin, d.xmax, d.ymax))
    end

    return "ok"
end
