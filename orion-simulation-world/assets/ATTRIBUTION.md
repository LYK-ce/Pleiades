# assets/ATTRIBUTION.md — 第三方资产署名与改动说明

> Presented by KeJi
> Created Date ： 2026-10-29
> Modified Date ： 2026-10-29
>
> 本文件只记**需要署名（CC BY）**的资产。CC0 来源的逐条台账仍在 `README.md`（无归属义务，仅留证）。
> 目前只有一件：`assets/chassis/uav_quadrotor.glb`（task_40 Step 5 换掉的四旋翼外观）。

---

## 1. 署名（CC BY 3.0 要求在再分发/展示时给出）

| 项 | 值 |
|---|---|
| 作品标题 | **Drone** |
| 作者 | **NateGazzard** |
| 来源页面 | <https://poly.pizza/m/DNbUoMtG3H> |
| 直下链接（当时用的） | <https://static.poly.pizza/4eb88feb-cb2d-46c3-980d-3e704868361c.glb> |
| 许可 | **CC BY 3.0 Unported**（<https://creativecommons.org/licenses/by/3.0/>） |
| 许可全文（本仓库留存） | `assets/chassis/LICENSE_uav_quadrotor.txt`（19 467 B，取自 <https://creativecommons.org/licenses/by/3.0/legalcode.txt>） |
| 抓取日期 | **2026-10-29**（直连，未走代理） |
| 源文件字节 / sha256 | `259 676` / `bc9f0d765e0fdb838a9baf26b472f955a11bb2509a3337e918054241b134638d` |
| 落地文件字节 / sha256 | `299 376` / `7aa77d6b21213819cef03c4531b7a810bcf8c962be83e7d7828f3238443daba9` |
| 落地位置 | `assets/chassis/uav_quadrotor.glb`（+ `.glb.import` + Godot 抽出的 `uav_quadrotor_ColorPalette.png` 及其 `.import`） |
| 源页面元数据 | 标题 `Drone - Free 3D Model By NateGazzard - Poly Pizza`；上传 `2023-12-31`；`Tris: 4564`；页面内许可链接 = `creativecommons.org/licenses/by/3.0` |

**一行署名（论文/演示/README 引用用这行）**：

```
"Drone" by NateGazzard — https://poly.pizza/m/DNbUoMtG3H — CC BY 3.0
```

改动后再分发时，须保留上面的作者与来源、标注本仓库做过的改动（见 §3），并保持 CC BY 3.0 的许可声明。
本仓库对模型的改动**不改变原许可**（不额外施加限制）。

---

## 2. 为什么不是 CC0（选型证据，§6.3 的顺序逐条走过）

| 顺位 | 来源 | 动作 | 结果 |
|---|---|---|---|
| 1 | Poly Pizza（搜 `quadcopter`） | 抓 17 条模型页，按许可过滤 | 仅 1 条 CC0（`EQJ2MECUbx`）→ 拉下来渲染确认**是直升机**（单主旋翼 + 尾桨），不是四旋翼 → 排除 |
| 1 | Poly Pizza（搜 `drone`） | 抓 21 条模型页 | **0 条 CC0**（全 CC BY 3.0）→ 无 CC0 四旋翼可挑 |
| 2 | Sketchfab | 下载需登录 token | 无头环境不可取 → 不走 |
| 3 | Kenney | `curl` 直连 **403**，`-x http://127.0.0.1:7897` 亦 **403** | 拿不到；且其无人机是风格化低模，文档自定位「仅兜底」 |
| 4 | Blender 自制 | 备选 | 第 4 顺位才用；本轮第 1 顺位已有合规候选，故未自制 |

⇒ 采用 **CC BY 3.0**：`Task/task_40_ugv_motion.md` §八 #7 已定「**CC0 优先；CC-BY 可接受（须 `LICENSE` + `ATTRIBUTION.md` + 台账 + 说明改动）**」。
排除项已逐条确认：**非** CC BY-SA、**非** CC BY-NC、**非** personal-only，也**不是**任何品牌机型（DJI 等）的翻制——它是通用四旋翼构型，无商标字样。

---

## 3. 改动说明（CC BY 3.0 §4(b)：必须指明对原作做了哪些改动）

处理脚本：`tmp/task40/tools/uav_norm.py`（Blender 4.5.14 LTS 批处理；脚本与 `.blend` 中间件只留在 `tmp/`，不入库）。

| # | 改动 | 细节 |
|---|---|---|
| 1 | 清除非几何节点 | 删除相机 / 灯光等非 `MESH` 对象；断父保世界变换后 `transform_apply` |
| 2 | 朝向 | 绕 Z 轴 **+90°**（机头 −Y → **+X**，与 UGV / human 资产同一套「朝 +X」口径） |
| 3 | 缩放（**非等比**） | `k = (0.8191, 1.1158, 0.6744)`，把原始 span `0.7813 × 0.5736 × 0.2076` 压到目标 AABB。缩放后（Glb Y-up 导出、Godot 系）**`0.64 × 0.14 × 0.64`**，与既有 UAV 碰撞体口径一致 ⇒ 动力学零改动 |
| 4 | 位置 | 足迹居中、**底面 `y = 0`**（原点 = 底面中心，与其它资产一致） |
| 5 | 材质 | `Body` 追加第 2 个材质 **`UAV_NoseRed`**（base color `0.85,0.16,0.12`，roughness `0.55`），把「正面朝前（`normal.x > 0.5`）且靠前（旋转后 `x ≥ x_max − 0.075`）」的 **194 个面**改用它 —— 这是机头红标记（自检 `M4-uav-nose-plus-x` 的 by-mat 通道靠它），不是新增零件 |
| 6 | 导出 | GLB / `export_apply=True` / `export_yup=True` / 关相机·灯光·动画·蒙皮·morph / **关 Draco** |
| 7 | **未改** | 贴图（原 128×128 `ColorPalette` 调色板贴图原样）· 几何拓扑 · 面数（**4 564 tris，与源一致**）· 顶点位置以外的 UV/法线数据 |

**比例失真的已知代价**：横向非等比（`0.819 / 1.116 / 0.674`）让机身略「短粗」、螺旋桨略扁，四旋翼特征（机架 / 4 臂 / 4 电机舱 / 4 桨 / 起落架 / 机腹云台舱）完整可辨，见 `tmp/task40/shots/uav_00_q34.png`。

---

## 4. 使用边界

- 只用于 `orion-simulation-world` 里 `Vehicle/ModelUAV` 的**外观**（`ApplyChassis("quadrotor_uav")` 时可见），不参与碰撞与动力学：碰撞盒仍是 `Vehicle/CollisionShape3D` 的 `0.40 × 0.12 × 0.40`。
- 本项目为非商用研究/仿真；如后续用于对外发布，请复核 §1 的署名是否随产物一起给出。
