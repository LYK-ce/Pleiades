-- Presented by KeJi
-- 车决策脚本（Task 22 步骤 5/6）：无状态决策，Rust 50ms 定时器驱动调 on_tick
-- 读位置 → get_path 拿下一格 → 算角偏差 → 转向/直行
--
-- caps 接口（Rust 侧 register_robot_caps 注册）：
--   self.get_position()  -> (x, y, z)
--   self.get_attitude()  -> (roll, pitch, yaw)
--   self.get_velocity()  -> (vx, vy, vz)
--   world.get_cell(x, y) -> 0=Free / 100=Occupied / 255=Unknown / nil
--   world.get_agents()   -> 其他设备列表
--   world.get_path()     -> {gx=, gy=} 下一格 / nil（无任务/到达）
--   action.move_forward(speed) / move_backward / turn_left(rate) / turn_right(rate) / stop()

local CELL_RESOLUTION = 0.5        -- 米/格（与 Rust CELL_RESOLUTION 一致）
local TURN_ALIGN_RAD = math.rad(5.0)  -- 5° 对齐阈值（与 ExecutorConfig 一致）
local TURN_SPEED = 10               -- 转向速度（与 ExecutorConfig.turn_speed 一致）
local MOVE_SPEED = 30               -- 直行速度（与 ExecutorConfig.move_speed 一致）

local function normalize_angle(a)
    return (a + math.pi) % (2.0 * math.pi) - math.pi
end

function on_tick()
    local x, y = self.get_position()
    local _, _, yaw = self.get_attitude()

    local next = world.get_path()
    if next == nil then
        action.stop()
        return
    end

    -- 格中心世界坐标（格 (gx, gy) 覆盖世界 [gx·R, (gx+1)·R)，中心 (gx+0.5)·R）
    local tx = (next.gx + 0.5) * CELL_RESOLUTION
    local ty = (next.gy + 0.5) * CELL_RESOLUTION
    local target_angle = math.atan(ty - y, tx - x)
    local delta = normalize_angle(target_angle - yaw)

    if math.abs(delta) > TURN_ALIGN_RAD then
        if delta > 0 then
            action.turn_right(TURN_SPEED)
        else
            action.turn_left(TURN_SPEED)
        end
    else
        action.move_forward(MOVE_SPEED)
    end
end
