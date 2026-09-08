-- programs/user/yolo_test_uav.lua
-- Presented by KeJi
-- Created Date ： 2026-09-08
-- UAV 端：抓一帧 JPEG → 包 U8 tensor → tensor stream 发给 UGV（单帧测试）
--
-- 前置：UAV config 里 [camera] enabled=true

COMMAND = "yolo_test_uav"
DESCRIPTION = "抓图 → U8 tensor → tensor stream 发 UGV（单帧）"

-- ==================== 配置（直接改这里）====================
local TASK_ID    = 1001    -- tensor stream 配对 id，两端要一致
local DEVICE     = "cpu"   -- U8 tensor 所在设备（图不参与计算，cpu 即可）
local PEER_NAME  = "ugv"   -- 目标 UGV 的 peer_name；留 "" 则自动找第一个远程节点
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

-- 兜底：取第一个非本机的 peer_id
local function find_remote_peer()
    local my_id = caps.network.get_local_peer_id()
    local peers = caps.network.get_all_peers()
    for _, p in ipairs(peers) do
        if p.peer_id ~= my_id then
            return p.peer_id
        end
    end
    return nil
end

function execute(params)
    local peer
    if PEER_NAME ~= "" then
        peer = find_peer_by_name(PEER_NAME)
        if not peer then
            caps.print("[uav] 未找到名为 '" .. PEER_NAME .. "' 的节点（确认已连接，或改脚本顶部 PEER_NAME）")
            return "no-peer"
        end
    else
        peer = find_remote_peer()
        if not peer then
            caps.print("[uav] 未发现远程节点（确认 UGV 已连接）")
            return "no-peer"
        end
    end
    caps.print("[uav] 目标: " .. peer .. " (task_id=" .. TASK_ID .. ")")

    caps.print("[uav] 抓图 ...")
    local jpeg = camera.capture()
    caps.print("[uav] 抓到 " .. #jpeg .. " 字节")

    local img_t = ml.tensor_from_u8_bytes(jpeg, DEVICE)
    caps.print("[uav] 已包成 U8 tensor")

    local s = caps.network.open_tensor_stream(peer, TASK_ID)
    caps.print("[uav] 发送第 1 帧 ...")
    caps.network.send_tensor(s, img_t, 1)
    caps.network.send_eof(s)
    caps.print("[uav] 已发送，结束")
    return "ok"
end
