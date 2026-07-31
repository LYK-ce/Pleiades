# Task 8: D* Lite 路径规划器

> 状态：设计中
> 创建日期：2026-07-31

## 目标

为 Executor 增加 D* Lite 路径规划能力，替代当前"sub_target = goal 网格，直线走"的简化逻辑，使小车能够绕开障碍物到达目标。

## 背景

当前 Executor 导航：

```
step_idle(): sub_target = goal 的网格坐标 → 直线走
```

问题：如果中间有障碍物，`Moving` 状态下的 `step_moving` 不检查 LiDAR 前方，车会撞上去。即使 `step()` 入口的急停逻辑触发，也只是停车，不会绕行。

Task 7 文档已设计规划器接口（stub 实现）：
- `next_step(start, goal, grid) → Option<(gx, gy)>` — 查询下一格
- `mark_obstacle(gx, gy)` — 通知规划器某格被标记为 Occupied

## 算法选择：D* Lite

相比 A*，D* Lite 的核心优势是**增量重规划**：

| 算法 | 首次规划 | 障碍出现后 | 适用场景 |
|------|:---:|:---:|------|
| A* | O(N log N) | 重新跑一遍 O(N log N) | 静态地图 |
| D* Lite | O(N log N) | O(Δ log Δ) 局部修补 | 动态环境、机器人导航 |

小车边走边建图，地图持续变化，D* Lite 不需要每次从头搜。

### 核心数据结构

```
DStarLite {
    g:       HashMap<(i32,i32), f32>,  // 已知最短距离 (g-value)
    rhs:     HashMap<(i32,i32), f32>,  // 一步前瞻值 (rhs-value)
    U:       BinaryHeap<Key, Cell>,     // 优先队列 (open list)
    km:      f32,                        // 启发式偏移累积
    start:   (i32, i32),                // 当前位置
    goal:    (i32, i32),                // 目标位置
}
```

### 算法流程（简化）

```
initialize():
    U.clear(); km = 0
    rhs[goal] = 0
    U.push(goal)

compute_shortest_path():
    while U.top().key < key(start) or rhs[start] != g[start]:
        u = U.pop()
        if g[u] > rhs[u]:
            g[u] = rhs[u]
            for each predecessor s of u:
                rhs[s] = min(rhs[s], cost(s,u) + g[u])
                update_vertex(s)
        else:
            g[u] = ∞
            for each predecessor s of u (包括 u 自己):
                rhs[s] = min over successors of cost(s,succ) + g[succ]
                update_vertex(s)

next_step():
    compute_shortest_path()
    从 start 沿 min(cost + g[neighbor]) 方向走一格

move_to(new_start):
    km += heuristic(start, new_start)
    start = new_start

mark_obstacle(cell):
    将该格 cost 设为 ∞
    update_vertex(cell)
    // 下次 next_step 时会局部修补
```

## 集成设计

### 嵌入点：Executor

```
DStarLite 放在 Executor 内：
  跨 tick 保留 g/rhs/U 状态
  每次 step_idle 中调用 next_step()
  遇到障碍时调用 mark_obstacle()
```

```rust
pub struct Executor {
    state: ExecState,
    goal: Option<(f32, f32)>,
    sub_target: Option<(i32, i32)>,
    config: ExecutorConfig,
    pathfinder: DStarLite,       // ← 新增
}
```

### 调用时序

```
step_idle():
  ┌─ pop 新 Mission → goal = (x,y)
  ├─ goal 网格 ≠ 当前位置
  │   └─ pathfinder.set_start(current_gx, current_gy)
  │   └─ pathfinder.set_goal(goal_gx, goal_gy)
  │
  └─ sub_target = None → 需要新格子
      └─ pathfinder.next_step(current, goal, grid)
          ├─ Some((sx, sy)) → sub_target = (sx, sy) → 转/走
          └─ None → 不可达 → warn, 跳过此任务

step():  // 急停检测
  └─ LiDAR 前方 < 0.3m
      ├─ stm32.stop()
      ├─ 计算障碍格坐标
      ├─ pathfinder.mark_obstacle(gx, gy)    // ← 通知规划器
      └─ state = Idle  → 下 tick 重规划
```

### 地图交互

```
OccupancyGrid 提供只读接口给 D* Lite：

  grid.state(gx, gy) → Free(0) / Occupied(1) / Unknown(2)

  D* Lite 判定：
    Free     → cost = 1  (可通过)
    Unknown  → cost = 1  (乐观，假设可通过)
    Occupied → cost = ∞  (不可通过)
```

## 待决策问题

### Q1: D* Lite 放在哪里？

| 方案 | 说明 |
|------|------|
| **A. Executor 内** | 唯一消费者，不需要共享。跨 tick 自然保留状态 |
| B. Robot 层 Arc<RwLock<>> | 和 grid 平级，允许多个消费者 |
| C. OccupancyGrid 内 | 地图自带规划 |

→ **倾向 A**

### Q2: 急停后如何处理？

当前急停只是 `stop → Idle`，下个 tick 还是同一个 goal 同一个 sub_target，可能重蹈覆辙。

```
方案 A: 急停 → mark_obstacle(障碍格) → sub_target = None → pathfinder 重新规划
方案 B: 急停 → mark_obstacle → 后退一格 → 重新规划
```

## ✅ 已决策

| # | 问题 | 决策 |
|---|------|------|
| Q1 | D* Lite 放哪里 | **Executor 内**（唯一消费者，跨 tick 保留状态） |
| Q2 | 急停后处理 | **mark_obstacle → sub_target=None → 下 tick 重规划** |
| Q3 | sub_target 粒度 | **到了再问**（sub_target=None 时才调 next_step） |
| Q4 | 连通方式 | **4 连通**（匹配小车前进+原地旋转运动模型） |
| Q5 | 初始化时机 | **pop 新 Mission 时 new**，到达后自然 drop |
| Q6 | 不可达目标 | 打 error 日志，跳过此 Mission，pop 下一个 |
| Q7 | g/rhs 存储 | **HashMap<(i32,i32), f32>**（稀疏，搜索节点远少于 65536） |
---

## 文件计划

```
Src/Robot/slam/pathfinder.rs    ← [重写] D* Lite 完整实现
Src/Robot/core/executor.rs      ← [修改] 集成 pathfinder
Src/Robot/core/robot.rs         ← [不修改] 主循环不变
```

## 实现步骤

1. 实现 `DStarLite` 核心：`g/rhs/U/km` + `compute_shortest_path` + `update_vertex`
2. 实现 `next_step(start, goal, grid)` 接口
3. 实现 `mark_obstacle(cell)` + `move_to(new_start)`
4. 集成到 Executor：`step_idle` 中替换 sub_target 计算
5. 集成急停：调用 `mark_obstacle`
6. 单元测试
7. 实车验证

---

## 人类评审

<!-- 在此区域写下评审意见 -->
