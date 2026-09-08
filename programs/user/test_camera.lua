-- programs/user/test_camera.lua
-- Presented by KeJi
-- Created Date ： 2026-09-08
-- 测试摄像头：抓一帧 → 存到 Pleiades_Workspace

COMMAND = "test_camera"
DESCRIPTION = "测试摄像头：拍一张图存到 workspace (test_camera.jpg)"

function execute(params)
    -- 1. 抓一帧（返回 JPEG 字节，二进制安全字符串）
    caps.print("[camera] 抓帧中...")
    local jpeg = camera.capture()
    caps.print("[camera] 抓到 " .. #jpeg .. " 字节")

    -- 2. 写入 workspace（走 storage 的写锁机制）
    local handle = caps.storage_acquire_write("test_camera.jpg")
    handle:write(jpeg)
    handle:release()
    caps.print("[camera] 已保存 test_camera.jpg")

    return "ok"
end
