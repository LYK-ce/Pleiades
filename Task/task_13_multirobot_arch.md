# Task 13: Multi-Robot Control（多车协同控制）

> 状态：进行中——**阶段一（协议调整：sysid → 完整 peer_id）已定案，待实施**
> 创建日期：2026-08-10
> 最后更新：2026-08-10
> 设计文档：`docs/design_doc/multi_robot_control.md`（多车协同总设计）
> 问题池：`Task/robot_review_problem.md`

## 目标

向多车协同演进。按阶段推进（与人类逐项讨论确认后实施），当前聚焦**阶段一**：调整 ORION 协议，把 1 字节 sysid 扩展为完整 libp2p PeerId，使其真正承担"车身份"职责——这是后续多车功能（RemoteRobotInfo 表、members 快照、避碰）的身份地基。

---

# 阶段一：协议调整——sysid 扩展为完整 peer_id

## 背景与决策记录（2026-08-10 讨论收敛）

| 议题 | 结论 |
|---|---|
| 现状缺陷 | `sysid` 1 字节 = peer_id multihash 末字节（`sysid.rs:13-15`），碰撞率 **1/256**；且 WS 链路恒填 0，同一辆车两条链路身份不一致 |
| 语义澄清 | MAVLink 的 sysid 是"域内编址"（可伪造、可重复），**不是身份**；我们的用法是"身份标识"——拿 1 字节压缩码硬扛身份职责，天生不够 |
| 决策 | **sysid 扩展为完整 peer_id 二进制（~38B，Ed25519 multihash `[0x00 0x24 + 36B 公钥]`）**。带宽影响可忽略（pose 10Hz ≈ +380B/s）；libp2p Signed 消息本就携带 from(34B)+签名(64B)，协议层边际成本 ~3% |
| 实现方案 | **方案 A（选定）：u8 长度前缀 + 变长 bytes**——与密钥类型解耦、天然支持"空身份"（地面站）、可检测旧帧。方案 B（定长 38B）被否：与 Ed25519 强耦合 |
| 字段语义 | 帧内 `sysid` 字段实质变为"作者/author"，但**保留字段名 sysid 与 compid 并存**（compid 继续表达 车=1/终端=200），文档需注明语义升级 |
| 不变量保持 | `sysid.rs` 模块保持"不依赖 libp2p 类型"（接口 `&[u8]`）；`DataType::Robot` wire compat 不受影响（网络层字节透传） |

## 新帧布局（frame.rs）

```
旧：magic(1) | len(4) | seq(1) | sysid(1) | compid(1) | msgid(2) | payload(len) | checksum(2)   = 10B 头
新：magic(1) | len(4) | seq(1) | sysid_len(1) | sysid(N) | compid(1) | msgid(2) | payload(len) | checksum(2)
    └──────── 固定部分 12B + sysid(N) 变长 ────────┘
```

- `len` 仍为 payload 长度（u32 BE），含义不变
- `sysid_len`（u8，上限 255）在前，`sysid` 为 `sysid_len` 字节；**长度 0 表示"无身份"（地面站上行用）**
- `decode_frame` 先读 `sysid_len` 再计算后续偏移（seq=5, sysid_len=6, sysid=[7..7+N], compid=[7+N], msgid=[8+N..10+N], payload=[10+N..10+N+len]）
- `FRAME_HEADER_LEN=10` 常量废弃，改为函数 `header_len(sysid_len)` 或内联动态计算

## 涉及文件与具体改法

