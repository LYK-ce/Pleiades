# Task 14: Group Goto 群发任务

> 创建日期：2026-08-12
> 状态：方案已定稿，待实施
> 范围：ORION_TASK_SET 扩展（members 群发）+ 分布式散布分配 + WS hello 身份补充 + planning 目录

---

## 方案定稿摘要（2026-08-12 讨论收敛）

### ① 协议：TASK_SET（msgid 5）扩展，不加新 msgid

- payload 布局：`mission_count u8 + member_count u8 + members[](len u8 + peer_id 变长) + missions[]`（成员变长编码复用 frame.rs sysid 先例）
- 语义（2026-08-12 定稿）：`member_count == 0` → **取消全部任务**（Set(vec![])，replace 空队列 = 老 count=0 取消语义）；`== 1` → 单车任务；`> 1` → 群发任务
- 群发任务：取 missions 第一个 Goto 作为目标点
- 兼容性：字节格式变更，车端 + Pictor **同批升级，无过渡期**

### ② 执行流程（定稿：分配在执行时，main_loop 零改动）

```
main_loop: 收到 AutoCmd::Set(missions) → replace(missions)（纯入队，现有代码零改动）

executor(50ms tick):
  ① pop Mission::Goto { x, y, members }
  ② 调 assignment((x,y), members, own_peer_id, grid) → 自己的 goal
     - members 空      → goal = (x,y)（老单车语义）
     - members.len()==1 → goal = (x,y)（精确目标点，L[0]）
     - members.len()>1 → goal = 散布位置 L[i]
  ③ 以 goal 建 D* 路径（当前格 → goal 格）→ 问 sub_target → 转向/直行
```

- 分配时机在执行时（pop 后）；任务队列存原始任务（目标点 + members）
- executor 需要 own_peer_id（step 签名加参数，launch 已取）

### ③ 分配算法（新模块 `Src/Robot/core/planning/assignment.rs`，纯函数）

- 输入：目标点 (x,y)、members（按 peer_id 字节升序排序）、自己的序号 i、本地 merged 地图
- 目标格 Occupied → **不可达，任务失败**（不偏移）
- 位置列表 L[0] = 精确目标点（头车）；L[1..N-1] = 棋盘同色格 `(gx+gy)%2==0` 的格中心，切比雪夫距离环从内到外枚举（环内固定顺序：正上方起顺时针），Occupied 跳过；环上限 10（≈5m）
- 所有车同一规则 → 同一份 L → 零冲突；N=1 时只需 L[0]
- 返回：自己的目标点 → executor 以它为 goal

### ④ 目录结构调整（2026-08-12 决策：planning 规划层）

- 新建 `Src/Robot/core/planning/` 规划层目录，容纳：
  - `assignment.rs`（新）— 任务分配（去哪）
  - `pathfinder.rs`（从 `slam/` 迁入）— D* 寻路（怎么走）
  - 未来：避碰/让行（multi_robot_control P3/P4 动态障碍注入、冲突检测）
- 分层语义：`planning` = 决策层（消费地图），`slam` = 感知/建图层（产生地图）；依赖方向 `planning → slam(grid 只读)`，无环
- 引用点调整：`executor.rs` use 路径、`slam/mod.rs` 删 `pub mod pathfinder`、`core/mod.rs` 加 `pub mod planning`；问题池 `robot_review_problem.md` 中 `slam/pathfinder.rs` 路径引用需同步

### ⑤ WS hello 身份补充（本任务配套）

**动机**：终端构造群发任务 members 需要每辆参与车的 peer_id 字节，且必须与车端排序所用字节表示完全一致；目前 hello 只发 `vehicle_id`（车名），终端只能从下行帧头 sysid 间接解析，无显式身份宣告通道（详见 orion_protocol.md §1.5）。

**最新 hello 帧格式**（补充 peer_id 后，仍是唯一保留的 JSON 消息）：

