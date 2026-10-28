//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27
//
// task_36 执行引擎（与 run_program 内联版本一致）
//
// 用途：按 .github/instructions.md 的「执行引擎使用规则」，串行执行
//       Task/task_36_behavior_tree.md 的 Step 1~8。
//       每个 step 依次 dispatch「规划 → 实现 → 评审」3 个子 agent；评审 FAIL 再跑一轮；
//       每 step 最多 3 轮，3 轮不过则中断整条链，等人类处理。
//
// 设计：本文件只保留流程；step 定义与三个提示词外置在
//       Tool/execution_engine_task36.config.json（改 step 不必改流程）。
//
// 说明：
// - 不调用 report()；进展写 Workbook/wb_36_behavior_tree.md
// - 中间产物只放 tmp/task36/（结束清理）
// - 每一步开始前刷新 git 基线，评审只比对“本步新增改动”

const ROOT = "/vepfs-mlp2/c20250205/240804016/Workspace/Orion";
const CFG_PATH = ROOT + "/Tool/execution_engine_task36.config.json";

const CFG = JSON.parse(await readTextFile(CFG_PATH));
const STEPS = CFG.steps;
const TMP = ROOT + "/" + CFG.tmp;
const WORKBOOK = ROOT + "/" + CFG.workbook;
const TASK = ROOT + "/" + CFG.task;
const PLAN_PROMPT = (CFG.prompts.plan || []).join("\n");
const IMPL_PROMPT = (CFG.prompts.impl || []).join("\n");
const REVIEW_PROMPT = (CFG.prompts.review || []).join("\n");

// 只跑指定 step 起（全量跑改成 0）；RUN_TAG 用于在 workbook 里区分第几次运行
const FROM_STEP = 8;
const RUN_TAG = "重跑 Step 8（第 2 次）";
const RUN_STEPS = STEPS.filter((s) => Number(s.n || 0) >= FROM_STEP);

async function readOr(path, fallback) {
  try {
    if (await exists(path)) return await readTextFile(path);
  } catch (e) {
    return fallback;
  }
  return fallback;
}

function nowIso() {
  try {
    return new Date().toISOString().replace("T", " ").slice(0, 19) + "Z";
  } catch (e) {
    return "(未知时间)";
  }
}

function allowedList(step) {
  const arr = Array.isArray(step.allowed) ? step.allowed : [];
  return arr.map((p) => "- " + p).join("\n");
}

function stepBrief(step, round) {
  const brief = (Array.isArray(step.brief) ? step.brief : []).join("\n");
  return "# Step " + step.n + " — " + step.title + "\n\n" +
    "> 执行轮次：第 " + round + " / 3 轮\n" +
    "> 对应任务文档章节：" + step.doc + "\n" +
    "> 改动范围：" + step.side + "\n" +
    "> 任务文档：" + TASK + "\n\n" +
    "## 本 step 要求（细节以任务文档 " + step.doc + " 为准）\n\n" + brief + "\n\n" +
    "## 本 step 允许改动的路径（越界即 FAIL）\n\n" + allowedList(step) + "\n\n" +
    "## 全局硬约束\n\n" +
    "- 中间产物只写 " + TMP + "/\n" +
    "- **禁止**修改：Task/、.github/、Cargo.toml / Cargo.lock（根）\n" +
    "- **禁止** git add / git commit\n" +
    "- 除 Step 8 外，不得启动任何长驻进程（不起 pleiades-sim、不起 Godot、不装软件）\n" +
    "- Step 8 允许 headless 启动 pleiades-sim 与测试脚本\n" +
    "- 行为回归红线：Step 1 必须“行为零变化”；Step 2 必须“不动设备端一行、不新增依赖”\n";
}

async function dispatchWithRetry(prompt, tries) {
  const n = tries || 2;
  let lastErr = null;
  for (let i = 0; i < n; i++) {
    try {
      const text = await dispatchsubagent(prompt);
      return { ok: true, text: text };
    } catch (e) {
      lastErr = e;
    }
  }
  return { ok: false, err: lastErr };
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
    } catch (e) {}
  }
  const m = /VERDICT:\s*(PASS|FAIL)/i.exec(text || "");
  if (m) {
    const pass = m[1].toUpperCase() === "PASS";
    return { pass: pass, issues: pass ? "" : ("评审文本：\n" + (text || "").slice(0, 3000)) };
  }
  return { pass: false, issues: "评审未返回可解析结论。原始回复：\n" + (text || "").slice(0, 2000) };
}

async function gitStatus() {
  try {
    const gs = await process(["bash", "-lc", "cd " + ROOT + " && git status --porcelain --untracked-files=all"]);
    return (gs.stdout || "") + (gs.stderr || "");
  } catch (e) {
    return "(git status 失败: " + String(e) + ")";
  }
}

await process(["mkdir", "-p", TMP]);
if (!(await exists(TASK))) return "ABORTED: 任务文档不存在 " + TASK;

