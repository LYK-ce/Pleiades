//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21
//
// task_31 执行引擎（参考实现，与 run_program 内联版本一致）
//
// 用途：按 .github/instructions.md 的「执行引擎使用规则」，串行执行
//       Task/task_31_sim_world_link.md 的 7 个 step。每个 step 依次 dispatch
//       「规划 → 实现 → 评审」3 个子 agent；评审 FAIL 再跑一轮；每 step 最多 3 轮，
//       3 轮不过则中断整条链，等人类处理。
//
// 设计：本文件只保留流程；step 定义与三个提示词外置在
//       Tool/execution_engine_task31.config.json（改 step 不必改流程）。
//
// 说明：
// - 不调用 report()；进展写 Workbook/wb_31_sim_world_link.md
// - 中间产物只放 tmp/task31/（不清理整个 tmp/，那里有历史调研资料）
// - Step 1 在 orion-simulation-world/；Step 2~6 在 pleiades-sim/；Step 7 端到端测试
// - 硬约束：不得修改 pleiades-base / pleiades-ugv / pleiades-uav / pleiades-terminal

const ROOT = "/vepfs-mlp2/c20250205/240804016/Workspace/Orion";
const TMP = ROOT + "/tmp/task31";
const WORKBOOK = ROOT + "/Workbook/wb_31_sim_world_link.md";
const TASK = ROOT + "/Task/task_31_sim_world_link.md";
const CFG_PATH = ROOT + "/Tool/execution_engine_task31.config.json";

const CFG = JSON.parse(await readTextFile(CFG_PATH));
const STEPS = CFG.steps;
const PLAN_PROMPT = CFG.prompts.plan;
const IMPL_PROMPT = CFG.prompts.impl;
const REVIEW_PROMPT = CFG.prompts.review;

async function readOr(path, fallback) {
  try {
    if (await exists(path)) return await readTextFile(path);
  } catch (e) {
    return fallback;
  }
  return fallback;
}

function sideLabel(side) {
  if (side === "world") return "orion-simulation-world/（Godot 工程）";
  if (side === "test") return "pleiades-sim/ + orion-simulation-world/（端到端测试）";
  return "pleiades-sim/（Rust crate）";
}

function sideScope(side) {
  if (side === "world") return "只改 orion-simulation-world/";
  if (side === "test") return "允许改 pleiades-sim/ 与 orion-simulation-world/（仅修 bug）";
  return "只改 pleiades-sim/";
}

function stepBrief(step, round) {
  return "# Step " + step.n + " — " + step.title + "\n\n" +
    "> 执行轮次：第 " + round + " / 3 轮\n" +
    "> 改动范围：" + sideLabel(step.side) + "\n" +
    "> 任务文档：" + TASK + "\n\n" +
    "## 本 step 要求\n\n" + step.brief + "\n\n" +
    "## 全局硬约束\n\n" +
    "- 不得修改 pleiades-base / pleiades-ugv / pleiades-uav / pleiades-terminal\n" +
    "- " + sideScope(step.side) + "\n" +
    "- pleiades-sim 内禁止 use pleiades_ugv / pleiades_uav（可复制语义、代码独立）\n" +
    "- 不执行 git add / git commit\n";
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

await process(["mkdir", "-p", TMP]);
if (!(await exists(TASK))) return "ABORTED: 任务文档不存在 " + TASK;

let wb = await readOr(WORKBOOK, "");
if (!wb.trim()) wb = "# wb_31_sim_world_link — task_31 工作记录\n\n---\n";
await writeTextFile(WORKBOOK, wb + "\n## 执行引擎启动\n- 引擎：Tool/execution_engine_task31.ts + Tool/execution_engine_task31.config.json\n- 计划：Step 1~" + STEPS.length + " 串行，每 step 最多 3 轮（规划 → 实现 → 评审）\n");

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
    if (v.pass) { passed = true; lastIssues = ""; break; }
    lastIssues = v.issues || "评审判 FAIL（未给出问题清单）";
  }

  const oneLine = (planSummary || "").replace(/\s+/g, " ").slice(0, 800);
  let rec = "\n## Step " + step.n + " — " + step.title + "\n";
  rec += "- 范围：" + sideLabel(step.side) + "\n";
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

await process(["rm", "-rf", TMP]);
return abortReason ? "ABORTED: " + abortReason : "ALL_STEPS_DONE";
