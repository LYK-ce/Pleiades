-- programs/user/yolo_uav.lua
-- Presented by KeJi
-- Created Date ： 2026-09-08
-- UAV 端：抓图 → 轮询发多辆 UGV（第 N 帧发第 N 辆车，循环直到 Ctrl-C）
--
-- 用法：exec yolo_uav cars=ugv1,ugv2,ugv3
-- 前置：UAV config 里 [camera] enabled=true

COMMAND = "yolo_uav"
DESCRIPTION = "抓图 → 轮询发多辆 UGV（cars=ugv1,ugv2）"

-- ==================== 配置（直接改这里）====================
local TASK_ID   = 1001    -- tensor stream 配对 id（多车可复用同一个）
local DEVICE    = "cpu"   -- U8 tensor 所在设备（图不参与计算，cpu 即可）
local SLEEP_MS  = 500     -- 每帧发送间隔（毫秒）
-- ==========================================================

-- 按名字找 peer_id
local function find_peer_by_name(name)
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.name == name then
            return p.peer_id
        end
    end
    return nil
end

function execute(params)
    -- 解析 cars 参数（逗号分隔车名，忽略空项）
    local names = {}
    for name in (params.cars or ""):gmatch("[^,]+") do
        name = name:match("^%s*(.-)%s*$")  -- trim
        if name ~= "" then
            names[#names + 1] = name
        end
    end
    if #names == 0 then
        caps.print("[uav] 用法: exec yolo_uav cars=ugv1,ugv2,ugv3")
        return "usage"
    end

    -- 解析每辆车 peer_id + 开流
    local targets = {}
    for _, name in ipairs(names) do
        local peer = find_peer_by_name(name)
        if not peer then
            caps.print("[uav] 未找到 '" .. name .. "'，跳过")
        else
            local s = caps.network.open_tensor_stream(peer, TASK_ID)
            targets[#targets + 1] = { name = name, stream = s }
            caps.print("[uav] 目标 " .. name .. " (" .. peer .. ") 已开流")
        end
    end
    if #targets == 0 then
        caps.print("[uav] 没有可用的目标车")
        return "no-peer"
    end

    caps.print("[uav] 开始轮询发图（" .. #targets .. " 辆车，间隔 " .. SLEEP_MS .. "ms，Ctrl-C 停止）")
    local frame = 0
    while true do
        local idx = (frame % #targets) + 1
        local jpeg = camera.capture()
        local img_t = ml.tensor_from_u8_bytes(jpeg, DEVICE)
        caps.network.send_tensor(targets[idx].stream, img_t, frame)
        caps.print("[uav] 帧 " .. frame .. " → " .. targets[idx].name .. "（" .. #jpeg .. " 字节）")
        frame = frame + 1
        os.execute("sleep " .. (SLEEP_MS / 1000))
    end
end
