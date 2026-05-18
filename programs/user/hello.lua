-- programs/hello.lua
-- Pleiades Lua 演示脚本
COMMAND = "hello"
DESCRIPTION = "演示脚本：验证 Lua 引擎正常启动"

function execute(params)
    caps.print("Hello from Pleiades Lua engine!")
    caps.print("Params received: msg = " .. (params.msg or "(none)"))
    return "ok"
end
