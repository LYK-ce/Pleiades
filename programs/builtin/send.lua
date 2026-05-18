-- programs/builtin/send.lua
-- 文件发送脚本
COMMAND = "send"
DESCRIPTION = "向指定节点发送文件"

function execute(params)
    caps.print("正在发送 " .. params.file .. " → " .. params.peer)
    local handle = caps.storage_acquire_read(params.file)
    caps.network.send_file(params.peer, handle:path())
    caps.print("文件发送完成: " .. params.file)
    return "ok"
end
