# 行人 WS 客户端（`orion-simulation-world/tool/pedestrian/`）

> Presented by KeJi
> Created Date ： 2026-10-25
> Modified Date ： 2026-10-25
>
> 对应 `Task/task_35_pedestrian.md` §二 Step 4、`orion-simulation-world/COMPONENTS.md` §8.1。
> 这三条 `.py` 是**世界外**的驱动脚本：Python 只发控制、只读位姿，不读地图、不连 P2P、
> 不认识任何 Pleiades 概念。人不进 `pleiades-sim`，也不经过任何决策链。

`tool/` 目录带一个 0 字节的 `.gdignore`：Godot 会**完全跳过**本目录（否则编辑器与
`--quit` 会把 `.py` / CSV / 日志当未知资源扫描，`.godot/` 缓存里出现噪音）。因此本目录
里的文件对世界**没有任何副作用**。

---

## 1. 文件

| 文件 | 作用 |
|---|---|
| `pedestrian_client.py` | `PedestrianClient`：hello → ready → 20~50 Hz 持续 `motion_cmd{v, dir}` → `pose()` → 退出发 `v=0`；本身也能当冒烟工具跑 |
| `patrol.py` | 示例行为：路点 → 方位角 → 恒定 `v` 的折线/矩形巡逻；`--csv` 采样、`--linger` 验证 300 ms 超时停车 |
| `ws_min.py` | 标准库 RFC6455 最小客户端（文本帧 + 掩码 + Ping→Pong + Close + 分片），`websockets` 未安装时的兜底 |
| `README.md` | 本文件 |

---

## 2. 快速开始

```bash
cd /vepfs-mlp2/c20250205/240804016/Workspace/Orion

# 1) 起世界（headless 就够；真渲染/截图是 Step 5 的事）
#    不要加 --fixed-fps：那会关掉实时同步，世界的 300 ms 超时在墙钟上只剩 ~120 ms
godot --headless --max-fps 60 --path orion-simulation-world &

# 2) 等这一行，再起客户端
#    [ws] listening on port 9999 (links=0 vehicles=0)
python3 orion-simulation-world/tool/pedestrian/patrol.py --duration 10 --speed 1.0 \
  --peer-id patrol-01 --csv tmp/task35/pose.csv

# 3) 人停下、连接关掉；世界日志应有
#    [ws] pedestrian spawned peer='patrol-01' id=1 node=patrol-01 spawn=(64.00, 56.00) yaw=0.00
#    [ws] pedestrian removed peer='patrol-01'
```

最简冒烟（不需要 `patrol.py`）：

```bash
python3 orion-simulation-world/tool/pedestrian/pedestrian_client.py \
  --v 1.0 --dir 1.5707963 --duration 3 --peer-id human-01 --spawn "70,60"
# 期望：y 增大、yaw 在 ~0.3 s 内收敛到 90.0°，退出码 0
```

---

## 3. 依赖

- **Python ≥ 3.8**（本机实测 3.13.2）。
- `websockets`（可选，实测 15.0.1）。装了就走它，没装自动回落标准库实现：

  ```bash
  pip install websockets        # 可选：更稳的连接管理与帧校验
  ```

  回落时**一定**打印一行，不会静默：

  ```
  [warn] websockets 不可用（No module named 'websockets'），改用标准库 RFC6455 客户端（pip install websockets 可更稳）
  [client] engine=stdlib (RFC6455 自研)
  ```

  也可以强制选：`--engine auto|websockets|stdlib`。`--engine websockets` 而没装 =
  直接 `[error] websockets 不可用（…）：请 pip install websockets …` + 退出码 2，不猜不降级。
- `wss://`（TLS）**只有** `websockets` 那条路支持；标准库实现只做 `ws://`。

---

## 4. 协议（客户端唯一需要知道的世界侧事实）

### 4.1 上行（只发**文本**帧；二进制会被世界忽略并 warn）

```jsonc
// 首包（必须第一条）
{"msg":"chassis_hello","peer_id":"patrol-01","chassis":"pedestrian","kind":"pedestrian",
 "params":{},"manifest":[],"spawn":{"x":64.0,"y":56.0,"z":0.0,"yaw":0.0}}   // spawn 可选

// 持续控制（20~50 Hz，seq 单调递增；dir 省略 = 只改速度、保持朝向）
{"msg":"motion_cmd","seq":123,"v":1.0,"dir":1.5707963267948966}

// 停车（退出 / 异常 / Ctrl-C 各发一次；不带 dir）
{"msg":"motion_cmd","seq":124,"v":0.0}
```

