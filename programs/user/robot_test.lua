-- Presented by KeJi
-- Date ： 2026-06-18
-- 简单机器人控制测试：前进 + 蜂鸣

COMMAND = "robot_test"
DESCRIPTION = "机器人控制测试：前进 2 秒 + 蜂鸣 200ms"

function execute(params)
    local port = params.port or "/dev/myserial"
    local car_type_str = params.car_type or "X3Plus"
    local speed = tonumber(params.speed) or 50

    -- 车型映射
    local car_type_map = {
        X3 = 1,
        X3Plus = 2,
        X1 = 4,
        R2 = 5,
    }
    local car_type = car_type_map[car_type_str] or 2

    caps.print("===== 机器人控制测试 =====")
    caps.print(string.format("串口: %s", port))
    caps.print(string.format("车型: %s (0x%02x)", car_type_str, car_type))

    -- 1. 打开串口
    caps.print("[1/4] 打开串口 ...")
    local ok, err = pcall(function()
        return robot.open(port, 115200, car_type)
    end)
    if not ok then
        caps.print(string.format("  ✗ 打开失败: %s", tostring(err)))
        return
    end
    caps.print("  ✓ 串口已打开")

    -- 2. 前进
    caps.print(string.format("[2/4] 前进 (速度=%d) ...", speed))
    robot.forward(speed)
    caps.print(string.format("  ✓ 前进指令已发送"))

    -- 等待 2 秒（让小车跑一会儿）
    caps.print("  等待 2 秒 ...")
    local start = caps.monotonic_time()
    while caps.monotonic_time() - start < 2.0 do
        local state = robot.get_state()
        caps.print(string.format(
            "  速度=(%.2f,%.2f,%.2f) 电池=%.1fV 编码器=(%d,%d,%d,%d)",
            state.vx, state.vy, state.vz,
            state.battery,
            state.encoders[1], state.encoders[2],
            state.encoders[3], state.encoders[4]
        ))
        -- 每 200ms 读一次传感器
        local wait_start = caps.monotonic_time()
        while caps.monotonic_time() - wait_start < 0.2 do end
    end

    -- 3. 停车
    caps.print("[3/4] 停车 ...")
    robot.stop()
    caps.print("  ✓ 停车指令已发送")

    -- 4. 蜂鸣
    caps.print("[4/4] 蜂鸣 200ms ...")
    robot.beep(200)
    caps.print("  ✓ 蜂鸣指令已发送")

    -- 最终状态
    local final = robot.get_state()
    caps.print("")
    caps.print("===== 测试完成 =====")
    caps.print(string.format("电池: %.1fV", final.battery))
    caps.print(string.format("速度: (%.2f, %.2f, %.2f)", final.vx, final.vy, final.vz))
    caps.print(string.format("姿态: roll=%.2f° pitch=%.2f° yaw=%.2f°",
        final.attitude.roll * 57.3,
        final.attitude.pitch * 57.3,
        final.attitude.yaw * 57.3))
    caps.print(string.format("编码器: (%d, %d, %d, %d)",
        final.encoders[1], final.encoders[2],
        final.encoders[3], final.encoders[4]))
end