// 全局基线（收尾时给人类看）
await writeTextFile(TMP + "/git_baseline_global.txt", await gitStatus());

let wb = await readOr(WORKBOOK, "");
if (!wb.trim()) {
  wb = "# wb_36_behavior_tree — task_36 工作记录\n\n" +
    "> Presented by KeJi\n" +
    "> Created Date ： 2026-10-27\n" +
    "> Modified Date ： 2026-10-27\n" +
    "> 任务：Task/task_36_behavior_tree.md（队列 + 行为树，Step 1~8）\n";
}
await writeTextFile(WORKBOOK, wb + "\n## 执行引擎启动\n" +
  "- 开始时间：" + nowIso() + "\n" +
  "- 引擎：Tool/execution_engine_task36.ts + Tool/execution_engine_task36.config.json\n" +
  "- 运行标记：" + RUN_TAG + "（本次从 Step " + FROM_STEP + " 起）\n" +
  "- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次\n" +
  "- 范围：base（队列 / 引擎 / 协议 / 接线）→ ugv → uav → sim（双栈）→ Pictor → sim 端到端验证\n");

let abortReason = "";
let stopAll = false;

for (const step of RUN_STEPS) {
  if (stopAll) break;
  const stepStart = nowIso();
  // 每步刷新基线：评审只比对本步新增的改动
  await writeTextFile(TMP + "/git_baseline.txt", await gitStatus());
  await writeTextFile(TMP + "/prev_issues.md", "(无)");

  let passed = false;
  let rounds = 0;
  let lastIssues = "";
  let planSummary = "";

  for (let round = 1; round <= 3; round++) {
    rounds = round;
    await writeTextFile(TMP + "/step.md", stepBrief(step, round));
    await process(["rm", "-f", TMP + "/plan.md", TMP + "/impl.md", TMP + "/review.json"]);

    const rPlan = await dispatchWithRetry(PLAN_PROMPT, 2);
    if (!rPlan.ok) {
      lastIssues = "规划子 agent 连续失败（已重试）：" + String(rPlan.err);
      await writeTextFile(TMP + "/prev_issues.md", lastIssues);
      continue;
    }
    planSummary = rPlan.text;

    const rImpl = await dispatchWithRetry(IMPL_PROMPT, 2);
    if (!rImpl.ok) {
      lastIssues = "实现子 agent 连续失败（已重试）：" + String(rImpl.err);
      await writeTextFile(TMP + "/prev_issues.md", lastIssues);
      continue;
    }

    const rReview = await dispatchWithRetry(REVIEW_PROMPT, 2);
    if (!rReview.ok) {
      lastIssues = "评审子 agent 连续失败（已重试）：" + String(rReview.err);
      await writeTextFile(TMP + "/prev_issues.md", lastIssues);
      continue;
    }
    const reviewText = rReview.text;

    const reviewJson = await readOr(TMP + "/review.json", "");
    const v = parseVerdict(reviewText, reviewJson);
    if (v.pass) {
      passed = true;
      lastIssues = "";
      break;
    }
    lastIssues = v.issues || "评审判 FAIL（未给出问题清单）";
    await writeTextFile(TMP + "/prev_issues.md", lastIssues);
  }

  const oneLine = (planSummary || "").replace(/\s+/g, " ").slice(0, 900);
  let rec = "\n## Step " + step.n + " — " + step.title + "（" + RUN_TAG + "）\n";
  rec += "- 起止：" + stepStart + " → " + nowIso() + "\n";
  rec += "- 范围：" + step.side + "\n";
  rec += "- 轮次：" + rounds + " / 3\n";
  rec += "- 结果：" + (passed ? "PASS" : "FAIL（已中断）") + "\n";
  rec += "- 规划要点：" + oneLine + "\n";
  if (!passed) rec += "- 3 轮未通过的问题：\n" + lastIssues.slice(0, 5000) + "\n";
  const wbNow = await readOr(WORKBOOK, "");
  await writeTextFile(WORKBOOK, wbNow + rec);

  if (!passed) {
    abortReason = "Step " + step.n + "（" + step.title + "）3 轮未通过";
    stopAll = true;
  }
}

// 收尾：改动摘要写进 workbook，再清理 tmp/task36
try {
  const finalStatus = await gitStatus();
  const wbNow = await readOr(WORKBOOK, "");
  await writeTextFile(WORKBOOK, wbNow + "\n## 引擎结束\n- 结束时间：" + nowIso() + "\n- 状态：" +
    (abortReason ? "ABORTED — " + abortReason : "ALL_STEPS_DONE") + "\n" +
    "- git status（结束时）：\n```\n" + finalStatus.slice(0, 4000) + "\n```\n");
} catch (e) {}

await process(["rm", "-rf", TMP]);

return abortReason ? "ABORTED: " + abortReason : "ALL_STEPS_DONE";