| 字段 | 语义 |
|---|---|
| `chassis` | 必须是 `"pedestrian"`（世界按名字查 `ChassisCatalog`） |
| `kind` | 必须是 `"pedestrian"`，否则世界按 robot 处理（这里没有 `Vehicle.tscn` 的排位，会走错分支） |
| `params` | 可空；`{max_speed, max_turn_rate}` 可覆盖（`max_speed` 上限 1.26 m/s、`max_turn_rate` 单位**度/秒**，默认 180） |
| `spawn` | `x,y ∈ [0,128)`、`z ∈ [0,2.0]`、与他人水平距 ≥ 1.0 m、不在 `World.OBSTACLES` 内（外扩 0.3 m） |
| `v` | m/s，世界 clamp **0~1.26**（行人**不倒着走**：负值一律当 0） |
| `dir` | **世界系弧度**，`0 = +X 东`，逆时针为正；世界内部 `rotation.y = -dir`，客户端**不要**再做取反 |
| 300 ms 无新 `motion_cmd` | 世界自动 `SetCommand({v:0})`（只停不断连），日志一行 `[ws] pedestrian idle peer='…'` |

### 4.2 下行（world → client）

```jsonc
{"msg":"chassis_ready","peer_id":"patrol-01","kind":"pedestrian","pedestrian_id":3,
 "spawn":{"x":64.0,"y":56.0,"z":0.0,"yaw":0.0},"world_time":12.34}
{"msg":"pose","world_time":12.4,"x":64.3,"y":56.02,"z":0.0,"yaw":0.12,"vx":1.0,"vy":0.0}  // 10 Hz
{"msg":"assembly_ready","peer_id":"patrol-01","slots":0}   // manifest=[] 时立刻来，客户端忽略
```

- `pose` 只发给**已握手**的底盘连接，10.0 Hz；`yaw` 是**世界 yaw**（弧度，与 `dir` 同口径）。
- 客户端把未知 `msg`（`assembly_ready` / `sensor_ready` / 未来新增）当噪音，`-v` 才打印；
  握手阶段也不会假设「下一帧就是 ready」——一直在窗口内等到 `chassis_ready` 为止。
- 世界每 10 s 发一次 WS Ping：`websockets` 自动回 Pong；`ws_min` 显式回 Pong。

### 4.3 `spawn` 不是「拒绝」而是「回退」

世界对非法 `spawn` 只 `push_warning` 并**回退到默认点 (64, 56)**，连接照旧建立。
所以客户端必须比对 `chassis_ready.spawn` 的回执：

```
[error] spawn 被拒：请求 (78.50, 78.50)，世界实际落在 (64.00, 56.00)（见世界日志 WARNING）   # 退出码 5
```

加 `--allow-spawn-fallback` 降级为 `[warn] …（--allow-spawn-fallback，继续）` 并继续跑。

---

## 5. `patrol.py` 参数

| 参数 | 默认 | 说明 |
|---|---|---|
| `--speed` | `1.0` | 线速度 m/s（世界 clamp 0~1.26） |
| `--loop` | 关 | 走完最后一个点回到第一个；不然后停在最后一个点（`v=0`） |
| `--duration` | `20.0` | 从 ready 起算的总时长（秒） |
| `--peer-id` | `patrol-01` | 一个 id 一个进程（同 id 重连会摘掉旧 avatar） |
| `--spawn` | 不带 | `"x,y"` / `"x,y,z"` / `"x,y,z,yaw_度"`；不带 = 世界默认点 (64,56) |
| `--waypoints` | `64,56;76,56;76,66;64,66` | 世界坐标（米）；默认矩形离最近的 `OBSTACLES` 净空 12.17 m |
| `--send-hz` | `30` | **硬限 20~50**，越界直接报错退出（世界 300 ms 超时） |
| `--arrive-tol` | `0.30` | 到点判定（米） |
| `--turn-threshold-deg` | `20` | 朝向误差 > 该值 → `v=0` **原地转**，转过来再走（不会画大弧） |
| `--csv` | 不带 | pose 采样（UTF-8，表头 `t_wall,world_time,x,y,z,yaw_deg,target_dir_deg,v,wp`） |
| `--linger` | `0.0` | 驱动结束后**停发但保持连接**的秒数：量「多久才停」「之后漂不漂」 |
| `--url` | `ws://127.0.0.1:9999` | |
| `--ready-timeout` | `8` | 等 `chassis_ready` 的秒数 |
| `--engine` | `auto` | `auto` / `websockets` / `stdlib` |
| `--allow-spawn-fallback` | 关 | `spawn` 被回退时降级为 WARNING 继续 |
| `--quiet` | 关 | 不打每秒一行的进度（`start` / `done` / `verify` 仍有） |

每帧决策（刻意最简单，**不含任何避障**）：

```
方位角 target_dir = atan2(wp.y - y, wp.x - x)
误差 err = wrap_pi(target_dir - yaw)
|err| > 阈值 -> drive(0, target_dir)        # 原地转
否则         -> drive(speed, target_dir)   # 边走边微调
到点（< arrive_tol）-> 下一个路点（--loop 取模；否则 v=0 站住）
```

`--linger` 的判据（`--duration 3 --linger 1.5` 实测 `last_move_at=0.34 s`、`drift=0.000 m`）：

```
[verify] linger=1.5s last_move_at=0.34s drift=0.000m -> PASS
```

判据：`last_move_at ≤ 0.45 s`（世界 300 ms 超时 + 10 Hz pose 的观测粒度）且
`0.45 s 之后到结束的位移 < 0.01 m`。不通过 → 退出码 6。

