//Presented by KeJi
//Created Date ： 2026-10-30
//Modified Date ： 2026-10-30
//
// task_41 执行引擎（与 run_program 内联版本一致）；本次运行：Step 1 ~ Step 4
//
// 用途：按 .github/instructions.md 的「执行引擎使用规则」，串行执行
//       Task/task_41_containment.md 的 Step 1 ~ Step 4。
//       Step 1/3：规划 → 实现 → 评审，评审 FAIL 再跑一轮，最多 3 轮；
//       Step 2/4：只验不改，**只跑 1 轮**；FAIL → 发邮件 + 立即终止（不重试、不调参）。
//
// 设计：本文件只保留流程；step 定义与三个提示词外置在
//       Tool/execution_engine_task41.config.json（改 step 不必改流程）。
//
// 说明：
// - 不调用 report()；进展写 Workbook/wb_41_containment.md
// - 中间产物只放 tmp/task41/（结束清理）
// - 失败（Step 2/4 FAIL、或任一 step 最终未通过）用 tool/mailer.py 邮件汇报人类
// - 每步开始前刷新 git 基线，评审只比对“本步新增改动”

const ROOT = "/vepfs-mlp2/c20250205/240804016/Workspace/Orion";
const CFG_PATH = ROOT + "/Tool/execution_engine_task41.config.json";
const MAILER = "/vepfs-mlp2/c20250205/240804016/tool/mailer.py";

const CFG = JSON.parse(await readTextFile(CFG_PATH));
const STEPS = CFG.steps;
const TMP = ROOT + "/" + CFG.tmp;
const WORKBOOK = ROOT + "/" + CFG.workbook;
const TASK = ROOT + "/" + CFG.task;
const PLAN_PROMPT = (CFG.prompts.plan || []).join("\n");
const IMPL_PROMPT = (CFG.prompts.impl || []).join("\n");
const REVIEW_PROMPT = (CFG.prompts.review || []).join("\n");

const FROM_STEP = 4;
const RUN_TAG = "Step 4 重跑（车挪四角 + 人停 120 s）";
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

function isVerify(step) {
  const n = Number(step && step.n ? step.n : 0);
  return n === 2 || n === 4;
}

function allowsLongProcess(step) {
  return Number(step && step.n ? step.n : 0) >= 2;
}

