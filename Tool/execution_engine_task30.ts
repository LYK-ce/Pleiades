//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21
//
// task_30 执行引擎程序（run_program）
//
// 用途：按 .github/instructions.md 的「执行引擎使用规则」，串行执行 Task/task_30_sim_world.md
//       的 8 个 step。每个 step 依次 dispatch「规划 → 实现 → 评审」3 个子 agent，
//       评审 FAIL 则再跑一轮，每个 step 最多 3 轮；3 轮不过则中断整条链。
//
// 说明：
// - 不调用 report()，不主动唤醒主 agent；进展写 Workbook/wb_30_sim_world.md
// - 中间产物放 tmp/，程序结束后清理
// - 子 agent 通过 tmp/step.md、tmp/plan.md、tmp/impl.md、tmp/review.json 传递上下文
// - Godot 4.7.2，校验命令：godot --headless --path <工程根> --quit

// ===== 常量 =====
const ROOT = "/vepfs-mlp2/c20250205/240804016/Workspace/Orion";
const TMP = ROOT + "/tmp";
const WORKBOOK = ROOT + "/Workbook/wb_30_sim_world.md";
const WORLD = ROOT + "/orion-simulation-world";

// ===== 8 个 step =====
const STEPS = [
  {
    n: 1,
    title: "整体规划",
    brief: "整体规划（先行）。\n\n产出 orion-simulation-world/COMPONENTS.md，内容必须覆盖：\n- 全场景节点树（World.tscn 为根，含各组件挂载点）\n- 每个组件一个 .tscn 的清单\n- 每个组件的节点树结构\n- 组件之间的挂载关系与数据流\n\n同时创建 Godot 工程骨架（project.godot 及必要目录），使 godot --headless --path orion-simulation-world --quit 能通过。\n\n验收：COMPONENTS.md 存在且覆盖上述四项；工程能被 Godot 4.7.2 加载。"
  },
  {
    n: 2,
    title: "世界组件",
    brief: "世界组件。\n\n- 产出：World.tscn（根场景）\n- 内容：GridMap 体素（程序化生成）+ 一整块地面（StaticBody3D）+ 光照/环境\n- 尺度：体素 0.5m；水平 128×128m；垂直 0~32m；原点 (64,64,0)\n- 世界坐标是 Z-up，Godot 是 Y-up，必须按 task_30 §7.3 只在一个转换函数里做映射\n- 地面用一整块（BoxShape3D 或 WorldBoundaryShape3D），不要 256×256 体素地面\n\n验收：场景可见体素世界；物体被地面挡住，不会无限下坠。"
  },
  {
    n: 3,
    title: "观察者相机",
    brief: "观察者相机。\n\n- 产出：ObserverCamera.tscn\n- 内容：Camera3D + 自由飞行 / 环绕 / 跟随 三种视角\n- 输入用 _unhandled_input，避免抢 UI 输入\n- 相机与仿真完全解耦（不参与任何物理或传感器计算）\n\n验收：能在世界里自由移动视角；转镜头不影响任何物理。"
  },
  {
    n: 4,
    title: "Vehicle 组件",
    brief: "Vehicle 组件。\n\n- 产出：Vehicle.tscn\n- 内容：CharacterBody3D + MeshInstance3D + CollisionShape3D + 装配接口（挂底盘 / 挂传感器）\n- Vehicle 本身就是 CharacterBody3D（不是 Node3D 容器）\n- 对外统一接口：apply_motion / sample_sensors / get_pose\n\n验收：能实例化一台「空车」；底盘与传感器可动态挂载。"
  },
  {
    n: 5,
    title: "差速底盘 diff_drive_ugv",
    brief: "差速底盘 diff_drive_ugv。\n\n- 产出：chassis/DiffDrive.tscn\n- 内容：MOTION_MODE_GROUNDED + 差速运动学 + 碰撞形状\n- 运动模型自己写（速度环在世界侧）；接收与真机相同的控制接口（运动状态 + 速度档位）\n\n验收：收到运动指令能前进/后退/转向；撞墙会停。"
  },
  {
    n: 6,
    title: "WebSocketManager",
    brief: "WebSocketManager。\n\n- 产出：WebSocketManager.tscn\n- 内容：监听端口、accept 连接、维护 peer_id → vehicle 与 连接 → (peer_id, 角色) 两张表、chassis_hello / sensor_hello 装配、manifest 就绪、assembly_ready、断开清理\n- 消息 schema 见 task_30 §6.2\n\n验收：一个 sim 能连上并完成装配，收到 assembly_ready（可用简易 ws 客户端测试）。"
  },
  {
    n: 7,
    title: "四旋翼底盘 quadrotor_uav",
    brief: "四旋翼底盘 quadrotor_uav。\n\n- 产出：chassis/Quadrotor.tscn\n- 内容：MOTION_MODE_FLOATING + 四旋翼运动学（平移/爬升/偏航）\n\n验收：无重力悬停；能按指令升降与平移。"
  },
  {
    n: 8,
    title: "传感器 lidar_2d",
    brief: "传感器 lidar_2d。\n\n- 产出：sensors/Lidar2D.tscn\n- 内容：从挂载点出发的水平面射线投射（360 条/圈，10Hz），输出 LaserScan 格式（task_30 §7.4）\n- 不复刻真机协议/噪声，只模拟「周围一圈距离」\n\n验收：一圈扫描能正确反映周围障碍的距离。"
  }
];

