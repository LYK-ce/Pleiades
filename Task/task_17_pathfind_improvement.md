# Task 17: 循路改善（Pathfind Improvement）

> 创建日期：2026-08-16
> 状态：已实施（代码 + 单测通过，2026-08-16）
> 范围：他车动态障碍「点格」→「圆形几何膨胀」（半径 20cm），膨胀半径进 config；本车 footprint 处理暂缓（记入 `robot_review_problem.md` N13）

---

## 一、背景

多车巡路时，系统把每辆小车的世界坐标经 `world_to_grid` 映射为单一障碍格注入动态障碍集合，再结合动态 + 静态障碍做 D* Lite 巡路。存在问题：

- **量化碰撞**：栅格 50cm×50cm，车径约 30cm（半径 15cm）。`cluster_to_obstacle_cells` 每辆车只标记其中心所在 **1 个格、零膨胀**。
- 两车物理相切（圆心距 30cm）时，只要圆心连线跨越格边界，就被识别为两个不同格，巡路不避让（极端时圆心距 0.2cm 也分属两格）。
- 车心靠格边/格角时，车体伸出相邻 Free 格最多 15cm；对角相切时 D* 会剪角穿行，实际碰撞。
- 静态地图不兜底：`lidar_mapper` 主动把他车格 decay 成 Unknown（可通行），避让完全依赖动态障碍。

## 二、循路机制变化：从「点」到「圆盘」

### 2.1 现状（点障碍）

```
他车世界坐标 (x, y)  ──world_to_grid──▶  中心格 (gx, gy)   ──▶  动态障碍集合 { (gx, gy) }
```

每辆车只贡献 **1 个格**。D* Lite 的 `cost()` 把「点机器人」规划路径，看到这个格才绕行；车体 15cm 伸出相邻格的部分完全不可见。

### 2.2 改动后（圆盘膨胀）

```
他车世界坐标 (x, y)  ──圆心 + 半径 R=0.2m 的圆盘──▶  与圆盘相交的所有格  ──▶  动态障碍集合
```

每辆车贡献 **1~4 个格**（取决于圆心在格内的位置），圆盘是车体的安全余量表示。

### 2.3 算法：圆心 + 半径 → 覆盖格集合（计算过程）

**常量/参数**

| 符号 | 值 | 含义 |
|---|---|---|
| `CELL_RESOLUTION` | 0.5 m/格 | 栅格分辨率（`grid.rs` 已有） |
| `R` | 0.2 m（20cm） | 膨胀半径，来自 config `[Robot].obstacle_inflation_radius` |
| `(x, y)` | 米 | 他车世界坐标（`ClusterInfo.x/y`，已是 f32 米） |

格 `(gx, gy)` 覆盖世界矩形 `[gx·0.5, (gx+1)·0.5) × [gy·0.5, (gy+1)·0.5)`。

**步骤**

1. **定位中心格**：`(cx, cy) = world_to_grid(x, y) = (floor(x/0.5), floor(y/0.5))`。
2. **圈定候选格**：`reach = ceil(R / 0.5) = ceil(0.4) = 1`，即圆盘最多伸入相邻 1 格 → 候选为以 `(cx, cy)` 为中心的 `(2·reach+1)²` 邻域（R=0.2 时为 3×3）。
3. **圆-矩形相交判定**（对每个候选格）：算「圆心到格子矩形的最近距离」，≤ R 则圆盘与该格相交 → 标记。最近距离用**分量夹取**计算：

   ```
   x0 = gx·0.5,  x1 = (gx+1)·0.5
   y0 = gy·0.5,  y1 = (gy+1)·0.5
   px = clamp(x, x0, x1)        // 圆心 x 夹到矩形 x 区间
   py = clamp(y, y0, y1)        // 圆心 y 夹到矩形 y 区间
   dist² = (x - px)² + (y - py)²
   若 dist² ≤ R²  →  标记 (gx, gy)
   ```

   语义：圆心在矩形内 → dist=0（必标）；在矩形外 → 到最近边/角的距离。
