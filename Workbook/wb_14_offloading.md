# Workbook 14: Offloading

- 开始时间：2026-06-01
- 分支：task14swaplayer
- 状态：设计完成，待实现

## 设计决策
- 以 MlContext 为单位 offload
- 权重不序列化，从 PGGUF reload
- KV Cache 序列化保存（每层 2 tensor）
- 磁盘缓存目录：.kvcache/，不走 Storage
- AnyModel 统一 extract/restore KV Cache 接口

## 待实现
1. .kvcache/ 目录创建
2. AnyModel::extract_kv_cache / restore_kv_cache
3. 序列化/反序列化函数
4. MlSession offload 方法
5. Lua 绑定
6. 测试
