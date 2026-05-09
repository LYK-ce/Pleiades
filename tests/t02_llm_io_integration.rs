//Presented by KeJi
//Date ： 2026-04-27

//! LLM_IO 模块集成测试
//!
//! 验证 LLM_IO_Broker 在多 Job 并发场景下的通道隔离和生命周期。
//! 每个测试使用独立的 Broker 实例，测试间互不干扰。

#![allow(non_snake_case)]

mod common;

use pleiades::llm_io::{LLM_IO_Broker, LLM_IO_Capability, LLM_IO_Error};
use pleiades::orchestrator::job::JobId;
use std::sync::Arc;

/// TC-01: 多 Job 并发分配与隔离
///
/// 并发 Allocate 10 个 JobId → Take 两端 → 每个 Job frontend 发唯一 prompt
/// → ml_side 收 → 验证无串台
#[tokio::test]
async fn tc01_multi_job_concurrent_isolation() {
    let broker = Arc::new(LLM_IO_Broker::New());
    let job_count = 10u64;

    // 1. 分配 10 个 Job 的通道并 Take 两端
    let mut frontends = Vec::new();
    let mut ml_sides = Vec::new();

    for i in 0..job_count {
        broker.Allocate(JobId(i)).await.unwrap();
        frontends.push(broker.Take_Frontend(JobId(i)).await.unwrap());
        ml_sides.push(broker.Take_ML_Side(JobId(i)).await.unwrap());
    }

    // 2. 每个 frontend 发送唯一 prompt
    for (i, frontend) in frontends.iter().enumerate() {
        let prompt = format!("prompt_from_job_{}", i);
        frontend.input_tx.send(prompt).await.unwrap();
    }

    // 3. 每个 ml_side 接收 → 验证无串台
    for (i, ml_side) in ml_sides.iter_mut().enumerate() {
        let received = ml_side.input_rx.recv().await.unwrap();
        let expected = format!("prompt_from_job_{}", i);
        assert_eq!(
            received, expected,
            "Job {} 收到的 prompt 应与发送的一致，验证无串台",
            i
        );
    }

    // 4. 每个 ml_side 回复，frontend 接收验证
    for (i, ml_side) in ml_sides.iter().enumerate() {
        let reply = format!("reply_from_ml_{}", i);
        ml_side.output_tx.send(reply).await.unwrap();
    }

    for (i, frontend) in frontends.iter_mut().enumerate() {
        let received = frontend.output_rx.recv().await.unwrap();
        let expected = format!("reply_from_ml_{}", i);
        assert_eq!(
            received, expected,
            "Job {} frontend 收到的回复应与 ml_side 发送的一致",
            i
        );
    }
}

/// TC-02: 通道关闭传播
///
/// Allocate → Take → 双向通信 → Deallocate → drop frontend → ml_side.recv() = None
#[tokio::test]
async fn tc02_channel_close_propagation() {
    let broker = LLM_IO_Broker::New();
    let job_id = JobId(200);

    // 1. 分配通道并 Take 两端
    broker.Allocate(job_id).await.unwrap();
    let frontend = broker.Take_Frontend(job_id).await.unwrap();
    let mut ml_side = broker.Take_ML_Side(job_id).await.unwrap();

    // 前端发送
    frontend.input_tx.send("ping".to_string()).await.unwrap();
    let received = ml_side.input_rx.recv().await.unwrap();
    assert_eq!(received, "ping");

    // ML 侧回复
    ml_side.output_tx.send("pong".to_string()).await.unwrap();

    // 2. Deallocate（移除 ChannelEntry 中的 Sender 副本）
    broker.Deallocate(job_id).await.unwrap();
    assert!(!broker.Is_Active(job_id).await);

    // 3. drop frontend（移除最后一个 input_tx）
    drop(frontend);

    // 4. ml_side.input_rx.recv() 应返回 None（通道关闭）
    let result = ml_side.input_rx.recv().await;
    assert!(
        result.is_none(),
        "Deallocate + drop frontend 后，ml_side.input_rx.recv() 应返回 None"
    );
}

/// TC-03: 高速吞吐
///
/// Allocate + Take 后连续发 1000 条消息 → 验证全部到达且顺序正确
#[tokio::test]
async fn tc03_high_speed_throughput() {
    let broker = LLM_IO_Broker::New();
    let job_id = JobId(300);
    let message_count = 1000usize;

    broker.Allocate(job_id).await.unwrap();
    let frontend = broker.Take_Frontend(job_id).await.unwrap();
    let mut ml_side = broker.Take_ML_Side(job_id).await.unwrap();

    // 1. 生产者 — frontend 连续发 1000 条
    let sender = frontend.input_tx.clone();
    let producer = tokio::spawn(async move {
        for i in 0..message_count {
            sender.send(format!("msg_{}", i)).await.unwrap();
        }
    });

    // 2. 消费者 — ml_side 接收并验证顺序
    let consumer = tokio::spawn(async move {
        let mut received_messages = Vec::with_capacity(message_count);
        for _ in 0..message_count {
            let msg = ml_side.input_rx.recv().await.unwrap();
            received_messages.push(msg);
        }
        received_messages
    });

    producer.await.unwrap();
    let received = consumer.await.unwrap();

    assert_eq!(received.len(), message_count, "应收到全部 1000 条消息");

    // 验证顺序正确
    for (i, msg) in received.iter().enumerate() {
        let expected = format!("msg_{}", i);
        assert_eq!(
            msg, &expected,
            "第 {} 条消息顺序应正确",
            i
        );
    }
}

/// TC-04: 并发分配不同 JobId 无竞争
///
/// spawn 20 个任务同时 Allocate 不同 JobId → 全部成功 → Is_Active 全部 true
#[tokio::test]
async fn tc04_concurrent_allocate_no_contention() {
    let broker = Arc::new(LLM_IO_Broker::New());
    let task_count = 20u64;

    // spawn 20 个并发 Allocate 任务
    let mut handles = Vec::new();
    for i in 0..task_count {
        let broker_clone = Arc::clone(&broker);
        let handle = tokio::spawn(async move {
            let result = broker_clone.Allocate(JobId(400 + i)).await;
            (i, result)
        });
        handles.push(handle);
    }

    // 等待全部完成
    for handle in handles {
        let (i, result) = handle.await.unwrap();
        assert!(
            result.is_ok(),
            "JobId({}) 并发 Allocate 应成功",
            400 + i
        );
    }

    // 验证 Is_Active 全部 true
    for i in 0..task_count {
        assert!(
            broker.Is_Active(JobId(400 + i)).await,
            "JobId({}) 应处于活跃状态",
            400 + i
        );
    }
}

/// TC-05: 重复分配同一 JobId 错误处理
///
/// Allocate(job_1) → 再 Allocate(job_1) → AllocationFailed
#[tokio::test]
async fn tc05_duplicate_allocate_error() {
    let broker = LLM_IO_Broker::New();
    let job_id = JobId(500);

    // 第一次分配成功
    broker.Allocate(job_id).await.unwrap();
    assert!(broker.Is_Active(job_id).await);

    // 第二次分配同一 JobId 应失败
    let result = broker.Allocate(job_id).await;
    assert!(result.is_err(), "重复 Allocate 同一 JobId 应返回错误");

    match result.unwrap_err() {
        LLM_IO_Error::AllocationFailed(msg) => {
            assert!(
                msg.contains("already has an active channel"),
                "错误信息应包含 'already has an active channel'，实际: {}",
                msg
            );
        }
        other => panic!("Expected AllocationFailed, got {:?}", other),
    }
}
