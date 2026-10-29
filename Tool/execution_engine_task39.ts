//Presented by KeJi
//Created Date ： 2026-10-29
//Modified Date ： 2026-10-29
//
// task_39 执行引擎（与 run_program 内联版本一致）
//
// 用途：按 .github/instructions.md 的「执行引擎使用规则」，串行执行
//       Task/task_39_executor_and_path.md 的 Step 1a / 1b / 2 / 3。
//       每个 step 依次 dispatch「规划 → 实现 → 评审」3 个子 agent；评审 FAIL 再跑一轮；
//       每 step 最多 3 轮，3 轮不过则中断整条链，等人类处理。
//
// 设计：本文件只保留流程；step 定义与三个提示词外置在
//       Tool/execution_engine_task39.config.json（改 step 不必改流程）。
//       流程照搬 task_36 / task_38 的执行引擎。
//
// 说明：
// - 不调用 report()；进展写 Workbook/wb_39_executor_and_path.md
// - 中间产物只放 tmp/task39/（结束清理）
// - 每一步开始前刷新 git 基线，评审只比对“本步新增改动”

const ROOT = "/vepfs-mlp2/c20250205/240804016/Workspace/Orion";
const CFG_PATH = ROOT + "/Tool/execution_engine_task39.config.json";

const CFG = JSON.parse(await readTextFile(CFG_PATH));
const STEPS = CFG.steps;
const TMP = ROOT + "/" + CFG.tmp;
const WORKBOOK = ROOT + "/" + CFG.workbook;
const TASK = ROOT + "/" + CFG.task;
const PLAN_PROMPT = (CFG.prompts.plan || []).join("\n");
const IMPL_PROMPT = (CFG.prompts.impl || []).join("\n");
const REVIEW_PROMPT = (CFG.prompts.review || []).join("\n");

// 只跑指定 step 起（全量跑改成 0）；RUN_TAG 用于在 workbook 里区分第几次运行
const FROM_STEP = 4;
const RUN_TAG = "口径对齐后重跑验证（Step 4）";
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

// 本步是否允许起长驻进程（只有 Step 3 验证需要世界/sim/行人）
function allowsLongProcess(step) {
  return Number(step.n || 0) >= 4;
}

function stepBrief(step, round) {
  const brief = (Array.isArray(step.brief) ? step.brief : []).join("\n");
  const longProc = allowsLongProcess(step)
    ? "- 本 step **允许**启动长驻进程（Godot 世界 / pleiades-sim / 行人客户端），但结束时必须收干净，不得留残留进程或占用端口\n"
    : "- 不得启动任何长驻进程（不起 pleiades-sim、不起 Godot 长跑）；Godot 只允许 `--headless --quit` 的自检\n";
  return "# Step " + step.n + " — " + step.title + "\n\n" +
    "> 执行轮次：第 " + round + " / 3 轮\n" +
    "> 对应任务文档章节：" + step.doc + "\n" +
    "> 改动范围：" + step.side + "\n" +
    "> 任务文档：" + TASK + "\n\n" +
    "## 本 step 要求（细节以任务文档 " + step.doc + " 为准）\n\n" + brief + "\n\n" +
    "## 本 step 允许改动的路径（越界即 FAIL）\n\n" + allowedList(step) + "\n\n" +
    "## 全局硬约束\n\n" +
    "- 中间产物只写 " + TMP + "/\n" +
    "- **禁止**修改：Task/、.github/、根 Cargo.toml / Cargo.lock\n" +
    "- **禁止** git add / git commit；**禁止** `cargo fmt`\n" +
    "- 验证一律逐 crate：`cargo check -p <crate> --all-targets`（workspace 全量 --all-targets 已知红：t01_storage_integration）\n" +
    "- 不碰急停逻辑（“前方 0.3 m 完全停住”是既定设计）\n" +
    longProc;
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

// git status 里混进了登录 shell 的噪音（/etc/profile: ... ShellCrash），逐行剔掉
function cleanStatus(raw) {
  const lines = String(raw || "").split("\n");
  const keep = lines.filter((l) => {
    if (!l.trim()) return false;
    if (l.indexOf("ShellCrash") >= 0) return false;
    if (l.indexOf("/etc/profile") >= 0) return false;
    return true;
  });
  return keep.join("\n");
}

async function gitStatus() {
  try {
    const gs = await process(["bash", "-lc", "cd " + ROOT + " && git status --porcelain --untracked-files=all"]);
    return cleanStatus((gs.stdout || "") + "\n" + (gs.stderr || ""));
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
  wb = "# wb_39_executor_and_path — task_39 工作记录\n\n" +
    "> Presented by KeJi\n" +
    "> Created Date ： 2026-10-29\n" +
    "> Modified Date ： 2026-10-29\n" +
    "> 任务：Task/task_39_executor_and_path.md（路径 8 连通 + 规划三态预算 + sim 机栈连续执行 + 验证）\n";
}
await writeTextFile(WORKBOOK, wb + "\n## 执行引擎启动\n" +
  "- 开始时间：" + nowIso() + "\n" +
  "- 引擎：Tool/execution_engine_task39.ts + Tool/execution_engine_task39.config.json（流程照搬 task_36/38）\n" +
  "- 运行标记：" + RUN_TAG + "（本次从 Step " + FROM_STEP + " 起）\n" +
  "- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次\n" +
  "- 范围：规划三态+预算（1a）→ 回退膨胀+8 连通（1b）→ sim 机栈连续执行器（2）→ 验证（3）\n" +
  "- 验收红线：Step 3 **只验不改**；不通过就如实汇报，不调阈值/不改代码\n");

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

// 收尾：改动摘要写进 workbook，再清理 tmp/task39
try {
  const finalStatus = await gitStatus();
  const wbNow = await readOr(WORKBOOK, "");
  await writeTextFile(WORKBOOK, wbNow + "\n## 引擎结束\n- 结束时间：" + nowIso() + "\n- 状态：" +
    (abortReason ? "ABORTED — " + abortReason : "ALL_STEPS_DONE") + "\n" +
    "- git status（结束时）：\n```\n" + finalStatus.slice(0, 4000) + "\n```\n");
} catch (e) {}

await process(["rm", "-rf", TMP]);

return abortReason ? "ABORTED: " + abortReason : "ALL_STEPS_DONE";
