# Task 12: Broadcast 广播机制改造（request-response → gossip hub）

> 状态：设计中——方向已明确（2026-08-08 讨论），具体方案待确认
> 创建日期：2026-08-08
> 最后更新：2026-08-08

## 目标

将小车之间（车↔车集群）的消息广播从 **request-response 模拟** 改为 **gossip hub（pub-sub）** 形式，降低广播性能开销，消除逐 peer 请求无回执导致的超时日志噪音。

## 背景

- 当前 `Broadcast`（`NodeHandle::Broadcast`）为 request-response 模拟：逐 peer 发送请求、无回执 → 每 peer 超时日志噪音 + 线性开销（O(N) 逐 peer 复制传输）
- 单车时代可用；多车集群（比赛 9/15 提交：大规模无人集群联合多域防控）下，广播频率（位姿 10Hz + 地图 5Hz）× 车辆数增长，性能开销偏高
- task_11 已记录此暂缓项：*"⏳ 传输层广播机制：Broadcast 现为 request_response 模拟（逐 peer 请求无回执 → 超时日志噪音）；建议多车阶段改用 gossipsub（Network 模块范畴，ML_review 分支）"*

## 方案方向

gossip hub（gossipsub / floodsub 等 P2P pub-sub 协议）替代 request-response 模拟广播。

## 待决策

（待讨论确认，2026-08-08 讨论后补充）

## ⚠️ 分支边界提醒

`.github/instructions.md` 分支职责声明：**Network / P2P 网络属 `ML_review` 分支职责**，`Pleiades-Orion` 分支禁止修改。
本任务涉及 Network 模块改造，需人类确认跨分支处理方式（Orion 分支实现 / 移交 ML_review / 其他）。

---

## 人类评审

<!-- 在此区域写下评审意见 -->
