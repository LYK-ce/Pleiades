//Presented by KeJi
//Created Date ： 2026-10-25
//Modified Date ： 2026-10-25
//
// task_35 执行引擎（行人底盘：Python 经 WS 直接驱动）
//
// 流程：Step 1~5 串行；每 step 规划 → 实现 → 评审，FAIL 再跑一轮，最多 3 轮。
// 产物：Workbook/wb_35_pedestrian.md（每 step 一节）+ tmp/task35/（中间产物，结束清理）
// 注意：Xvfb 由主 agent 起好；引擎只探测，收尾时统一 kill（人类要求）。

const ROOT = "/vepfs-mlp2/c20250205/240804016/Workspace/Orion";
const TMP = ROOT + "/tmp/task35";
const WORKBOOK = ROOT + "/Workbook/wb_35_pedestrian.md";
const TASK = ROOT + "/Task/task_35_pedestrian.md";
const CFG_PATH = ROOT + "/Tool/execution_engine_pedestrian.config.json";

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

function stepBrief(step, round) {
  return "# Step " + step.n + " — " + step.title + "\n\n" +
    "> 执行轮次：第 " + round + " / 3 轮\n" +
    "> 任务文档：" + TASK + "\n" +
    "> 中间产物：" + TMP + "/\n\n" +
    "## 本 step 要求\n\n" + step.brief + "\n\n" +
    "## 全局硬约束\n\n" +
    "- 只允许改：orion-simulation-world/（含其下 tool/）、" + TMP + "/、Tool/execution_engine_pedestrian.*\n" +
    "- **禁止**改：pleiades-base/、pleiades-ugv/、pleiades-uav/、pleiades-terminal/、pleiades-sim/、programs/、rust_uav_camera_test/、根 Cargo.toml/Cargo.lock、Task/\n" +
    "- **不涉及 SLAM/建图/动态障碍/避让**；不做 YOLO、不做图传\n" +
    "- 不 git add / commit；不自己起停 Xvfb（用现成 :99）\n" +
    "- 取不到/做不到就如实写失败，不许编造\n";
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

await process(["mkdir", "-p", TMP + "/shots", TMP + "/logs"]);
if (!(await exists(TASK))) return "ABORTED: 任务文档不存在 " + TASK;

let baseline = "";
try {
  const gs = await process(["bash", "-lc", "cd " + ROOT + " && git status --porcelain --untracked-files=all"]);
  baseline = (gs.stdout || "") + (gs.stderr || "");
} catch (e) {
  baseline = "(git status 失败: " + String(e) + ")";
}
await writeTextFile(TMP + "/git_baseline.txt", baseline);

let xvfbPids = "";
try {
  const p = await process(["bash", "-lc", "ps -eo pid,comm | awk '$2==\"Xvfb\"{print $1}' | tr '\\n' ' '"]);
  xvfbPids = (p.stdout || "").trim();
} catch (e) {}

let wb = await readOr(WORKBOOK, "");
if (!wb.trim()) {
  wb = "# wb_35_pedestrian — task_35 工作记录\n\n" +
    "> Presented by KeJi\n" +
    "> Created Date ： 2026-10-25\n" +
    "> Modified Date ： 2026-10-25\n" +
    "> 任务：Task/task_35_pedestrian.md（行人底盘：Python 经 WS 直接驱动）\n";
}
await writeTextFile(WORKBOOK, wb + "\n## 执行引擎启动\n" +
  "- 引擎：Tool/execution_engine_pedestrian.ts + Tool/execution_engine_pedestrian.config.json\n" +
  "- 计划：Step 1~" + STEPS.length + " 串行，每 step 最多 3 轮（规划 → 实现 → 评审）\n" +
  "- 范围：仅 orion-simulation-world/（含 tool/）与 tmp/task35/；不涉及 SLAM/建图\n");

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

  const oneLine = (planSummary || "").replace(/\s+/g, " ").slice(0, 900);
  let rec = "\n## Step " + step.n + " — " + step.title + "\n";
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

// 收尾：记录 git 摘要，清理 tmp，kill Xvfb
try {
  const gs2 = await process(["bash", "-lc", "cd " + ROOT + " && git status --porcelain --untracked-files=all"]);
  const wbNow = await readOr(WORKBOOK, "");
  await writeTextFile(WORKBOOK, wbNow + "\n## 引擎结束\n- 状态：" +
    (abortReason ? "ABORTED — " + abortReason : "ALL_STEPS_DONE") + "\n" +
    "- git status（结束时）：\n```\n" + ((gs2.stdout || "") + (gs2.stderr || "")).slice(0, 4000) + "\n```\n");
} catch (e) {}

await process(["rm", "-rf", TMP]);
if (xvfbPids) {
  try {
    await process(["bash", "-lc", "kill " + xvfbPids + " 2>/dev/null; true"]);
  } catch (e) {}
}

return abortReason ? "ABORTED: " + abortReason : "ALL_STEPS_DONE";
