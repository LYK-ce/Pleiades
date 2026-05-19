//Presented by KeJi
//Date ： 2026-05-19

//! EventBus 模块集成测试
//!
//! 验证 EventBus 在高并发和慢消费者场景下的行为。
//! 每个测试使用独立的 EventBus 实例，测试间互不干扰。

#![allow(non_snake_case)]

mod common;

use pleiades::event_bus::{EventBus, Bus_Event, NotifyLevel};
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

/// TC-01: 多生产者多消费者
///
/// 5 个生产者各发 100 条 → 3 个消费者各收 500 条
#[tokio::test]
async fn tc01_multi_producer_multi_consumer() {
    let bus = Arc::new(EventBus::New(1024));
    let producer_count = 5usize;
    let messages_per_producer = 100usize;
    let consumer_count = 3usize;
    let total_messages = producer_count * messages_per_producer;

    // 创建消费者（必须在生产者发布前订阅）
    let mut consumer_handles = Vec::new();
    for consumer_id in 0..consumer_count {
        let mut rx = bus.Subscribe();
        let handle = tokio::spawn(async move {
            let mut received_count = 0usize;
            loop {
                match rx.recv().await {
                    Ok(_event) => {
                        received_count += 1;
                        if received_count >= total_messages {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        panic!(
                            "Consumer {} unexpectedly lagged by {} messages",
                            consumer_id, n
                        );
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            received_count
        });
        consumer_handles.push(handle);
    }

    // 创建生产者
    let mut producer_handles = Vec::new();
    for producer_id in 0..producer_count {
        let bus_clone = Arc::clone(&bus);
        let handle = tokio::spawn(async move {
            for msg_idx in 0..messages_per_producer {
                bus_clone.Publish(Bus_Event::Notify {
                    level: NotifyLevel::Info,
                    message: format!("producer_{}_msg_{}", producer_id, msg_idx),
                });
            }
        });
        producer_handles.push(handle);
    }

    // 等待所有生产者完成
    for handle in producer_handles {
        handle.await.unwrap();
    }

    // 等待所有消费者完成（5秒超时）
    for (i, handle) in consumer_handles.into_iter().enumerate() {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            handle,
        )
        .await;
        match result {
            Ok(Ok(count)) => {
                assert_eq!(
                    count, total_messages,
                    "Consumer {} 应收到全部 {} 条消息",
                    i, total_messages
                );
            }
            Ok(Err(e)) => panic!("Consumer {} panicked: {:?}", i, e),
            Err(_) => panic!("Consumer {} timed out", i),
        }
    }
}

/// TC-02: 慢消费者 Lagged 恢复
///
/// EventBus(16) 小缓冲 → 快速发 100 条 → 消费者遇 Lagged → 验证可继续接收
#[tokio::test]
async fn tc02_slow_consumer_lagged_recovery() {
    let bus = Arc::new(EventBus::New(16)); // 小缓冲
    let mut rx = bus.Subscribe();
    let publish_count = 100usize;

    // 快速发布 100 条（不等待消费者）
    for i in 0..publish_count {
        bus.Publish(Bus_Event::Notify {
            level: NotifyLevel::Info,
            message: format!("fast_msg_{}", i),
        });
    }

    // 消费者尝试接收：应遇到 Lagged，然后能继续接收后续消息
    let mut lagged_count = 0u64;
    let mut received_count = 0usize;
    let mut encountered_lagged = false;

    loop {
        match rx.try_recv() {
            Ok(_event) => {
                received_count += 1;
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                lagged_count += n;
                encountered_lagged = true;
                continue;
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
        }
    }

    assert!(
        encountered_lagged,
        "小缓冲(16) + 快速发 100 条应导致 Lagged"
    );
    assert!(
        received_count > 0,
        "Lagged 后仍应能接收到部分消息，实际收到: {}",
        received_count
    );
    assert!(
        (received_count as u64 + lagged_count) == publish_count as u64,
        "收到的消息({}) + 丢失的消息({}) 应等于发布总数({})",
        received_count,
        lagged_count,
        publish_count
    );

    // 在 Lagged 恢复后，新发布的消息应能正常接收
    bus.Publish(Bus_Event::Notify {
        level: NotifyLevel::Info,
        message: "after_recovery".to_string(),
    });
    let event = rx.recv().await.unwrap();
    match event {
        Bus_Event::Notify { level, message } => {
            assert!(matches!(level, NotifyLevel::Info));
            assert_eq!(message, "after_recovery", "Lagged 恢复后应能接收新消息");
        }
        _ => panic!("期望 Notify 事件"),
    }
}

/// TC-03: 全部 4 种 Bus_Event 类型
///
/// 逐一发布 4 种类型 → 消费者 match 验证
#[tokio::test]
async fn tc03_all_event_types() {
    let bus = EventBus::New(64);
    let mut rx = bus.Subscribe();

    // Notify
    bus.Publish(Bus_Event::Notify {
        level: NotifyLevel::Error,
        message: "test error".to_string(),
    });

    // State
    bus.Publish(Bus_Event::State {
        payload: serde_json::json!({"type":"peer_discovered","peer_id":"peer-1"}).to_string(),
    });

    // Stream
    bus.Publish(Bus_Event::Stream {
        payload: serde_json::json!({"type":"token","text":"Hello"}).to_string(),
    });

    // Output
    bus.Publish(Bus_Event::Output {
        payload: serde_json::json!({"type":"cmd_result","text":"done","completed":true}).to_string(),
    });

    // Receive Notify
    let event = rx.recv().await.unwrap();
    match event {
        Bus_Event::Notify { level, message } => {
            assert!(matches!(level, NotifyLevel::Error));
            assert_eq!(message, "test error");
        }
        _ => panic!("期望 Notify"),
    }

    // Receive State
    let event = rx.recv().await.unwrap();
    match event {
        Bus_Event::State { payload } => {
            let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(v["type"], "peer_discovered");
            assert_eq!(v["peer_id"], "peer-1");
        }
        _ => panic!("期望 State"),
    }

    // Receive Stream
    let event = rx.recv().await.unwrap();
    match event {
        Bus_Event::Stream { payload } => {
            let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(v["type"], "token");
            assert_eq!(v["text"], "Hello");
        }
        _ => panic!("期望 Stream"),
    }

    // Receive Output
    let event = rx.recv().await.unwrap();
    match event {
        Bus_Event::Output { payload } => {
            let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(v["type"], "cmd_result");
            assert_eq!(v["text"], "done");
            assert_eq!(v["completed"], true);
        }
        _ => panic!("期望 Output"),
    }
}

/// TC-04: Subscribe 后发布才能接收
///
/// 发布 → Subscribe → 再发布 → 仅收到后者
#[tokio::test]
async fn tc04_subscribe_after_publish_only_receives_later() {
    let bus = EventBus::New(64);

    // 先发布一条（无订阅者）
    bus.Publish(Bus_Event::Notify {
        level: NotifyLevel::Info,
        message: "before_subscribe".to_string(),
    });

    // 订阅
    let mut rx = bus.Subscribe();

    // 再发布一条
    bus.Publish(Bus_Event::Notify {
        level: NotifyLevel::Info,
        message: "after_subscribe".to_string(),
    });

    // 消费者应只收到订阅之后的事件
    let event = rx.recv().await.unwrap();
    match event {
        Bus_Event::Notify { level: _, message } => {
            assert_eq!(
                message, "after_subscribe",
                "应只收到 Subscribe 之后发布的事件"
            );
        }
        _ => panic!("期望 Notify 事件"),
    }

    // 验证没有更多消息（try_recv 应返回 Empty）
    match rx.try_recv() {
        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
            // 预期行为
        }
        other => panic!("期望 Empty，实际: {:?}", other),
    }
}
