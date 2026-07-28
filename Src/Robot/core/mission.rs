//Presented by KeJi
//Created Date ： 2026-07-28
//Modified Date ： 2026-07-28

//! Mission 任务队列

use crate::robot::core::command::Mission;

/// FIFO 任务队列
#[derive(Debug, Clone, Default)]
pub struct MissionQueue {
    queue: Vec<Mission>,
}

impl MissionQueue {
    /// 追加任务列表
    pub fn push(&mut self, missions: Vec<Mission>) {
        self.queue.extend(missions);
    }

    /// 取出下一个任务
    pub fn pop_next(&mut self) -> Option<Mission> {
        if self.queue.is_empty() {
            None
        } else {
            Some(self.queue.remove(0))
        }
    }

    /// 清空所有任务
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// 队列是否为空
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// 查询队首任务（不移除）
    pub fn peek(&self) -> Option<&Mission> {
        self.queue.first()
    }

    /// 剩余任务数
    pub fn len(&self) -> usize {
        self.queue.len()
    }
}