4. **去重收集**：所有他车的结果并入 `HashSet<(i32,i32)>`（同格自动去重）。

**伪代码（Rust）**

```rust
pub fn cluster_to_obstacle_cells(others: &[ClusterInfo], radius: f32) -> HashSet<(i32, i32)> {
    let reach = (radius / CELL_RESOLUTION).ceil() as i32;
    let r2 = radius * radius;
    let mut cells = HashSet::new();
    for info in others {
        let (cx, cy) = world_to_grid(info.x, info.y);
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let gx = cx + dx;
                let gy = cy + dy;
                let x0 = gx as f32 * CELL_RESOLUTION;
                let x1 = (gx + 1) as f32 * CELL_RESOLUTION;
                let y0 = gy as f32 * CELL_RESOLUTION;
                let y1 = (gy + 1) as f32 * CELL_RESOLUTION;
                let px = info.x.clamp(x0, x1);
                let py = info.y.clamp(y0, y1);
                let ddx = info.x - px;
                let ddy = info.y - py;
                if ddx * ddx + ddy * ddy <= r2 {
                    cells.insert((gx, gy));
                }
            }
        }
    }
    cells
}
```

**几何数值例子（格 50cm，R=20cm）**

| 车心位置 | 覆盖格 | 格数 |
|---|---|---|
| 格中心 `(0.25, 0.25)` | `(0,0)` | 1 |
| 靠格边 `(0.45, 0.25)` | `(0,0) (1,0)` | 2 |
| 靠格角 `(0.45, 0.45)` | `(0,0) (1,0) (0,1) (1,1)` | 4 |

- **格中心 → 1 格**：圆盘 `[0.05,0.45]²` 完全在格 `(0,0)` 内（R=0.2 < 半格 0.25），不堵通道——这是选 20cm 而非 30cm 的几何依据。
- **相切不避让的修复**：两车 `(0.25,0.25)` 与 `(0.55,0.25)`（圆心距 30cm，跨边界 0.5），A 标 `(0,0)`，B 圆盘 `[0.35,0.75]` 标 `(0,0)(1,0)` → 两格均被标，巡路绕行。
- **边界归属**：`clamp` 闭区间使恰在格界上的点同时标两侧格（如 x=0.5 标 `(0,·)` 与 `(1,·)`），属无害的 1 格过度标记。

## 三、文件改动清单

改动仅涉及 **5 个文件**；`executor.rs`、`pathfinder.rs` **不改**（动态障碍仍是 `Vec<(i32,i32)>` 格集合，只有其「来源」变了）。

### 3.1 `Src/Config/config.rs`

- `Robot_Config` 结构体增字段（放在 `lidar_baudrate` 后）：

  ```rust
  pub struct Robot_Config {
      pub serial_port: Option<String>,
      pub baudrate: Option<u32>,
      pub car_type: Option<String>,
      pub lidar_port: Option<String>,
      pub lidar_baudrate: Option<u32>,
      pub obstacle_inflation_radius: Option<f32>,   // 新增：他车障碍膨胀半径（米），缺省 0.2
  }
  ```

- `DEFAULT_CONFIG` 常量字符串的 `[Robot]` 段增一行：

  ```toml
  [Robot]
  serial_port = "/dev/myserial"
  baudrate = 115200
  car_type = "X3Plus"
  lidar_port = "/dev/rplidar"
  lidar_baudrate = 230400
  obstacle_inflation_radius = 0.2   # 新增：他车障碍膨胀半径（米，20cm）
  ```

### 3.2 `Src/Config/config.toml`（仓库示例）

`[Robot]` 段增同一行 `obstacle_inflation_radius = 0.2`。

> 提醒：运行时读的是 `.config/config.toml`，部署时需同步；字段缺失时 `robot_bootstrap` 回落默认 0.2。