function stepBrief(step, round) {
  const brief = (Array.isArray(step.brief) ? step.brief : []).join("\n");
  const verify = isVerify(step);
  const roundLine = verify
    ? "> 执行轮次：**只验不改，仅 1 轮**（FAIL 即终止，不重试）\n"
    : "> 执行轮次：第 " + round + " / 3 轮\n";
  const longProc = allowsLongProcess(step)
    ? "- 本 step **允许**启动长驻进程（Godot 世界 / pleiades-sim / Pictor / 行人客户端），但结束时必须收干净，不得留残留进程或占用端口\n"
    : "- 不得启动任何长驻进程（不起 pleiades-sim、不起 Godot 长跑）；Godot 只允许 `--headless --quit` 的自检\n";
  return "# Step " + step.n + " — " + step.title + "\n\n" +
    roundLine +
    "> 对应任务文档章节：" + step.doc + "\n" +
    "> 改动范围：" + step.side + "\n" +
    "> 任务文档：" + TASK + "\n\n" +
    "## 本 step 要求（细节以任务文档 " + step.doc + " 为准）\n\n" + brief + "\n\n" +
    "## 本 step 允许改动的路径（越界即 FAIL）\n\n" + allowedList(step) + "\n\n" +
    "## 全局硬约束\n\n" +
    "- 中间产物只写 " + TMP + "/\n" +
    "- **禁止**修改：Task/、.github/、根 Cargo.toml / Cargo.lock\n" +
    "- **禁止** git add / git commit；**禁止** `cargo fmt`\n" +
    "- 验证一律逐 crate：`cargo check -p <crate> --all-targets`；三个 Rust crate 只有 `[[bin]]`，**测试用 `--bins` 不用 `--lib`**（base 用 `--lib`）\n" +
    "- Godot 只允许 `--headless`（`--import` / `--quit` / `-s <脚本>`）；需要显示时用 `xvfb-run -a`\n" +
    "- 联网：直连优先，失败再加 `-x http://127.0.0.1:7897`（mihomo mixed-port）\n" +
    "- 一切定值以任务文档 §二 为准，不得自行改动\n" +
    "- **Pictor 是独立 git 仓**：改动单独核对 `git -C Pictor status --porcelain --untracked-files=all`\n" +
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

async function latestResults(step) {
  const n = Number(step && step.n ? step.n : 0);
  const prefix = n === 2 ? "task41_s2_" : (n === 4 ? "task41_s4_" : "");
  if (!prefix) return "";
  try {
    const r = await process(["bash", "-lc", "cd " + ROOT + " && ls -d RobotSim_TestEnv/results/" + prefix + "* 2>/dev/null | tail -1"]);
    const p = (r.stdout || "").trim();
    return p ? ROOT + "/" + p : "";
  } catch (e) {
    return "";
  }
}

async function sendMail(step, reason, lastIssues) {
  const dir = await latestResults(step);
  const verdict = dir ? dir + "/verdict.md" : "";
  const subject = "[Orion][task_41] Step " + step.n + " FAIL — " + String(step.title || "").slice(0, 70);
  const lines = [];
  lines.push("Step " + step.n + "（" + step.title + "）FAIL，执行程序已终止。");
  lines.push("");
  lines.push("原因：" + String(reason || "").slice(0, 500));
  lines.push("产物目录：" + (dir || "(未找到 task41 结果目录)"));
  lines.push("verdict：" + (verdict || "(无)"));
  lines.push("");
  lines.push("评审问题（截断）：");
  lines.push(String(lastIssues || "(无)").slice(0, 3000));
  lines.push("");
  lines.push("已做的：只验不改（未调参、未改判据、未重跑）。");
  lines.push("若是 FAIL(env)（fps 低 / 位姿龄大 / 检测链不稳 / 端口残留），请在有 GPU 的环境复验。");
  const body = lines.join("\n");

  const args = ["python3", MAILER, subject, body];
  if (verdict && (await exists(verdict))) args.push(verdict);

  let ok = false;
  let lastOut = "";
  for (let i = 0; i < 2; i++) {
    try {
      const r = await process(args);
      lastOut = String(r.stdout || "") + "\n" + String(r.stderr || "");
      if (Number(r.code) === 0) { ok = true; break; }
    } catch (e) {
      lastOut = String(e);
    }
  }

  if (!ok) {
    try {
      const fallbackDir = ROOT + "/RobotSim_TestEnv/results";
      await process(["mkdir", "-p", fallbackDir]);
      const path = fallbackDir + "/EMAIL_FAILED_" + Date.now() + ".txt";
      await writeTextFile(path, "SUBJECT: " + subject + "\n\n" + body + "\n\nMAILER OUTPUT:\n" + lastOut);
    } catch (e) {}
  }
  return ok;
}

await process(["mkdir", "-p", TMP]);
if (!(await exists(TASK))) return "ABORTED: 任务文档不存在 " + TASK;

await writeTextFile(TMP + "/git_baseline_global.txt", await gitStatus());

let wb = await readOr(WORKBOOK, "");
if (!wb.trim()) {
  wb = "# wb_41_containment — task_41 工作记录\n\n" +
    "> Presented by KeJi\n" +
    "> Created Date ： 2026-10-30\n" +
    "> Modified Date ： 2026-10-30\n" +
    "> 任务：Task/task_41_containment.md（封控：协议/Mission + sim 机栈行为树 + 协助请求 + Pictor 协同控制）\n";
}
await writeTextFile(WORKBOOK, wb + "\n## 执行引擎启动\n" +
  "- 开始时间：" + nowIso() + "\n" +
  "- 引擎：Tool/execution_engine_task41.ts + Tool/execution_engine_task41.config.json（流程照搬 task_36/38/39/40）\n" +
  "- 运行标记：" + RUN_TAG + "（本次从 Step " + FROM_STEP + " 起）\n" +
  "- 计划：Step 1/3 最多 3 轮（规划 → 实现 → 评审）；Step 2/4 只跑 1 轮，FAIL → 邮件 + 终止\n" +
  "- 范围：封控指令贯通（1）→ tree v1 验证（2）→ tree v2 + 协助链路 + Pictor 协控（3）→ 端到端验证（4）\n");

let abortReason = "";
let stopAll = false;

for (const step of RUN_STEPS) {
  if (stopAll) break;
  const stepStart = nowIso();
  const verify = isVerify(step);
  const maxRounds = verify ? 1 : 3;

  await writeTextFile(TMP + "/git_baseline.txt", await gitStatus());
  await writeTextFile(TMP + "/prev_issues.md", "(无)");

  let passed = false;
  let rounds = 0;
  let lastIssues = "";
  let planSummary = "";

  for (let round = 1; round <= maxRounds; round++) {
    rounds = round;
    await writeTextFile(TMP + "/step.md", stepBrief(step, round));
    await process(["rm", "-f", TMP + "/plan.md", TMP + "/impl.md", TMP + "/review.json"]);

    const rPlan = await dispatchWithRetry(PLAN_PROMPT, 2);
    if (!rPlan.ok) {
      lastIssues = "规划子 agent 连续失败（已重试）：" + String(rPlan.err);
      await writeTextFile(TMP + "/prev_issues.md", lastIssues);
      if (verify) break;
      continue;
    }
    planSummary = rPlan.text;

    const rImpl = await dispatchWithRetry(IMPL_PROMPT, 2);
    if (!rImpl.ok) {
      lastIssues = "实现子 agent 连续失败（已重试）：" + String(rImpl.err);
      await writeTextFile(TMP + "/prev_issues.md", lastIssues);
      if (verify) break;
      continue;
    }

    const rReview = await dispatchWithRetry(REVIEW_PROMPT, 2);
    if (!rReview.ok) {
      lastIssues = "评审子 agent 连续失败（已重试）：" + String(rReview.err);
      await writeTextFile(TMP + "/prev_issues.md", lastIssues);
      if (verify) break;
      continue;
    }

    const reviewJson = await readOr(TMP + "/review.json", "");
    const v = parseVerdict(rReview.text, reviewJson);
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
  rec += "- 轮次：" + rounds + " / " + maxRounds + "\n";
  rec += "- 结果：" + (passed ? "PASS" : "FAIL（已中断）") + "\n";
  rec += "- 规划要点：" + oneLine + "\n";
  if (!passed) rec += "- 未通过的问题：\n" + String(lastIssues).slice(0, 5000) + "\n";
  const wbNow = await readOr(WORKBOOK, "");
  await writeTextFile(WORKBOOK, wbNow + rec);

  if (!passed) {
    abortReason = "Step " + step.n + "（" + step.title + "）" + (verify ? "验证 FAIL" : "3 轮未通过");
    // 人类要求：失败就发邮件（Step 2 与 Step 4 尤其重要）
    await sendMail(step, abortReason, lastIssues);
    stopAll = true;
  }
}

try {
  const finalStatus = await gitStatus();
  const wbNow = await readOr(WORKBOOK, "");
  await writeTextFile(WORKBOOK, wbNow + "\n## 引擎结束\n- 结束时间：" + nowIso() + "\n- 状态：" +
    (abortReason ? "ABORTED — " + abortReason : "ALL_STEPS_DONE") + "\n" +
    "- git status（结束时）：\n```\n" + finalStatus.slice(0, 4000) + "\n```\n");
} catch (e) {}

await process(["rm", "-rf", TMP]);

return abortReason ? "ABORTED: " + abortReason : "ALL_STEPS_DONE";