| # | 文件:行号 | 当前代码 | 改法 |
|---|---|---|---|
| 1 | `protocol/frame.rs:19` | `FRAME_HEADER_LEN=10` | 废弃常量 → 动态 header 长度 |
| 2 | `protocol/frame.rs:23-28` | `Frame { sysid: u8, ... }` | `sysid: u8` → `sysid: Vec<u8>` |
| 3 | `protocol/frame.rs:31-42` | `encode_frame(msgid, sysid: u8, compid, payload)` | 签名 `sysid: u8` → `sysid: &[u8]`；写 1B 长度前缀 + N 字节 |
| 4 | `protocol/frame.rs:45-68` | `decode_frame` 固定偏移（L46/L52/L56/L58/L59/L60/L61） | 动态偏移（先读 sysid_len） |
| 5 | `protocol/frame.rs:9` | doc 帧布局图 | 更新为新布局 |
| 6 | `protocol/sysid.rs:13-15` | `sysid_from_multihash(&[u8]) -> u8`（末字节） | **删除**（或改为 `peer_id_to_bytes` 直通）；测试同步删 |
| 7 | `protocol/mod.rs:25` | `pub use sysid::sysid_from_multihash;` | 跟随删除/改名 |
| 8 | `core/robot.rs:217` | `sysid_from_multihash(&nh.Get_Local_Peer_Id().to_bytes())` | **launch 时算一次 peer_id 传闭包**（顺带解决问题池 P3#9 每拍重算）；encode_frame 传 `&peer_id` |
| 9 | `core/robot.rs:225` | `encode_frame(MSGID_POSE, sysid, ...)` | 传 `&peer_id` |
| 10 | `core/robot.rs:288` | 同 8（slam_task） | 同上 |
| 11 | `core/robot.rs:294` | `encode_frame(MSGID_MAP_DELTA, sysid, ...)` | 传 `&peer_id` |
| 12 | `core/robot.rs:342-343` | `info!("... sysid={} ...")` | 打印改 hex 短格式（前 8 字节 hex）或 `{:?}`；若做身份过滤则在此消费 |
| 13 | `WebSocket/server.rs:63,83,150` | `encode_frame(..., 0, COMPID_ROBOT, ...)` sysid 恒 0 | 填本车 peer_id（见"WS 链路"） |
| 14 | `WebSocket/mod.rs:12-27` | `start(bind, vehicle_id, cmd_tx, pose_rx, map_rx, grid)` | 增参 `peer_id: Vec<u8>`（或 Arc<NodeHandle>），传给 `server::run` |
| 15 | `bootstrap.rs:225-235` | `websocket::start(&ws_bind, &peer_name, ...)` | 从已有 `Arc<NodeHandle>` 取 `Get_Local_Peer_Id().to_bytes()` 一并传入 |
| 16 | `WebSocket/protocol.rs:19-45` | `parse_orion_frame` 只用 msgid/payload | **无需改**（不消费 sysid；签名变化编译期自动传导） |

**不受影响（已确认）**：`executor.rs`、`TUI/mod.rs`、`main.rs`、`swarm_events.rs`（字节透传）、`command_handler.rs`、`codec.rs`（DataType::Robot 保留）、`messages.rs` payload 编解码（无 sysid）。

## WS 链路身份方案（地面站方向）

- **下行（车→地面站）**：三处 encode_frame 填**本车完整 peer_id**（经 bootstrap 注入）——消除"gossip 链路真实身份 vs WS 链路恒 0"的不一致（问题池 N6 关联）
- **上行（地面站→车）**：Pictor 当前无 peer_id → **sysid_len=0（空身份）+ compid=200**（启用 `COMPID_GROUND_STATION` 常量，`mod.rs:36` 目前零使用）；decode 端约定"空 sysid + compid==200"= 终端命令。未来 Pictor 迁移 libp2p 后天然升级
- 当前 `parse_orion_frame` 不校验身份，此改动**零风险**

## 测试改动

| 测试 | 位置 | 改法 |
|---|---|---|
| `test_sysid_from_multihash` | `sysid.rs:22-29` | 删除（函数删除）或改为完整字节断言 |
| `test_frame_roundtrip` | `frame.rs:75-83` | 传 `&[u8]` 身份（如 `vec![7]` 或 38B 假 peer_id）；断言 `decoded.sysid == 期望字节` |
| `test_frame_bad_magic` | `frame.rs:86-89` | 签名跟随 |
| `test_frame_len_mismatch` | `frame.rs:92-96` | 变长后"截断一字节"语义需重新设计测试数据 |
| `test_frame_empty_payload` | `frame.rs:99-104` | 签名跟随 |
| `test_frame_with_message` | `messages.rs:295-311`（L309 断言 sysid==7） | 签名跟随 + 断言改字节比较 |
| 集成测试 t01~t13 | `tests/` | **不受影响**（不涉及 ORION 帧） |

