//Presented by KeJi
//Date ： 2026-04-24

//! EventBus 核心实现
//!
//! 提供 `EventBus` 结构体，对 `tokio::sync::broadcast` 的薄封装。
//! 支持多生产者多消费者的事件广播。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use tokio::sync::broadcast;

use super::event::{Bus_Event, NotifyLevel};

// ============================================================
// EventBus 结构体
// ============================================================

/// EventBus — 全局事件总线
///
/// 独立于任何组件之外，通过 `tokio::sync::broadcast` 实现
/// 多生产者多消费者的事件广播。
///
/// ## 使用方式
/// - 启动时创建 `Arc<EventBus>`，分发给各组件
/// - 生产者调用 `Publish()` 发布事件
/// - 消费者调用 `Subscribe()` 获取接收端
///
/// ## 线程安全
/// `broadcast::Sender` 本身是 `Send + Sync`，因此 `EventBus`
/// 可以安全地用 `Arc` 包装在多线程环境中共享。
pub struct EventBus {
    sender: broadcast::Sender<Bus_Event>,
}

impl EventBus {
    /// 创建新的 EventBus 实例
    ///
    /// # 参数
    /// - `capacity`: broadcast channel 的缓冲区大小。
    ///   建议设为 1024，以降低慢消费者丢失消息（Lagged）的风险。
    pub fn New(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// 向总线发布一个事件
    ///
    /// 所有当前订阅者将收到此事件的克隆副本。
    ///
    /// - 如果当前没有订阅者，事件被静默丢弃
    /// - 此方法是**同步的**，可在同步和异步上下文中调用
    pub fn Publish(&self, event: Bus_Event) {
        let _ = self.sender.send(event);
    }

    /// 创建一个新的订阅者（接收端）
    ///
    /// 每次调用返回一个独立的 `broadcast::Receiver`，
    /// 订阅者从**调用 Subscribe 之后**的事件开始接收。
    ///
    /// ## 慢消费者处理
    /// 当消费者处理速度跟不上生产速度时，会收到
    /// `RecvError::Lagged(n)` 错误，表示丢失了 n 条消息。
    /// 消费者应记录警告日志并继续处理后续事件。
    pub fn Subscribe(&self) -> broadcast::Receiver<Bus_Event> {
        self.sender.subscribe()
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_publish_and_subscribe() {
        let bus = EventBus::New(16);
        let mut rx = bus.Subscribe();

        bus.Publish(Bus_Event::Notify {
            level: NotifyLevel::Info,
            message: "hello".to_string(),
        });

        let event = rx.recv().await.unwrap();
        match event {
            Bus_Event::Notify { level: _, message } => assert_eq!(message, "hello"),
            _ => panic!("unexpected event variant"),
        }
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = EventBus::New(16);
        let mut rx1 = bus.Subscribe();
        let mut rx2 = bus.Subscribe();

        bus.Publish(Bus_Event::State {
            payload: serde_json::json!({"type":"peer_discovered","peer_id":"peer-1"}).to_string(),
        });

        // 两个订阅者都应收到事件副本
        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();

        match (&e1, &e2) {
            (Bus_Event::State { .. }, Bus_Event::State { .. }) => {}
            _ => panic!("unexpected event variants"),
        }
    }

    #[tokio::test]
    async fn test_no_subscriber_no_panic() {
        let bus = EventBus::New(16);
        // 没有订阅者时发布不应 panic
        bus.Publish(Bus_Event::Notify {
            level: NotifyLevel::Error,
            message: "test error".to_string(),
        });
    }

    #[tokio::test]
    async fn test_subscribe_after_publish_misses_event() {
        let bus = EventBus::New(16);

        // 先发布，后订阅
        bus.Publish(Bus_Event::Notify {
            level: NotifyLevel::Info,
            message: "before subscribe".to_string(),
        });

        let mut rx = bus.Subscribe();

        // 再发布一条
        bus.Publish(Bus_Event::Notify {
            level: NotifyLevel::Info,
            message: "after subscribe".to_string(),
        });

        // 订阅者只应收到订阅之后的事件
        let event = rx.recv().await.unwrap();
        match event {
            Bus_Event::Notify { level: _, message } => assert_eq!(message, "after subscribe"),
            _ => panic!("unexpected event variant"),
        }
    }
}