// ===== 提示词（静态字面量） =====
const PLAN_PROMPT = "你是「规划」子 agent。工作目录：/vepfs-mlp2/c20250205/240804016/Workspace/Orion\n\n请按顺序读：\n1. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/step.md —— 当前 step 的要求（含编号、标题、内容、验收标准、轮次）\n2. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/Task/task_30_sim_world.md —— 整个任务的设计，必须完整读（§三 核心模型、§四 架构/组件分解、§五 装配流程、§六 消息 schema、§七 已定决策）\n3. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/prev_issues.md —— 上一轮评审的问题清单；若内容不是「(无)」，本轮计划必须逐条解决\n4. 需要时读 /vepfs-mlp2/c20250205/240804016/Workspace/Orion/orion-simulation-world/ 下已有文件\n\n产出该 step 的实施计划，写入：\n/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/plan.md\n\n计划必须包含：\n- 要创建/修改的文件清单（完整路径）\n- 每个文件的职责与关键内容\n- 节点树结构 / 数据结构 / 接口签名\n- 与已有组件的挂载关系\n- 本 step 的验收自检方法（含 Godot 校验命令）\n- 风险与注意点\n\n约束：\n- Godot 4.7.2，可执行文件 godot 已在 PATH 中\n- 工程根目录：/vepfs-mlp2/c20250205/240804016/Workspace/Orion/orion-simulation-world/\n- 校验命令示例：godot --headless --path <工程根> --quit\n- 严格遵守 task_30 的已定决策，不要改设计\n- 只做本 step 范围内的事\n\n最后用 3-5 行总结计划要点。";

const IMPL_PROMPT = "你是「实现」子 agent。工作目录：/vepfs-mlp2/c20250205/240804016/Workspace/Orion\n\n请先读：\n1. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/step.md（当前 step 要求）\n2. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/plan.md（本轮规划）\n3. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/prev_issues.md（上一轮评审问题，若存在必须修复）\n4. 需要时查阅 /vepfs-mlp2/c20250205/240804016/Workspace/Orion/Task/task_30_sim_world.md\n\n然后严格按 plan.md 实施，改动落到：\n/vepfs-mlp2/c20250205/240804016/Workspace/Orion/orion-simulation-world/\n\n要求：\n- Godot 4.7.2；实现后用 godot --headless --path /vepfs-mlp2/c20250205/240804016/Workspace/Orion/orion-simulation-world --quit 校验工程能加载，stderr 不应有脚本/场景错误\n- 新建的代码文件遵循项目归属注释规范（Presented by KeJi / Created Date / Modified Date）\n- 不要执行 git commit / git add\n- 不要修改 orion-simulation-world/ 之外的任何文件（除非 plan.md 明确要求）\n- 完成后把「改了什么、怎么验证、遗留什么」写入 /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/impl.md\n\n最后用 3-5 行总结。";

const REVIEW_PROMPT = "你是「评审」子 agent。工作目录：/vepfs-mlp2/c20250205/240804016/Workspace/Orion\n\n请先读：\n1. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/step.md（本 step 要求与验收标准）\n2. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/plan.md（规划）\n3. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/impl.md（实现说明）\n4. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/prev_issues.md（上一轮问题）\n5. /vepfs-mlp2/c20250205/240804016/Workspace/Orion/Task/task_30_sim_world.md（设计，核对是否偏离）\n\n然后独立评审实际代码（必须打开文件核对，不能只看 impl.md）：\n- 是否满足 step.md 的验收标准\n- 是否偏离 task_30 的已定决策（核心模型、组件分解、消息 schema、坐标转换、尺度、性能）\n- 代码质量与项目规范（归属注释、命名、错误处理）\n- 实跑校验：godot --headless --path /vepfs-mlp2/c20250205/240804016/Workspace/Orion/orion-simulation-world --quit\n- 上一轮问题是否真的修好\n\n把结论写入：\n/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/review.json\n\n格式严格为（只输出这个 JSON，不要多余文字）：\n{\"verdict\":\"PASS\",\"issues\":[]}\n或\n{\"verdict\":\"FAIL\",\"issues\":[\"问题1\",\"问题2\"]}\n\n并在你的最终回复第一行输出：VERDICT: PASS 或 VERDICT: FAIL\n\n判定：\n- 验收标准全部满足、无阻塞性缺陷 → PASS\n- 有阻塞性缺陷（跑不起来、偏离设计、上一轮问题未修）→ FAIL，并在 issues 里写清楚\n- 小改进建议可写进 issues，但不影响验收时仍判 PASS";