---

## 6. `pedestrian_client.py` 参数（冒烟工具）

`--v` / `--dir`（弧度）/ `--duration` / `--url` / `--peer-id` / `--spawn` / `--send-hz` /
`--ready-timeout` / `--engine` / `--allow-spawn-fallback` / `-v`（打印每条收发）。

行为：connect → 按 `--duration` 以 `send_hz` 发 `{v, dir}` → 每秒打印一行 `[pose]` →
`finally` 里发 `v=0` 再关连接。

作为库用时：

```python
from pedestrian_client import PedestrianClient

with PedestrianClient(peer_id="human-01", spawn={"x": 64.0, "y": 56.0}) as client:
    client.drive(1.0, 0.0)          # 1 m/s 朝 +X
    while True:
        client.tick()               # 收 pose + 到点补发 motion_cmd
        pose = client.pose()        # 最近一条 pose 的副本；没收到过 = None
        client.sleep_to_next()      # 睡到下一个发送节拍
```

`close()`（含 `with` 退出、`Ctrl-C`、异常）一定先发一次 `v=0`，且**幂等**。

---

## 7. 退出码

| 码 | 含义 |
|---|---|
| `0` | 正常（含 `--allow-spawn-fallback` 的降级继续） |
| `2` | 连接失败（端口没开 / URL 错 / 世界没起 / `websockets` 指定但未安装） |
| `3` | `--ready-timeout` 内没等到 `chassis_ready` |
| `4` | 协议或传输异常（`kind` 不是 pedestrian、世界主动关连接、参数非法） |
| `5` | `spawn` 被世界回退（未给 `--allow-spawn-fallback`） |
| `6` | `--linger` 停车判据不过（`last_move_at` 太大或停后仍漂移） |
| `130` | `Ctrl-C` / `SIGTERM`（先发 `v=0` 再退出） |

失败一律是**一行可读的 `[error] …`**，不是 Python traceback。

---

## 8. 排障

**端口被占（世界只 warn 不报错，客户端 `Connection refused`）**

```bash
ss -ltnp | grep 9999          # 谁占着
# 上一轮 world 没退干净：
pkill -f "godot --headless --path orion-simulation-world"
```

**`chassis_ready` 一直不来（退出码 3）** — 世界没起（先看 `listening on port 9999`）、
`--url` 端口不对，或同一 `peer_id` 被另一个客户端占着（世界重连会先摘旧 avatar，
两边互踢）。一个 `peer_id` 只跑一个进程。

**「人不动」排查顺序**

1. `pose` 有没有来？没有 → 连接/握手问题（看退出码）。有 → 继续。
2. 有没有在发 `motion_cmd`？`-v` 能看 `[client] send#N v=… dir=…`，20~50 Hz。
3. 世界日志里 `[ws] pedestrian idle peer='…'（300 ms 无新指令）-> v=0` 说明**发帧断了**
   （sleep 卡住、GIL 长阻塞、Ctrl-C 收尾路径提前 return）。这个脚本的节拍是
   `time.monotonic()` 驱动的，落后超过 2 个周期会重置节拍而不是补发。
4. `dir` 反了？判据：`dir=+π/2`（90°）应当让 `y` **增大**。若 `x` 增大而 `y` 不动，
   说明有人在客户端多做了一次取反——世界的 `rotation.y = -dir` 已经做过了。
5. 撞在 `OBSTACLES` 上：本示例**不做避障**，改 `--waypoints` 前先看
   `src/world/World.gd` 的 `OBSTACLES`（默认矩形已留 12.17 m 净空）。

**「腿不动 = 人没走」是错觉**：`speed_scale = v / 1.26`，`v=0` 时人物定格；
glb 缺失只会 `push_warning`（人照样位移）。headless 下动画照播，只是没画面。

**`spawn` 落在障碍/越界/与他人重叠** → 世界回退默认点（不报错）。
本客户端比对回执并给退出码 5；真要看原因，看世界日志的 `WARNING: [ws] peer='…' spawn …`。

**时基**：验收/量距离**不要**用 `--fixed-fps`（它会关掉实时同步，世界比墙钟快数倍，
300 ms 超时在墙钟上只剩 ~120 ms）。用 `--max-fps 60`。

**截图/录像**（属 Step 5，不是本目录）：`--headless` 是 dummy 渲染器，出不了图；
真渲染要 `DISPLAY=:99 godot --rendering-driver opengl3 --audio-driver Dummy --path orion-simulation-world`
（**不加** `--headless`）。

---

## 9. 归属与命名

- 每个 `.py` 头部：`Presented by KeJi` / `Created Date` / `Modified Date` + 一句话职责。
- 命名沿用仓库口径：模块函数 `snake_case`、类 `PascalCase`、常量 `UPPER_SNAKE`
  （GDScript 侧的 `Pascal_Snake` 函数名**不**带进 Python）；协议字段名保持英文原文。
- 注释中文，与 `orion-simulation-world` 其余部分一致。
