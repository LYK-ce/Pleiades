//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 任务队列（Task 36 §12.1）
//!
//! 语义（§十一 #15~#17）：
//! - `replace`：整队替换（打断 + 丢弃 + **不恢复**）；空列表 = 停车待命 —— 与改造前行为逐字等价
//! - `append`：插队尾（当前任务不受影响）
//! - `preempt`：插队头（当前任务立即让位）
//! - `head` / `pop_next`：队首 = 正在执行的任务位；出队只走 `pop_next` 一条路径
//!
//! ⚠️ 本 step 只**提供能力**：`append` / `preempt` / `head` 的**消费方切换**属于行为树 step
//! （任务文档 §十二 Step 4 起）。本 step 内三者在生产代码中**没有调用者**，设备端
//! `GoalService::get_path()` 仍按旧语义"开始时 `pop_next` 取走队首"。

use std::collections::VecDeque;

use super::element::{Mission, QueuedTask, TaskId};

/// FIFO 任务队列（队首 = 正在执行的任务位）
#[derive(Debug, Clone, Default)]
pub struct MissionQueue {
    queue: VecDeque<QueuedTask>,
    /// 下一个待分配编号；`clear` 不回退（避免 id 复用）
    next_id: u64,
}

impl MissionQueue {
    /// 内部：分配编号并包成队列元素
    fn assign(&mut self, mission: Mission) -> QueuedTask {
        let id = TaskId(self.next_id);
        // 到 u64::MAX 才回绕；按 50ms 一拍最多入队一条算，实际不可能触发
        self.next_id = self.next_id.wrapping_add(1);
        QueuedTask { id, mission }
    }

    /// 整体替换队列（打断 + 丢弃 + 不恢复）；空列表 = 停车待命
    pub fn replace(&mut self, missions: Vec<Mission>) {
        self.queue.clear();
        self.append(missions);
    }

    /// 追加队尾（当前任务不受影响；批内顺序保持）
    pub fn append(&mut self, missions: Vec<Mission>) {
        for m in missions {
            let task = self.assign(m);
            self.queue.push_back(task);
        }
    }

    /// 插入队头（当前任务立即让位；批内 `missions[0]` 最优先）
    pub fn preempt(&mut self, missions: Vec<Mission>) {
        // 先按自然顺序分配编号（id 与批内顺序一致），再整体倒序插到队头
        let batch: Vec<QueuedTask> = missions.into_iter().map(|m| self.assign(m)).collect();
        for task in batch.into_iter().rev() {
            self.queue.push_front(task);
        }
    }

    /// 清空（`next_id` 不回退，避免 id 复用）
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// 只读队首（= 执行中的位置），不移除
    pub fn head(&self) -> Option<&QueuedTask> {
        self.queue.front()
    }

    /// 唯一出队口（拿到终值之后调用）
    pub fn pop_next(&mut self) -> Option<QueuedTask> {
        self.queue.pop_front()
    }

    /// 剩余任务数（含正在执行的队首）
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// 队列是否为空
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一条 Goto（用 x 当可辨标识，便于断言顺序）
    fn goto(x: f32) -> Mission {
        Mission::Goto { x, y: 0.0 }
    }

    /// 队列里所有 Goto 的 x（直接读私有字段，不消费队列）
    fn xs(q: &MissionQueue) -> Vec<f32> {
        q.queue
            .iter()
            .map(|t| match &t.mission {
                Mission::Goto { x, .. } => *x,
                other => panic!("测试只放 Goto，实际 {other:?}"),
            })
            .collect()
    }

    /// 队列里所有编号
    fn ids(q: &MissionQueue) -> Vec<u64> {
        q.queue.iter().map(|t| t.id.0).collect()
    }

