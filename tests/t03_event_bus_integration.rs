//Presented by KeJi
//Date ： 2026-04-24

//! EventBus 模块集成测试
//!
//! 验证 EventBus 在高并发和慢消费者场景下的行为。
//! 每个测试使用独立的 EventBus 实例，测试间互不干扰。

#![allow(non_snake_case)]

mod common;

use pleiades::event_bus::{EventBus, Bus_Event};
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
                        // 在大缓冲下不应发生 Lagged
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
                bus_clone.Publish(Bus_Event::Log {
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
        bus.Publish(Bus_Event::Log {
            message: format!("fast_msg_{}", i),
        });
    }

    // 消费者尝试接收：应遇到 Lagged，然后能继续接收后续消息
    let mut lagged_count = 0u64;
    let mut received_count = 0usize;
    let mut encountered_lagged = false;

    // 尝试读取所有可用消息
    loop {
        match rx.try_recv() {
            Ok(_event) => {
                received_count += 1;
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                lagged_count += n;
                encountered_lagged = true;
                // Lagged 后继续接收
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
    bus.Publish(Bus_Event::Log {
        message: "after_recovery".to_string(),
    });
    let event = rx.recv().await.unwrap();
    match event {
        Bus_Event::Log { message } => {
            assert_eq!(message, "after_recovery", "Lagged 恢复后应能接收新消息");
        }
        _ => panic!("期望 Log 事件"),
    }
}

/// TC-03: 全部 Bus_Event 变体
///
/// 逐一发布每种 Bus_Event 变体 → 消费者 match 验证
#[tokio::test]
async fn tc03_all_bus_event_variants() {
    let bus = EventBus::New(64);
    let mut rx = bus.Subscribe();

    // 定义所有 14 种 variant
    let all_events = vec![
        Bus_Event::Peer_Discovered {
            peer_id: "peer-1".to_string(),
        },
        Bus_Event::Peer_Left {
            peer_id: "peer-2".to_string(),
        },
        Bus_Event::Connection_Established {
            peer_id: "peer-3".to_string(),
        },
        Bus_Event::Connection_Closed {
            peer_id: "peer-4".to_string(),
        },
        Bus_Event::Job_Created {
            job_id: 1,
            kind: "Run".to_string(),
            model_name: "qwen3".to_string(),
        },
        Bus_Event::Job_State_Changed {
            job_id: 1,
            phase: "Executing".to_string(),
        },
        Bus_Event::Job_Completed {
            job_id: 1,
            result: "Success".to_string(),
        },
        Bus_Event::Inference_Started {
            job_id: 2,
            model_name: "qwen3".to_string(),
            device_count: 3,
            layer_range: "0-15".to_string(),
        },
        Bus_Event::Inference_Token {
            job_id: 2,
            token: "Hello".to_string(),
        },
        Bus_Event::Inference_Completed {
            job_id: 2,
            text: "Hello World".to_string(),
            tokens: 2,
            tok_per_sec: 10.5,
            total_secs: 0.19,
        },
        Bus_Event::File_Progress {
            file_name: "model.gguf".to_string(),
            direction: "send".to_string(),
            peer: "peer-5".to_string(),
            sent: 512,
            total: 1024,
        },
        Bus_Event::Log {
            message: "test log".to_string(),
        },
        Bus_Event::Error {
            message: "test error".to_string(),
        },
        Bus_Event::Device_Changed {
            device: "cuda".to_string(),
        },
    ];

    let expected_count = all_events.len();

    // 发布所有事件
    for event in &all_events {
        bus.Publish(event.clone());
    }

    // 接收并用 match 验证每个 variant
    let mut variant_names = Vec::new();
    for _ in 0..expected_count {
        let event = rx.recv().await.unwrap();
        let name = match &event {
            Bus_Event::Peer_Discovered { peer_id } => {
                assert_eq!(peer_id, "peer-1");
                "Peer_Discovered"
            }
            Bus_Event::Peer_Left { peer_id } => {
                assert_eq!(peer_id, "peer-2");
                "Peer_Left"
            }
            Bus_Event::Connection_Established { peer_id } => {
                assert_eq!(peer_id, "peer-3");
                "Connection_Established"
            }
            Bus_Event::Connection_Closed { peer_id } => {
                assert_eq!(peer_id, "peer-4");
                "Connection_Closed"
            }
            Bus_Event::Job_Created { job_id, kind, model_name } => {
                assert_eq!(*job_id, 1);
                assert_eq!(kind, "Run");
                assert_eq!(model_name, "qwen3");
                "Job_Created"
            }
            Bus_Event::Job_State_Changed { job_id, phase } => {
                assert_eq!(*job_id, 1);
                assert_eq!(phase, "Executing");
                "Job_State_Changed"
            }
            Bus_Event::Job_Completed { job_id, result } => {
                assert_eq!(*job_id, 1);
                assert_eq!(result, "Success");
                "Job_Completed"
            }
            Bus_Event::Inference_Started { job_id, model_name, device_count, layer_range } => {
                assert_eq!(*job_id, 2);
                assert_eq!(model_name, "qwen3");
                assert_eq!(*device_count, 3);
                assert_eq!(layer_range, "0-15");
                "Inference_Started"
            }
            Bus_Event::Inference_Token { job_id, token } => {
                assert_eq!(*job_id, 2);
                assert_eq!(token, "Hello");
                "Inference_Token"
            }
            Bus_Event::Inference_Completed { job_id, text, tokens, tok_per_sec, total_secs } => {
                assert_eq!(*job_id, 2);
                assert_eq!(text, "Hello World");
                assert_eq!(*tokens, 2);
                assert!(*tok_per_sec > 10.0);
                assert!(*total_secs < 1.0);
                "Inference_Completed"
            }
            Bus_Event::File_Progress { file_name, direction, peer, sent, total } => {
                assert_eq!(file_name, "model.gguf");
                assert_eq!(direction, "send");
                assert_eq!(peer, "peer-5");
                assert_eq!(*sent, 512);
                assert_eq!(*total, 1024);
                "File_Progress"
            }
            Bus_Event::Log { message } => {
                assert_eq!(message, "test log");
                "Log"
            }
            Bus_Event::Error { message } => {
                assert_eq!(message, "test error");
                "Error"
            }
            Bus_Event::Device_Changed { device } => {
                assert_eq!(device, "cuda");
                "Device_Changed"
            }
            _ => continue,
        };
        variant_names.push(name.to_string());
    }

    assert_eq!(
        variant_names.len(),
        expected_count,
        "应接收到全部 {} 种 variant",
        expected_count
    );
}

/// TC-04: Subscribe 后发布才能接收
///
/// 发布 → Subscribe → 再发布 → 仅收到后者
#[tokio::test]
async fn tc04_subscribe_after_publish_only_receives_later() {
    let bus = EventBus::New(64);

    // 先发布一条（无订阅者）
    bus.Publish(Bus_Event::Log {
        message: "before_subscribe".to_string(),
    });

    // 订阅
    let mut rx = bus.Subscribe();

    // 再发布一条
    bus.Publish(Bus_Event::Log {
        message: "after_subscribe".to_string(),
    });

    // 消费者应只收到订阅之后的事件
    let event = rx.recv().await.unwrap();
    match event {
        Bus_Event::Log { message } => {
            assert_eq!(
                message, "after_subscribe",
                "应只收到 Subscribe 之后发布的事件"
            );
        }
        _ => panic!("期望 Log 事件"),
    }

    // 验证没有更多消息（try_recv 应返回 Empty）
    match rx.try_recv() {
        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
            // 预期行为
        }
        other => panic!("期望 Empty，实际: {:?}", other),
    }
}
