-- programs/user/camera_photo.lua
-- Presented by KeJi
-- Created Date ： 2026-10-24
-- Modified Date ： 2026-10-24
--
-- 拍照脚本（Task 34 Step 2）：依次探测 camera_uav → camera_sim → camera_pc，
-- 打印用的是哪个 sensor、info() 的四个字段、JPEG 字节数，再把 JPEG 写进
-- Pleiades_Workspace/camera_photo.jpg。
--
-- info() 的 age_ms 语义（三处 sensor 一致）：距最近一次成功抓帧的毫秒数，单调不减；
-- 从未抓到帧时为 0，且此时 format 通常是 "none"（或打开时协商到的格式）。
-- 脚本刻意在 capture() 之前打印 info：反映的是"上一帧"，落盘的这张图由下面这次 capture 刷新。
--
-- 刻意不做的事：
-- 1) 不提供统一的 `camera` 别名（Task 34 §二 Step 2 硬要点）：脚本自己按名字挑，
--    避免两个都开时"到底哪个生效"的隐式歧义；
-- 2) 不改动任何既有脚本（test_camera.lua / yolo_*.lua 一律不动）；
-- 3) 不把 capture() 包进 pcall：设备侧抛的错自带排查提示，交给 execute 调用方
--    记成"执行失败: ..."比在这里吞掉更好诊断。

COMMAND = "camera_photo"
DESCRIPTION = "拍照：camera_uav → camera_sim → camera_pc 依次探测，打印 info 并把 camera_photo.jpg 存到 workspace"

-- 探测顺序（优先级从高到低）
local SENSOR_ORDER = { "camera_uav", "camera_sim", "camera_pc" }
-- 落盘文件名（storage 决定实际目录，通常是 Pleiades_Workspace/）
local OUTPUT_FILE = "camera_photo.jpg"

-- 依次探测：返回第一个存在的 sensor 名字与其 Lua 表；都没有则 (nil, nil)
local function Pick_Sensor()
    for _, name in ipairs(SENSOR_ORDER) do
        local sensor = _G[name]
        if sensor ~= nil then
            return name, sensor
        end
    end
    return nil, nil
end

-- info() 的四个字段 → 一行文本（字段名与 Rust 侧 / 仿真侧逐字一致）
local function Format_Info(info)
    return string.format(
        "width=%s height=%s format=%s age_ms=%s",
        tostring(info.width), tostring(info.height),
        tostring(info.format), tostring(info.age_ms)
    )
end

function execute(params)
    caps.print("[camera_photo] 探测相机 sensor（顺序：camera_uav → camera_sim → camera_pc）")

    local name, sensor = Pick_Sensor()
    if sensor == nil then
        caps.print("[camera_photo] 错误：没有任何可用相机 sensor（camera_uav / camera_sim / camera_pc 都不存在）")
        caps.print("[camera_photo]   真机：看 .config/config.toml 的 [camera_pc] / [camera_uav] 与启动日志里的打开结果")
        caps.print("[camera_photo]   仿真：确认 [[sim_world.sensor]] 里声明了 sensor=\"camera\"，且日志出现 [world] assembled")
        return "no-camera"
    end
    caps.print("[camera_photo] sensor=" .. name)

    -- info() 在抓帧之前打印：它反映"上一帧"（首拍时是打开时协商的格式，
    -- age_ms = 0 表示还没有帧）；落盘的这张图由下面的 capture() 刷新。
    local info = sensor.info()
    if info ~= nil then
        caps.print("[camera_photo] info: " .. Format_Info(info))
    else
        caps.print("[camera_photo] 警告：" .. name .. ".info() 返回 nil，跳过 info 打印")
    end

    local jpeg = sensor.capture()
    if jpeg == nil then
        caps.print("[camera_photo] 错误：" .. name .. ".capture() 返回 nil（没有取到帧）")
        caps.print("[camera_photo]   真机：看启动日志里该 sensor 是否打开成功；抓帧超时 / 设备被占用都会导致无帧")
        caps.print("[camera_photo]   仿真：等世界侧推过一帧（rate_hz），或确认世界相机的 warning 是否表示出不了图")
        return "no-jpeg"
    end
    if type(jpeg) ~= "string" then
        caps.print("[camera_photo] 错误：" .. name .. ".capture() 返回 " .. type(jpeg) .. "，期望 JPEG 字符串")
        return "bad-jpeg"
    end
    if #jpeg == 0 then
        caps.print("[camera_photo] 错误：" .. name .. ".capture() 返回 0 字节（空 JPEG）")
        return "empty-jpeg"
    end
    caps.print("[camera_photo] jpeg 字节数=" .. #jpeg)

    local handle = caps.storage_acquire_write(OUTPUT_FILE)
    handle:write(jpeg)
    local path = handle:path()
    handle:release()
    caps.print("[camera_photo] 已保存 " .. tostring(path) .. "（" .. #jpeg .. " 字节）")

    return "ok"
end
