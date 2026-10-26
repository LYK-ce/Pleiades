//Presented by KeJi
//Created Date ： 2026-10-25
//Modified Date ： 2026-10-25
//
// 支线：让世界里的人"走起来"（Renderpeople 免费 Animated People → glb → Godot）
//
// 与正式 task 无关：不建 Task/、不写 Workbook/；中间产物留 tmp/walking_person/。
// 目标单一：角色能在世界里走动（走路动画在播 + 位置在变），不加碰撞、不碰感知、不做 YOLO。
//
// 流程：Step 1~4 串行；每 step 规划 → 实现 → 评审，FAIL 再跑一轮，最多 3 轮。

const ROOT = "/vepfs-mlp2/c20250205/240804016/Workspace/Orion";
const TMP = ROOT + "/tmp/walking_person";
const CFG_PATH = ROOT + "/Tool/execution_engine_walking_person.config.json";

const CFG = JSON.parse(await readTextFile(CFG_PATH));
const STEPS = CFG.steps;
const PLAN_PROMPT = CFG.prompts.plan;
const IMPL_PROMPT = CFG.prompts.impl;
const REVIEW_PROMPT = CFG.prompts.review;

const DESIGN = "# 支线目标：让世界里的人走起来\n\n" +
"## 目标（只这一件事）\n" +
"人物在世界里沿路径走动：走路动画在播（不是滑行）+ 位置随时间变化。\n" +
"**不做**：碰撞体、雷达/建图/决策链接入、YOLO 验证、多机/网络、正式 task 文档。\n\n" +
"## 素材来源（官方）\n" +
"- 首选 renderpeople.com 的 Free Animated People（自带烘焙走路，免费商用、免注册，可直连）\n" +
"- 兜底 Khronos CesiumMan.glb（自带 walk，写实度低，仅证链路；github 需代理 `-x http://127.0.0.1:7897`）\n\n" +
"## 目录约定（全在 tmp 下）\n" +
"- tmp/walking_person/design.md / plan.md / impl.md / review.json / step.md / prev_issues.md\n" +
"- tmp/walking_person/downloads/   原始下载物 + README + sha256\n" +
"- tmp/walking_person/normalized/  处理后的 glb（保留动画）+ README\n" +
"- tmp/walking_person/shots/       三帧出图\n" +
"- tmp/walking_person/walk_report.md  截图与位移/差分数字\n\n" +
"## 边界（硬）\n" +
"- 允许改：orion-simulation-world/、tmp/walking_person/、本引擎两份文件\n" +
"- 禁止改：pleiades-base/、pleiades-ugv/、pleiades-uav/、pleiades-terminal/、pleiades-sim/、Task/、Workbook/、根 Cargo.toml/Cargo.lock、programs/、rust_uav_camera_test/\n" +
"- 不 git add / commit\n\n" +
"## 验收口径\n" +
"- 资产层：glb 含 animations>=1、skins>=1、时长>0.5s；人高 1.60~1.85m、脚底 z≈0、朝 +X\n" +
"- 运行层：AnimationPlayer 在播；位置随时间单调变化；走完一圈回起点（≤0.05m）；无抖动\n" +
"- 渲染层：三帧 JPEG 非全黑、人体投影框在移动、框内像素在变（姿态变化）\n\n" +
"## 清理策略\n" +
"- tmp/walking_person/ 保留（素材与证据给人类看）\n" +
"- Xvfb :99 由引擎收尾统一关闭（实现者在缺显示时临时起一个）\n";

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
    "> 设计文档：" + TMP + "/design.md\n" +
    "> 中间产物：全部写 " + TMP + "/ 下\n\n" +
    "## 本 step 要求\n\n" + step.brief + "\n\n" +
    "## 全局硬约束\n\n" +
    "- **只允许**改：orion-simulation-world/、" + TMP + "/、Tool/execution_engine_walking_person.*\n" +
    "- **禁止**改：pleiades-base/、pleiades-ugv/、pleiades-uav/、pleiades-terminal/、pleiades-sim/、Task/、Workbook/、根 Cargo.toml/Cargo.lock、programs/、rust_uav_camera_test/\n" +
    "- 只做「走动」：不加碰撞、不改雷达/建图/决策链、不做 YOLO\n" +
    "- 只从官方域名取素材；不 git add / commit\n" +
    "- 取不到就如实写失败，不许编造链接或结果\n";
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

await process(["mkdir", "-p", TMP + "/downloads", TMP + "/normalized", TMP + "/shots", TMP + "/tools"]);
await writeTextFile(TMP + "/design.md", DESIGN);

let baseline = "";
try {
  const gs = await process(["bash", "-lc", "cd " + ROOT + " && git status --porcelain --untracked-files=all"]);
  baseline = (gs.stdout || "") + (gs.stderr || "");
} catch (e) {
  baseline = "(git status 失败: " + String(e) + ")";
}
await writeTextFile(TMP + "/git_baseline.txt", baseline);

// Xvfb：**引擎不自己起**（实测：在 process() 里后台起 Xvfb 会让该调用永不返回，
// 整条链卡死在第 0 步）。只探测现成的 :99；缺了就在摘要里如实写明，交给 Step 4 判"未验证"。
let xvfbPids = "";
try {
  const p = await process(["bash", "-lc", "ps -eo pid,comm | awk '$2==\"Xvfb\"{print $1}' | tr '\\n' ' '"]);
  xvfbPids = (p.stdout || "").trim();
} catch (e) {}

let abortReason = "";
let stopAll = false;
const summaryLines = [];

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

  const oneLine = (planSummary || "").replace(/\s+/g, " ").slice(0, 700);
  summaryLines.push("- Step " + step.n + " " + step.title + "：轮次 " + rounds + "/3，" +
    (passed ? "PASS" : "FAIL（中断）") + "；规划要点：" + oneLine);
  if (!passed) {
    abortReason = "Step " + step.n + "（" + step.title + "）3 轮未通过";
    summaryLines.push("  - 3 轮问题：\n" + lastIssues.slice(0, 2500));
    stopAll = true;
  }
}

if (xvfbPids) {
  try {
    await process(["bash", "-lc", "kill " + xvfbPids + " 2>/dev/null; true"]);
  } catch (e) {}
}
const left = await process(["bash", "-lc", "ps -eo pid,comm | awk '$2==\"Xvfb\"' | wc -l"]);

const summary = "# 支线执行摘要（walking_person）\n\n" +
  "- 状态：" + (abortReason ? "ABORTED — " + abortReason : "ALL_STEPS_DONE") + "\n" +
  "- Xvfb：启动前 pid=[" + xvfbPids + "]，收尾后残留=" + (left.stdout || "?").trim() + "\n" +
  "- 产物目录：tmp/walking_person/（downloads/ normalized/ shots/）\n\n" +
  "## 分步结果\n" + summaryLines.join("\n") + "\n";
await writeTextFile(TMP + "/run_summary.md", summary);

return abortReason ? "ABORTED: " + abortReason : "ALL_STEPS_DONE";
