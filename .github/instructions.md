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


# 工作流程
在根目录下包含以下内容：
1. task.md  此文件指定agent需要执行的任务，agent只有在人类同意的情况下才能进行修改。
2. Workbook目录 此目录中包含workbook文件，每个文件命名方式为workbook_xxx_task.md，每一个任务都有一个独立的workbook文件，用于agent记录工作进度，作为工作上下文，agent在完成每一项任务后必须记录必要信息和重要细节。若文件不存在，agent应创建一个。采用最高效,最精简的记录方式,无需考虑人类可读性。确保workbook.md若启动新的agent，其可快速切换至当前工作上下文，并基于现有的 workbook.md 文件继续工作。
3. docs目录，用于存放设计文档，说明文档等内容

Agents 必须按照如下的工作流程进行工作
1. 阅读task.md，并根据其中的完成情况，人类评审意见决定下一项任务
2. 更新workbook.md，记录任务的开始时间
3. 根据人类的要求，执行task.md中的特定任务
4. 更新workbook.md，任务的依赖关系，结束时间，并使用最精简的方式记录必要信息。
5. 结束任务






# 初始化

Agent 在首次启动时必须执行以下 Git SSH 环境初始化步骤：

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
