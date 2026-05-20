-- programs/builtin/flush.lua
-- Presented by KeJi
-- Date: 2026-05-20
-- 刷新存储索引

COMMAND = "flush"
DESCRIPTION = "刷新存储索引（扫描磁盘文件，更新 FileEntry）"

function execute(params)
    local r = caps.storage_flush()
    caps.print(string.format("flush 完成: 新增 %d 个, 移除 %d 个", r.added, r.removed))
end
