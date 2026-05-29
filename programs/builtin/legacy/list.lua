-- programs/list.lua
-- Presented by KeJi
-- Date: 2026-05-17
-- 节点列表脚本
COMMAND = "list"
DESCRIPTION = "列出所有可用节点及其资源信息"

function execute(params, caps)
    local peers = caps.get_available_peers()
    if #peers == 0 then
        return "No peers available"
    end

    local result = {}
    for i, p in ipairs(peers) do
        table.insert(result, string.format(
            "%s  [%s]  mem=%dMB  latency=%dms",
            p.peer_id, p.status or "unknown",
            p.memory_mb or 0, p.latency_ms or 0
        ))
    end
    return table.concat(result, "\n")
end