```json
{
  "type": "hello",
  "vehicle_id": "robot-01",
  "address": "ws://0.0.0.0:9090",
  "peer_id": "0024080112201c...76 个 hex 字符..."
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| type | string | 固定 `"hello"`（连接握手，仅发一次） |
| vehicle_id | string | 车名 = peer_name（Task 9_2 决策 #6） |
| address | string | 本车 WS 地址 |
| peer_id | string | 本车完整 peer_id 的 **hex 编码** = `PeerId::to_bytes()` 逐字节 `{:02x}`（multihash，Ed25519 下 38B → 76 hex 字符）；**与帧头 sysid 字节完全一致** |

**关键语义**：
- 终端连接即得车身份，无需解析帧头 sysid；
- 终端构造 members 时：hex 解码回原始字节 → **原样回填**帧内 → 与车端排序所用字节一致（一致性核心）；
- 编码选 hex：简单可读、零新依赖（base58 需引入库，不做）。

**实施步骤（hello 部分）**：

1. `Src/WebSocket/server.rs` `handle_connection`（L128-137，**已有 `local_peer_id: Vec<u8>` 参数，签名零变更**）L139-143 hello 构造处改为：
   ```rust
   let peer_hex: String = local_peer_id.iter().map(|b| format!("{:02x}", b)).collect();
   let hello = serde_json::json!({
       "type": "hello",
       "vehicle_id": vehicle_id,
       "address": format!("ws://{bind_addr}"),
       "peer_id": peer_hex,
   });
   ```
2. 验证：`cargo build` 通过后，用 wscat/python websocket 手工连接，确认 hello 输出含 `peer_id` 且与帧头 sysid hex 一致；
3. 文档：`orion_protocol.md` §1.5 状态"待补充"→"已实施"，附字段表；
4. Pictor（外部仓库）：解析 hello 新增 `peer_id` 字段，维护"在线车 → peer_id"表，群发任务 members 使用 hex 解码后的原始字节。

**改动面**：仅 `server.rs` hello 构造一处 + 文档；无签名变更、无新依赖、不影响现有帧协议。

---

## 实施步骤（全量，按依赖顺序）

### Step 1: planning 目录 + pathfinder 迁移（结构先行）

- 新建 `Src/Robot/core/planning/mod.rs`：`pub mod pathfinder;`（内容原样迁入）
- `Src/Robot/slam/mod.rs`：删 `pub mod pathfinder;`
- `Src/Robot/core/executor.rs` L17：`use` 改为 `crate::robot::core::planning::pathfinder::DStarLite`
- `Src/Robot/core/mod.rs`：加 `pub mod planning;`
- 文件头 Modified Date bump 至 2026-08-12；问题池 `robot_review_problem.md` 中 `slam/pathfinder.rs` 路径引用同步
- 验证：`cargo check`

### Step 2: 协议编解码扩展（`Src/Robot/core/protocol/messages.rs`）

- 新增 `TaskSetPayload { mission_count: u8, member_count: u8, members: Vec<Vec<u8>>, missions: Vec<MissionItem> }`
- `encode_task_set` / `decode_task_set`（L235-267）重写为新布局：`mission_count u8 + member_count u8 + members[](len u8 + peer_id 变长) + missions[](9B/条)`；成员变长编码复用 frame.rs sysid 先例（L37-42）
- 解码校验：`payload.len() < 2 → None`；members 每项 `off+len` 越界 → None（防恶意帧）；missions 剩余严格 `9×mission_count`
- `protocol/mod.rs` re-export 新结构
- 测试：群发帧往返（单车/多车）、**取消帧（member_count=0）往返 → Set(vec![])**、长度越界 → None、**旧格式（`[0]` / `[1,0,...]` / `[2,0,...]` 三种典型旧帧）显式断言被拒**
- 验证：`cargo test -p Pleiades --lib robot::core::protocol`（现有 `test_task_set_roundtrip` 需同步重写——预期变更）

### Step 3: Mission 扩展（`Src/Robot/core/command.rs`）

- `Mission::Goto(f32, f32)`（L50-54）扩展为结构体：`Goto { x: f32, y: f32, members: Vec<Vec<u8>> }`
  - members 空 → 老单车语义（goal = (x,y)）；非空 → 群发（executor 调 assignment）
- `AutoCmd::Set(Vec<Mission>)` **零改动**（队列替换语义不变）
- 引用点：`WebSocket/protocol.rs` 构造处、`executor.rs` pop 解构处、相关测试
- 验证：编译通过

### Step 4: 分配模块（`Src/Robot/core/planning/assignment.rs` 新文件）

- `core/mod.rs` / `core/planning/mod.rs` 注册
- 纯函数（拆两个便于测试）：
  - `build_slots(target, grid) -> Result<Vec<(f32,f32)>, AssignError>`：目标格 Occupied → `Err(Unreachable)`；目标点 `world_to_grid` 越界（gx/gy 出 [0,256)）→ `Err(OutOfBounds)`；L[0]=精确目标点；切比雪夫环 r=1..=10 枚举同色格 `(gx+gy)%2==0`（环内固定顺序：正上方起顺时针，**禁止 HashMap/不稳定枚举**），**Occupied / OOB 格同等跳过**，格中心 `(gx*0.5+0.25, gy*0.5+0.25)`；槽位不足（r=10 共 221 槽）→ `Err(InsufficientSlots)`
  - `group_goto_mission(target, members, own_peer_id, grid) -> Result<(f32,f32), AssignError>`：members 空 → 直接返回 target（老单车）；members `sort()`（**字节升序，不信任帧序**）→ 序号 i → L[i]；own 不在列表 → `Err(NotMember)`
- 单测（≥10 用例）：单车 N=1、多车 N=3、同色性、障碍跳过、目标格不可达、环上限、确定性（同输入两次全等）、peer_id 排序/不在列表、格中心公式与 executor 交叉断言、members 空直接返回 target
- 验证：`cargo test`

### Step 5: WS 解析三分支（`Src/WebSocket/protocol.rs`）

- L26-36 TASK_SET 分支改：`decode_task_set` 新签名 → 按 `member_count` 三分支（**统一产出 `AutoCmd::Set(Vec<Mission>)`**）：
  - `== 0` → **取消：`Set(vec![])`**（replace 空队列，老取消语义）
  - `== 1` → 老行为：missions 过滤未知 type → `Mission::Goto { x, y, members: vec![] }` → `Set(missions)`
  - `> 1` → **遍历 missions 找第一个 `MISSION_GOTO`** 为目标点 → `Set(vec![Mission::Goto { x, y, members }])`；无 Goto → warn + `None`
- 新增测试（该文件现无测试模块）：三分支（含取消）+ 无 Goto + 未知 type 过滤 + members 透传

### Step 6: WS hello 身份补充（`Src/WebSocket/server.rs`）

- 见上 ⑤ 节详细步骤（仅 hello 构造一处，签名零变更）

### Step 7: main_loop 签名加参（`Src/Robot/core/robot.rs`）— Set 分支零改动

- `launch`（L193-202 spawn 前）：算一次 `local_peer_id`（仿 L181 consumer 同款：`node_handle.as_ref().map(|nh| nh.Get_Local_Peer_Id().to_bytes()).unwrap_or_default()`，单机为空 vec）→ 传入 main_loop
- `main_loop` 签名（L366-374）加 `own_peer_id: Vec<u8>` 参数；`auto_tick` 调 step 处（L442-446）传入
- `AutoCmd::Set` 分支（L415-421）本身**零改动**（纯入队 replace）
- 注：单机（无 node_handle）收到群发任务 → `NotMember` 失败 warn，属预期行为（单车不受影响）
- 验证：编译（`Mission::Goto` 变体变化仅影响 protocol.rs / executor.rs）

### Step 8: executor 接入 assignment（`Src/Robot/core/executor.rs`）

- `own_peer_id` **三层贯穿**：`step()`（L105-116）/ `step_impl()`（L119）/ `step_idle()`（L171，pop 分支所在，已有 `grid` 参数 L175）签名均加 `own_peer_id: &[u8]`
- `step_idle` pop 分支（L192-209）：`Mission::Goto { x, y, members }` → 调 `assignment::group_goto_mission((x,y), &members, own_peer_id, grid)` → `Ok(goal)` → `self.goal = Some(goal)` → 建 D*（当前格 → goal 格）；`Err(e)` → warn + 任务失败（不装载）
- 逻辑测试全部落在 assignment 纯函数，executor 只做胶水

### Step 9: 编译 + 全量测试

- `./build.sh check` + `cargo test`；确认：`decode_task_set` 签名变更波及面 = WebSocket/protocol.rs + messages.rs 测试两处（已核实）；`Mission::Goto` 变体变更波及面 = protocol.rs / executor.rs / 相关测试

### Step 10: 文档同步

- `orion_protocol.md` §3.5（payload 布局 + 三分支语义 + 同批升级说明）、§1.5（hello peer_id 已实施）
- `multi_robot_control.md` §4.1/§4.2/§4.3（命令格式对齐 + 定稿算法 + 头车占精确点）
- `Pleiades_Architecture.md` §3.10（文件结构补 planning/ + Mission 扩展说明，顺带更新已脱节的 Command 描述）

### Step 11: Pictor 端同步（外部仓库）

- 编码侧全在外部：TASK_SET 新布局编码 + hello 新字段解析 + 在线车表维护
- **与车端同批上线，无过渡期**（老 Pictor 发旧帧 → 车端全拒）

---

## 待办

- [ ] hello 增加 peer_id 字段（server.rs hello 构造处 + orion_protocol.md 已记录）
- [ ] 协议编解码扩展（messages.rs）+ 测试
- [ ] Mission::Goto 扩展 { x, y, members }（command.rs）
- [ ] pathfinder.rs 迁移 `slam/` → `core/planning/`（引用点/问题池路径同步）
- [ ] 分配模块（core/planning/assignment.rs）+ 单元测试
- [ ] WS 解析三分支（WebSocket/protocol.rs）
- [ ] executor 接入 assignment（pop 后算 goal，step 加 own_peer_id）
- [ ] 编译验证 + 全量测试
- [ ] 文档同步（orion_protocol.md §3.5 / multi_robot_control.md §4 / Pleiades_Architecture.md §3.10）
- [ ] Pictor 端同步（外部仓库，同批升级）
