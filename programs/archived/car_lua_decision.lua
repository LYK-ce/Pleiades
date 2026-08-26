-- Presented by KeJi
-- Created Date ： 2026-08-20
-- Modified Date ： 2026-08-21

-- 车决策脚本（Task 22_3）：纯决策函数，零副作用
-- on_tick(next_cell) 收 main_loop 传入的「下一格」，读状态+感知，返回 DecisionResult 表
-- 不调 get_path、不调 action.*、不写状态（状态在 Rust ExecuteState，由 main_loop 单点写）

-- caps 接口（Rust register_robot_caps 注册，全只读）：
--   self.get_state()     -> { state = "Idle"/"Turning"/"Moving", sub_target = {gx, gy} 或 nil }
--   self.get_position()  -> (x, y, z)
--   self.get_attitude()  -> (roll, pitch, yaw)

-- 返回值（DecisionResult 表）：
--   { state = "Idle"/"Turning"/"Moving", sub_target = {gx,gy} 或 nil,
--     action = "move_forward"/"move_backward"/"turn_left"/"turn_right"/"stop"/"none", arg = 速度/速率 }
--   action = "none" 或缺省 = 保持、不发命令（命令去重）

local CELL_RESOLUTION = 0.5           -- 米/格
local SUB_TARGET_THRESHOLD_M = 0.2    -- 到达当前格判定（0.2m 距离门）
local TURN_ALIGN_RAD = math.rad(5.0)  -- 转向对齐阈值
local STRAIGHT_ALIGN_RAD = math.rad(10.0)  -- 直行连续化阈值
local TURN_SPEED = 10                 -- 转向速度
local MOVE_SPEED = 30                 -- 直行速度

local function normalize_angle(a)
    return (a + math.pi) % (2.0 * math.pi) - math.pi
end

local function cell_center(coord)
    return (coord + 0.5) * CELL_RESOLUTION
end

local function angle_to_target(gx, gy, x, y, yaw)
    local tx = cell_center(gx)
    local ty = cell_center(gy)
    return normalize_angle(math.atan(ty - y, tx - x) - yaw)
end

function on_tick(next_cell)   -- next_cell = {gx=, gy=} 或 nil
    local st = self.get_state()
    local x, y = self.get_position()
    local _, _, yaw = self.get_attitude()

    if st.state == "Idle" then
        if next_cell == nil then
            -- 无任务/已到达：车已停，不发 stop（去重）
            return { state = "Idle", sub_target = nil, action = "none" }
        end
        local delta = angle_to_target(next_cell.gx, next_cell.gy, x, y, yaw)
        if math.abs(delta) > TURN_ALIGN_RAD then
            return { state = "Turning", sub_target = next_cell,
                     action = (delta > 0) and "turn_right" or "turn_left", arg = TURN_SPEED }
        else
            return { state = "Moving", sub_target = next_cell, action = "move_forward", arg = MOVE_SPEED }
        end

    elseif st.state == "Turning" then
        if next_cell == nil then
            return { state = "Idle", sub_target = nil, action = "stop" }
        end
        if st.sub_target == nil
            or next_cell.gx ~= st.sub_target.gx
            or next_cell.gy ~= st.sub_target.gy then
            -- 目标变了（任务/路径变化）：停车回 Idle，下 tick 重新决策
            return { state = "Idle", sub_target = next_cell, action = "stop" }
        end
        local delta = angle_to_target(st.sub_target.gx, st.sub_target.gy, x, y, yaw)
        if math.abs(delta) <= TURN_ALIGN_RAD then
            return { state = "Idle", sub_target = st.sub_target, action = "stop" }
        end
        -- 未对齐：保持（不重复发 turn）
        return { state = "Turning", sub_target = st.sub_target, action = "none" }

    elseif st.state == "Moving" then
        if next_cell == nil then
            return { state = "Idle", sub_target = nil, action = "stop" }
        end
        if st.sub_target == nil then
            return { state = "Idle", sub_target = next_cell, action = "stop" }
        end
        local tx = cell_center(st.sub_target.gx)
        local ty = cell_center(st.sub_target.gy)
        local dist = math.sqrt((x - tx)^2 + (y - ty)^2)
        if dist < SUB_TARGET_THRESHOLD_M then
            -- 到达当前格：前瞻下一格
            local nd = angle_to_target(next_cell.gx, next_cell.gy, x, y, yaw)
            if math.abs(nd) < STRAIGHT_ALIGN_RAD then
                -- 直行连续化：不停车，换目标继续走（车在动，不重发 move_forward）
                return { state = "Moving", sub_target = next_cell, action = "none" }
            else
                -- 需转向：停车回 Idle（下 tick 转向，天然隔 50ms）
                return { state = "Idle", sub_target = next_cell, action = "stop" }
            end
        end
        -- 未到 0.2m：保持（命令去重）
        return { state = "Moving", sub_target = st.sub_target, action = "none" }
    end

    -- 兜底
    return { state = "Idle", sub_target = nil, action = "stop" }
end