## 文档同步

| 文档 | 位置 | 动作 |
|---|---|---|
| `docs/design_doc/orion_protocol.md` | §2 帧格式（L66-83）、§4 身份（L281-300） | **整体重写**：帧布局表、sysid 派生公式（L288"末字节"→ 完整 peer_id）、compid 语义、WS 身份约定 |
| `Task/archived task/task_11_robot_update.md` | L37-39 旧决策 | 标注作废（已归档，仅注释） |
| `docs/design_doc/multi_robot_control.md` | — | P1 任务协议章节引用帧内 peer_id 字段（后续阶段做） |
| **外部仓库提醒** | Pictor（GodotProject，不在本仓库） | 帧解析需同步升级——跨仓库改动面，需人类协调 |

## 实施步骤（顺序）

1. `frame.rs`：Frame 结构 + encode/decode 动态偏移（核心，含 doc 注释更新）
2. `sysid.rs`：删除函数 + 测试；`mod.rs` re-export 同步
3. `robot.rs`：launch 时取一次 peer_id → 传闭包 → 两处 encode_frame 改签名；打印改 hex
4. `bootstrap.rs` + `WebSocket/`：peer_id 注入链（bootstrap → websocket::start → server::run → 三处 encode_frame）
5. 测试重写（frame.rs × 4、messages.rs × 1）
6. `cargo check` + `cargo build --release --bin orion-robot` + `cargo test`（Robot 相关）
7. `orion_protocol.md` §2/§4 重写
8. （可选）双车联调：A/B 车互收帧，日志打印对方完整 peer_id 短 hex，验证身份区分
9. git 提交（单 commit，含代码+测试+文档）

## 风险清单

1. **硬编码偏移**：frame.rs L46/L52/L56-L61 漏改任一处即解出错误字段（最易错点）
2. **最小/精确长度校验**：L46/L53 的 `FRAME_HEADER_LEN+2` 与 `FRAME_HEADER_LEN+len+2` 必须动态化；`test_frame_len_mismatch` 语义需重设计
3. **日志噪音**：`sysid={}` 打印 Vec<u8> 会刷 `[1, 36, ...]`——必须格式化（hex 短格式）
4. **旧帧兼容**：SnapshotCache 可能重放旧格式帧（变长方案可检测：旧帧 sysid_len=1 恰好是旧 sysid 值，需评估是否拒绝/容忍）
5. **协议模块的 libp2p 边界**：保持 `&[u8]` 接口，不让 protocol 依赖 libp2p crate
6. **Pictor 跨仓库**：外部地面站帧解析不同步则显示异常——需人类安排

## 待决策点

1. ✅ 方案 A（u8 长度前缀 + 变长）——已定
2. ✅ 上行空身份（sysid_len=0）+ compid=200——已定（Pictor 无 peer_id）
3. ⏳ 日志身份格式：前 8 字节 hex vs 完整 hex vs base58（建议前 8 字节 hex，调试够用）
4. ⏳ 是否顺带把 WS 下行改为订阅 robot_bus 复用已编码帧（更彻底消除双链路身份不一致，但改动面更大）——本期先做"填 peer_id"，该优化留后续

---

# 后续阶段（待与人类讨论，逐项确认后填充）

- P0：DStarLite 动态障碍接口（问题池 P1）
- P1：任务消息协议 + `pleiades/robot/task` topic + 确定性分配
- P2~P6：散布 / 障碍注入 / 冲突检测避碰 / 点云过滤 / 僵持协商
- 待决策清单（9 项）暂存 `multi_robot_control.md` §8

---

## 人类评审

<!-- 在此区域写下评审意见 -->