// ===== 工具函数 =====
async function readOr(path, fallback) {
  try {
    if (await exists(path)) {
      return await readTextFile(path);
    }
  } catch (e) {
    return fallback;
  }
  return fallback;
}

function stepBrief(step, round) {
  return "# Step " + step.n + " — " + step.title + "\n\n" +
    "> 执行轮次：第 " + round + " / 3 轮\n" +
    "> 任务文档：/vepfs-mlp2/c20250205/240804016/Workspace/Orion/Task/task_30_sim_world.md\n" +
    "> 工程根：/vepfs-mlp2/c20250205/240804016/Workspace/Orion/orion-simulation-world/\n\n" +
    "## 本 step 要求\n\n" + step.brief + "\n\n" +
    "## 通用约束\n\n" +
    "- Godot 4.7.2，godot 在 PATH 中\n" +
    "- 必须遵循 task_30 的已定决策\n" +
    "- 只做本 step 范围内的事\n" +
    "- 不要 git commit / git add\n";
}

function parseVerdict(text, jsonText) {
  if (jsonText && jsonText.trim()) {
    try {
      const o = JSON.parse(jsonText);
      if (o && typeof o.verdict === "string") {
        const pass = o.verdict.toUpperCase() === "PASS";
        const issues = Array.isArray(o.issues) ? o.issues.join("\n") : "";
        return { pass: pass, issues: pass ? "" : (issues || "评审判 FAIL（issues 为空）") };
      }
    } catch (e) {
      // fallthrough
    }
  }
  const m = /VERDICT:\s*(PASS|FAIL)/i.exec(text || "");
  if (m) {
    const pass = m[1].toUpperCase() === "PASS";
    return { pass: pass, issues: pass ? "" : ("评审文本：\n" + (text || "").slice(0, 3000)) };
  }
  return { pass: false, issues: "评审未返回可解析结论。原始回复：\n" + (text || "").slice(0, 2000) };
}

// ===== 主流程 =====
await process(["mkdir", "-p", TMP]);

let wb = await readOr(WORKBOOK, "");
if (!wb.trim()) {
  wb = "# wb_30_sim_world — task_30 工作记录\n\n" +
    "> Presented by KeJi\n" +
    "> Created Date ： 2026-10-21\n" +
    "> Modified Date ： 2026-10-21\n" +
    "> 任务：Task/task_30_sim_world.md（sim 接入 Godot 模拟世界）\n\n---\n";
  await writeTextFile(WORKBOOK, wb);
}

let abortReason = "";
let stopAll = false;

for (const step of STEPS) {
  if (stopAll) break;

  let passed = false;
  let rounds = 0;
  let lastIssues = "";
  let planSummary = "";

  for (let round = 1; round <= 3; round++) {
    rounds = round;

    await writeTextFile(TMP + "/step.md", stepBrief(step, round));
    await writeTextFile(TMP + "/prev_issues.md", lastIssues || "(无)");
    await process(["rm", "-f", TMP + "/plan.md", TMP + "/impl.md", TMP + "/review.json"]);

    try {
      planSummary = await dispatchsubagent(PLAN_PROMPT);
    } catch (e) {
      lastIssues = "规划子 agent 失败：" + String(e);
      continue;
    }

    try {
      await dispatchsubagent(IMPL_PROMPT);
    } catch (e) {
      lastIssues = "实现子 agent 失败：" + String(e);
      continue;
    }

    let reviewText = "";
    try {
      reviewText = await dispatchsubagent(REVIEW_PROMPT);
    } catch (e) {
      lastIssues = "评审子 agent 失败：" + String(e);
      continue;
    }

    const reviewJson = await readOr(TMP + "/review.json", "");
    const v = parseVerdict(reviewText, reviewJson);

    if (v.pass) {
      passed = true;
      lastIssues = "";
      break;
    }
    lastIssues = v.issues || "评审判 FAIL（未给出问题清单）";
  }

  // 写 workbook（每个 step 结束后都写）
  const oneLine = (planSummary || "").replace(/\s+/g, " ").slice(0, 800);
  let rec = "\n## Step " + step.n + " — " + step.title + "\n";
  rec += "- 轮次：" + rounds + " / 3\n";
  rec += "- 结果：" + (passed ? "PASS" : "FAIL（已中断）") + "\n";
  rec += "- 规划要点：" + oneLine + "\n";
  if (!passed) {
    rec += "- 3 轮未通过的问题：\n" + lastIssues.slice(0, 5000) + "\n";
  }
  wb = await readOr(WORKBOOK, wb);
  await writeTextFile(WORKBOOK, wb + rec);

  if (!passed) {
    abortReason = "Step " + step.n + "（" + step.title + "）3 轮未通过";
    stopAll = true;
  }
}

// 清理 tmp（无论成功还是中断）
await process(["rm", "-rf", TMP]);
await process(["mkdir", "-p", TMP]);

return abortReason ? "ABORTED: " + abortReason : "ALL_STEPS_DONE";