    /// #1 replace：旧队列被清空，新队列顺序保持
    #[test]
    fn test_replace_clears_and_installs() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0), goto(2.0)]);
        q.replace(vec![goto(3.0), goto(4.0)]);
        assert_eq!(xs(&q), vec![3.0, 4.0]);
        assert_eq!(q.len(), 2);
    }

    /// #2 replace(vec![])：队列空、head() == None、is_empty()
    #[test]
    fn test_replace_empty_is_cancel() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0)]);
        q.replace(vec![]);
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert!(q.head().is_none());
    }

    /// #3 append：追加在尾部，已有元素位置不变
    #[test]
    fn test_append_tail_keeps_order() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0)]);
        q.append(vec![goto(2.0), goto(3.0)]);
        assert_eq!(xs(&q), vec![1.0, 2.0, 3.0]);
    }

    /// #4 preempt 单条：新元素在队首，原队首退到第二位
    #[test]
    fn test_preempt_single_goes_head() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0), goto(2.0)]);
        q.preempt(vec![goto(9.0)]);
        assert_eq!(xs(&q), vec![9.0, 1.0, 2.0]);
        // head() 命中的就是队首那条
        assert_eq!(q.head().map(|t| t.id.0), ids(&q).first().copied());
    }

    /// #5 preempt 多条：批内 [0] 在队首，整批顺序保持
    #[test]
    fn test_preempt_batch_keeps_batch_order() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0)]);
        q.preempt(vec![goto(8.0), goto(9.0)]);
        assert_eq!(xs(&q), vec![8.0, 9.0, 1.0]);
    }

    /// #6 连续两次 preempt：后一次在前（后进先出），原始队列尾部不变
    #[test]
    fn test_preempt_twice_is_lifo() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0), goto(2.0)]);
        q.preempt(vec![goto(8.0)]);
        q.preempt(vec![goto(9.0)]);
        assert_eq!(xs(&q), vec![9.0, 8.0, 1.0, 2.0]);
    }

    /// #7 head 不移除：连调两次结果相同、len 不变
    #[test]
    fn test_head_does_not_remove() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0), goto(2.0)]);
        let a = q.head().map(|t| t.id);
        let b = q.head().map(|t| t.id);
        assert_eq!(a, b);
        assert!(a.is_some());
        assert_eq!(q.len(), 2);
        assert_eq!(xs(&q), vec![1.0, 2.0]);
    }

    /// #8 pop_next：取队首并移除；空队列返回 None
    #[test]
    fn test_pop_next_removes_head() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0), goto(2.0)]);
        let first = q.pop_next().expect("队首存在");
        assert_eq!(first.mission, goto(1.0));
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop_next().map(|t| t.mission), Some(goto(2.0)));
        assert!(q.pop_next().is_none());
        assert!(q.is_empty());
    }

    /// #9 len / is_empty：三种操作后计数正确
    #[test]
    fn test_len_and_is_empty() {
        let mut q = MissionQueue::default();
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
        q.append(vec![goto(1.0), goto(2.0)]);
        assert_eq!(q.len(), 2);
        assert!(!q.is_empty());
        q.replace(vec![goto(3.0)]);
        assert_eq!(q.len(), 1);
        q.clear();
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
    }

    /// #10 TaskId 分配：入队递增；replace 后更大；clear 不回退（不复用）
    #[test]
    fn test_task_id_allocation() {
        let mut q = MissionQueue::default();
        q.append(vec![goto(1.0), goto(2.0), goto(3.0)]);
        let first = ids(&q);
        assert_eq!(first.len(), 3);
        assert!(first.windows(2).all(|w| w[0] < w[1]), "入队顺序递增: {first:?}");
        assert_eq!(q.head().map(|t| t.id.0), first.first().copied());

        // replace 后新元素拿到更大的 id
        q.replace(vec![goto(4.0)]);
        let after_replace = ids(&q)[0];
        assert!(after_replace > *first.last().unwrap(), "replace 后更大: {after_replace}");

        // clear 后 id 不回退（不复用）
        q.clear();
        q.append(vec![goto(5.0)]);
        let after_clear = ids(&q)[0];
        assert!(after_clear > after_replace, "clear 后不回退: {after_clear}");
    }
}
