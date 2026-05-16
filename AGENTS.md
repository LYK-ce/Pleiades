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
1. task.md  此文件指定agent需要执行的任务，agent不得进行修改。
2. Workbook目录 此目录中包含workbook文件，每个文件命名方式为workbook_<task_name>.md，每一个任务都有一个独立的workbook文件，用于agent记录工作进度，作为工作上下文，agent在完成每一项任务后必须记录必要信息和重要细节。若文件不存在，agent应创建一个。采用最高效,最精简的记录方式,无需考虑人类可读性。确保workbook.md若启动新的agent，其可快速切换至当前工作上下文，并基于现有的 workbook.md 文件继续工作。如果task.md当中的任务和workbook.md文件当中的任务有冲突，那么按照task.md中的要求进行任务。
3. problem.md 此文件用于总结并记录当前人类与agent讨论的问题。

Agents 必须按照如下的工作流程进行工作
1. 阅读task.md，并根据其中的完成情况，人类评审意见决定下一项任务
2. 更新workbook.md，记录任务的开始时间
3. 根据人类的要求，执行task.md中的一项或多项任务，如果人类没有具体要求，那么就只完成一项任务，在执行任务过程中需要检查workbook.md，了解此任务的依赖关系。
4. 更新workbook.md，任务的依赖关系，结束时间，并使用最精简的方式记录必要信息。
5. 结束任务

# 用户输入
1. 当用户输入 go 时，agent只需要完成task.md当中的一项任务！记住，只完成一项任务。一个数字标表示一个任务，它可能包含多个子任务，agent应该完成一个数字标内的所有子任务！
2. 当用户输入 summary 时，agent需要根据当前workbook的内容，总结更新work_summary.md



