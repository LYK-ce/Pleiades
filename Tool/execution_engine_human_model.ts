//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-24
//
// 支线：人体模型素材搜集 → 规范化 → Godot 接入 → 截图 + YOLO 验证
//
// 与正式 task 引擎的区别（人类明确要求）：
// - 不建 Task/ 文件、不写 Workbook/（不占正式 task 编号）
// - 中间产物与备选模型全部留在 tmp/human_model/（结束时**不清理**，供人类挑）
// - Xvfb :99 只借用，引擎结束统一清理
//
// 流程：Step 1~5 串行；每 step 规划 → 实现 → 评审，FAIL 再跑一轮，最多 3 轮。

const ROOT = "/vepfs-mlp2/c20250205/240804016/Workspace/Orion";
const TMP = ROOT + "/tmp/human_model";
const CFG_PATH = ROOT + "/Tool/execution_engine_human_model.config.json";

const CFG = JSON.parse(await readTextFile(CFG_PATH));
const STEPS = CFG.steps;
const PLAN_PROMPT = CFG.prompts.plan;
const IMPL_PROMPT = CFG.prompts.impl;
const REVIEW_PROMPT = CFG.prompts.review;

const DESIGN = `# 支线目标：给仿真世界加一个"YOLO 认得出的人"

## 目标
1. 从**官方站点**搜集真实感较强的人形模型（Renderpeople Free / Microsoft Rocketbox / Khronos glTF-Sample-Assets），下载到 tmp，作为备选；
2. 处理成 Godot 可用的 .glb（米制、朝 +X、脚底 z=0、贴图 ≤2k、三角数可控）；
3. 接入 orion-simulation-world（assets/humans/ + Person.tscn + 世界摆位 + 自检）；
4. 用 Godot 真渲染截图，交给 rust_uav_camera_test 里的 YOLOv8n 检测，证明"person 能被检出"。

## 来源范围（只许官方域名）
- renderpeople.com（Free 3D People，免费商用、免注册，真人扫描）
- github.com/microsoft/Microsoft-Rocketbox（MIT，115 个带骨骼 avatar）
- raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets（CesiumMan / RiggedFigure，仅用于链路验证）

## 目录约定（全部在 tmp 下）
- tmp/human_model/candidates.md   来源与候选清单
- tmp/human_model/proxy.md        代理用法与实测
- tmp/human_model/downloads/      原始下载物（含 README 与 sha256）
- tmp/human_model/normalized/     转换后的 glb（含 README 对比表）
- tmp/human_model/shots/          Godot 截图
- tmp/human_model/yolo_report.md  YOLO 检测结果
- tmp/human_model/plan.md / impl.md / review.json / step.md / prev_issues.md  引擎中间产物

## 边界（硬）
- 允许改：orion-simulation-world/、rust_uav_camera_test/、tmp/human_model/、本引擎两份文件
- 禁止改：pleiades-base/、pleiades-ugv/、pleiades-uav/、pleiades-terminal/、pleiades-sim/、Task/、Workbook/、根 Cargo.toml/Cargo.lock、programs/
- 不 git add / commit
- 只从官方站点下载；下载物出现可执行文件立即丢弃

## 验证链路
Godot（DISPLAY=:99 + opengl3 + Dummy 音频 + 非 headless）--截图--> tmp/human_model/shots/*.jpg
  --> rust_uav_camera_test "step 3 --image <jpg>"（同一 YOLOv8n 权重）
  --> person 类别置信度（判据：至少一张 ≥ 0.25）

## 网络
- renderpeople.com 可直连
- github.com / raw.githubusercontent.com 直连不通，必须 \`-x http://127.0.0.1:7897\`

## 清理策略
- tmp/human_model/ **保留**（备选模型与报告要给人类看）
- Xvfb :99 在引擎结束时统一关闭
`;

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
    "- **只允许**改：orion-simulation-world/、rust_uav_camera_test/、" + TMP + "/、Tool/execution_engine_human_model.*\n" +
    "- **禁止**改：pleiades-base/、pleiades-ugv/、pleiades-uav/、pleiades-terminal/、pleiades-sim/、Task/、Workbook/、根 Cargo.toml/Cargo.lock、programs/\n" +
    "- 只从官方域名取素材（renderpeople.com / github.com/microsoft / raw.githubusercontent.com/KhronosGroup）\n" +
    "- 不 git add / commit；不自己起停 Xvfb（用现成 :99）\n" +
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

// Xvfb：优先复用现成的 :99；没有就起一个，结束时统一清理
let xvfbPids = "";
try {
  const p = await process(["bash", "-lc", "pgrep -f 'Xvfb :99' | tr '\\n' ' '"]);
  xvfbPids = (p.stdout || "").trim();
} catch (e) {}
if (!xvfbPids) {
  try {
    await process(["bash", "-lc", "Xvfb :99 -screen 0 1280x720x24 +extension GLX +render -noreset >/tmp/xvfb99_engine.log 2>&1 & sleep 2; pgrep -f 'Xvfb :99' | tr '\\n' ' '"]);
    const p2 = await process(["bash", "-lc", "pgrep -f 'Xvfb :99' | tr '\\n' ' '"]);
    xvfbPids = (p2.stdout || "").trim();
  } catch (e) {}
}

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

// 收尾：关掉 Xvfb（人类要求），保留 tmp/human_model/
if (xvfbPids) {
  try {
    await process(["bash", "-lc", "kill " + xvfbPids + " 2>/dev/null; sleep 1; pgrep -f 'Xvfb :99' | tr '\\n' ' ' || true"]);
  } catch (e) {}
}
const left = await process(["bash", "-lc", "pgrep -f 'Xvfb :99' | wc -l"]);

const summary = "# 支线执行摘要\n\n" +
  "- 状态：" + (abortReason ? "ABORTED — " + abortReason : "ALL_STEPS_DONE") + "\n" +
  "- Xvfb：启动前 pid=[" + xvfbPids + "]，收尾后残留=" + (left.stdout || "?").trim() + "\n" +
  "- 产物目录：tmp/human_model/（downloads/ normalized/ shots/）\n\n" +
  "## 分步结果\n" + summaryLines.join("\n") + "\n";
await writeTextFile(TMP + "/run_summary.md", summary);

return abortReason ? "ABORTED: " + abortReason : "ALL_STEPS_DONE";
