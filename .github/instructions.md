# 规则文件

agent必须严格遵守以下规则



# 思考模式
1. agent必须用中文进行思考

# 代码规范
## 归属规范
每一份代码文件必须在开头添加以下注释,注释规范应当严格对应当前语言：
'''
Presented by KeJi
Date ： Current date
'''
## 命名规范
1. 变量 采用lower snake case规范进行命名
2. 函数 采用Pascal snake case规范进行命名
3. 常数 采用Upper snake case规范进行命名

## Lua 绑定规范
1. 禁止在 mlua 闭包（`create_function`、`create_async_function`、`add_method`、`add_method_mut`）内编写业务逻辑。
2. 所有业务逻辑必须提取为独立的 Rust 函数（`fn` 或 `impl` 方法），闭包仅负责：
   - Lua 类型 → Rust 类型转换（`AnyUserData::borrow`、参数解包）
   - 调用提取好的 Rust 函数
   - `map_err` 转换为 `mlua::Error`
3. 正例：
   ```rust
   // MlSession 独立实现
   impl MlSession {
       pub fn encode(&self, text: &str) -> Result<Vec<u32>, String> { ... }
   }
   // 闭包只做薄胶水
   methods.add_method("encode", |_, sess, text: String| {
       sess.encode(&text).map_err(|e| mlua::Error::runtime(e))
   });
   ```
4. 反例：
   ```rust
   // ❌ 17 行业务逻辑直接写在闭包内
   methods.add_method("analyze_model", |lua, path: String| {
       let info = ...;  let t = lua.create_table()?;
       t.set("architecture", info.architecture)?;
       t.set("num_layers", info.num_layers)?;  // 共 15 个字段
       ...
   });
   ```


# 工作流程

## 目录结构
- `Task/` — 当前待执行/进行中的任务文件
  - `archived task/` — 已完成的任务归档
- `Workbook/` — 工作记录（命名格式 `wb_{id}_{description}.md`）
- `docs/` — 设计文档、说明文档

## 任务文件规范
每个任务文件命名格式：`task_{id}_{description}.md`，存储在 `Task/` 目录下。
- 例如：`Task/task_6_session_manager.md`
- 已完成的任务移入 `Task/archived task/`
- agent 只有在人类同意的情况下才能对任务文件进行修改。

## Workbook 规范
Workbook 目录中的工作记录文件与任务文件一一对应，命名方式为 `wb_{id}_{description}.md`（与 `task_{id}_{description}.md` 共享相同的 id 和 description）。用于 agent 记录工作进度，作为工作上下文。agent 在完成每一项任务后必须记录必要信息和重要细节。若文件不存在，agent 应创建一个。采用最高效、最精简的记录方式，无需考虑人类可读性。确保若启动新的 agent，其可基于现有 workbook 文件快速切换至当前工作上下文。

## Agent 工作流程
1. 阅读 `Task/` 目录下的任务文件，根据完成情况和人类评审意见决定下一项任务
2. 更新对应 workbook，记录任务开始时间
3. 根据人类的要求执行特定任务
4. 更新 workbook，记录依赖关系、结束时间，使用最精简的方式记录必要信息
5. 若任务完成，将任务文件移入 `Task/archived task/`
6. 结束任务







# 初始化

Agent 在首次启动时必须执行以下初始化步骤：

0. **阅读架构文档** — 读取 `Architecture/Pleiades_Architecture.md`，理解项目整体架构、各模块职责、关键 API 以及常见错误。重点理解：
   - Storage API 是文件访问的唯一入口，禁止绕过直接使用 `std::fs`
   - PGGUF = 原始 GGUF + 元信息，**不是** split model 的残缺产物
   - 层编号规则：0=embedding, 1..N=transformer blocks, N+1=output
   - Lua 脚本可用的全部 API 表：`caps`, `ml`, `caps.storage_*`, `caps.network.*`, `local_tensor.*`

1. **检查 SSH 密钥权限** — 私钥文件（如 `~/.ssh/id_ed25519`）权限必须为 `600`，公钥为 `644`。若权限不正确，执行 `chmod 600 ~/.ssh/id_ed25519` 修复。若 `.ssh` 目录以只读方式挂载导致无法修改权限，则将密钥复制到可写目录（如 `~/.ssh-local/`）并修复权限。

2. **配置 Git SSH 命令** — 设置环境变量 `GIT_SSH_COMMAND` 指向正确的 SSH 密钥和选项：
   ```
   export GIT_SSH_COMMAND="ssh -i ~/.ssh/id_ed25519 -o StrictHostKeyChecking=accept-new"
   ```
   若密钥在备用目录则改用对应路径（如 `-i ~/.ssh-local/id_ed25519`）。

3. **验证 SSH 连接** — 执行 `ssh -T git@github.com`（或对应 Git 托管平台）确认认证成功，预期输出包含 `successfully authenticated`。

4. **确保 Remote 使用 SSH URL** — 检查 `git remote -v`，若为 HTTPS URL 则改为 SSH 格式：
   ```
   git remote set-url origin git@github.com:<user>/<repo>.git
   ```

> **注意**：`.ssh` 目录应当以 **rw** 模式挂载。只读挂载会导致无法修正私钥权限和无法写入 `known_hosts`，从而阻碍 SSH 正常工作。