### 3.3 `Src/bootstrap.rs`（`robot_bootstrap`）

- 读配置（`let r = config.Robot.as_ref();` 附近）：

  ```rust
  let obstacle_inflation_radius = r.and_then(|r| r.obstacle_inflation_radius).unwrap_or(0.2);
  ```

- `Robot::launch(...)` 调用新增该参数；`info!` 日志补打该字段。

### 3.4 `Src/Robot/core/robot.rs`

- `Robot::launch` 签名增参数 `obstacle_inflation_radius: f32`。
- `main_loop` spawn 时透传（f32 Copy，直接捕获）。
- `main_loop` 签名增参数 `obstacle_inflation_radius: f32`。
- auto_tick 分支的调用点（约 476 行）改为：

  ```rust
  let dynamic_obstacles: Vec<(i32, i32)> =
      cluster_to_obstacle_cells(&others, obstacle_inflation_radius).into_iter().collect();
  ```

**补充（实施时发现）：`cluster_to_obstacle_cells` 有第二个调用点 —— `slam_task` 的他车格掩蔽（Task 15 C 节）。**

- `slam_task` 签名增参数 `obstacle_inflation_radius: f32`，spawn 时透传。
- 掩蔽调用同步改为 `cluster_to_obstacle_cells(&others, obstacle_inflation_radius)`。
- **附加收益**：掩蔽足迹与障碍足迹一致（均 20cm）。原「只掩蔽中心格」时，他车车身伸入相邻格的部分会被 LiDAR 标成静态障碍，车走后残留幽灵障碍；同步膨胀后一并掩蔽，消除该问题。
### 3.5 `Src/Robot/core/planning/cluster_obstacles.rs`

- import 改：`use crate::robot::slam::grid::{world_to_grid, CELL_RESOLUTION};`
- 函数签名与实现改为 §2.3 的伪代码（加 `radius: f32` 参数）。
- 文件头 doc 注释更新：footprint 由「1 格不膨胀」改为「圆形膨胀 20cm」。
- 单测重写（见 §五）。

## 四、实施步骤（顺序）

1. `config.rs`：结构体字段 + `DEFAULT_CONFIG`。
2. `config.toml`：示例加字段。
3. `bootstrap.rs`：读配置 → 透传 `Robot::launch`。
4. `robot.rs`：`launch` / `main_loop` 签名 + 捕获变量 + 调用点。
5. `cluster_obstacles.rs`：实现圆形几何膨胀 + 重写单测。
6. `cargo build --bin orion-robot` + `cargo test --lib robot::core::planning::cluster_obstacles` + 全量 `cargo test`。
7. `deploy_robot.sh` 部署实车联调（两车相切场景验证避让）。

## 五、测试

**单测（`cluster_obstacles.rs` 重写）**

| 用例 | 输入 | 期望 |
|---|---|---|
| 空输入 | `&[]` | 空集 |
| 格中心 → 1 格 | `(0.25, 0.25)` | `{(0,0)}` |
| 靠格边 → 2 格 | `(0.45, 0.25)` | `{(0,0),(1,0)}` |
| 靠格角 → 4 格 | `(0.45, 0.45)` | `{(0,0),(1,0),(0,1),(1,1)}` |
| 相切跨边界 | `(0.25,0.25)` 与 `(0.55,0.25)` | 含 `(0,0)` 与 `(1,0)`，无空隙 |
| 去重 | 两车同格 | 集合长度正确 |
| 负坐标 floor | `(-0.1,-0.1)` | 语义不变 |

> 注意：现有 `test_single_floor` / `test_no_inflation` 等按「1 格不膨胀」语义编写，需按新语义重写（例如 `(1.0,1.0)` 恰在格角，新语义下覆盖 4 格）。

**实车联调（后续）**：两车相向/相切场景，确认巡路绕行、无物理碰撞；验证 2m 通道内膨胀不堵路。
