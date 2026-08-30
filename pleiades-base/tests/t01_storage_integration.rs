//Presented by KeJi
//Date ： 2026-05-17

//! Storage 模块集成测试
//!
//! 从外部消费者视角验证 Storage 生命周期。
//! 每个测试使用独立的 TempDir，测试间互不干扰。

mod common;

use pleiades_base::storage::{StorageManager, StorageCapability, StorageError, ChecksumAlgorithm};
use tempfile::TempDir;
use tokio::time::{timeout, Duration};

/// TC-01: 完整生命周期
///
/// New → acquire_write → 写入 100KB → drop → acquire_read → 读取验证
/// → checksum → remove → 验证磁盘删除
#[tokio::test]
async fn tc01_full_lifecycle() {
    let temp_dir = TempDir::new().unwrap();
    let manager = StorageManager::New(temp_dir.path()).await.unwrap();

    // 1. acquire_write → 写入 100KB 数据
    let data_100kb: Vec<u8> = (0u8..=255).cycle().take(100 * 1024).collect();
    let (write_path, write_guard) = manager.acquire_write("lifecycle.bin").await.unwrap();
    tokio::fs::write(&write_path, &data_100kb).await.unwrap();
    drop(write_guard);

    // 2. flush 刷新 size，然后 list 验证
    manager.flush().await.unwrap();

    // 3. acquire_read → 读取验证
    let (read_path, read_guard) = manager.acquire_read("lifecycle.bin").await.unwrap();
    let content = tokio::fs::read(&read_path).await.unwrap();
    assert_eq!(content.len(), 100 * 1024, "读取的数据长度应为 100KB");
    assert_eq!(content, data_100kb, "读取内容应与写入内容完全一致");
    drop(read_guard);

    // 4. checksum — 默认算法 (XxHash64) 返回格式正确
    let checksum = manager.checksum("lifecycle.bin", None).await.unwrap();
    assert!(
        checksum.starts_with("xxhash64:"),
        "默认校验码应以 xxhash64: 开头，实际: {}",
        checksum
    );
    assert!(checksum.len() > "xxhash64:".len(), "校验码应包含哈希值");

    // 5. remove → 验证索引和磁盘均已清除
    manager.remove("lifecycle.bin").await.unwrap();
    assert!(
        !manager.exists("lifecycle.bin").await.unwrap(),
        "remove 后 exists 应返回 false"
    );
    assert!(
        !temp_dir.path().join("lifecycle.bin").exists(),
        "remove 后磁盘文件应已删除"
    );
}

/// TC-02: 并发多读单写
///
/// 写入文件 → spawn 10 个并发 acquire_read → 全部成功
/// → 同时 spawn acquire_write 被阻塞 → drop 所有 ReadGuard → 写锁获得
#[tokio::test]
async fn tc02_concurrent_multi_read_single_write() {
    let temp_dir = TempDir::new().unwrap();
    let manager = StorageManager::New(temp_dir.path()).await.unwrap();

    // 准备文件
    let (write_path, write_guard) = manager.acquire_write("concurrent.bin").await.unwrap();
    tokio::fs::write(&write_path, b"concurrent_data").await.unwrap();
    drop(write_guard);

    let manager = std::sync::Arc::new(manager);

    // 1. spawn 10 个并发读取任务
    let mut read_guards = Vec::new();
    for _i in 0..10 {
        let (_, rg) = manager.acquire_read("concurrent.bin").await.unwrap();
        read_guards.push(rg);
    }
    assert_eq!(read_guards.len(), 10, "应成功获取 10 个读锁");

    // 2. 在读锁存活期间，acquire_write 应被阻塞
    let manager_clone = std::sync::Arc::clone(&manager);
    let write_future = tokio::spawn(async move {
        manager_clone.acquire_write("concurrent.bin").await
    });

    let write_result = timeout(Duration::from_millis(200), write_future).await;
    assert!(write_result.is_err(), "读锁存活时 acquire_write 应超时阻塞");

    // 3. drop 所有 ReadGuard → 写锁应能获得
    drop(read_guards);

    let manager_clone2 = std::sync::Arc::clone(&manager);
    let write_result2 = timeout(
        Duration::from_secs(5),
        tokio::spawn(async move {
            manager_clone2.acquire_write("concurrent.bin").await
        }),
    )
    .await;
    assert!(
        write_result2.is_ok(),
        "释放读锁后 acquire_write 应成功"
    );
    let write_ok = write_result2.unwrap().unwrap();
    assert!(write_ok.is_ok(), "写锁应成功获取");
}

/// TC-03: 初始化扫描
///
/// 预先在目录中创建 3 个文件 → New(dir) → list() 返回 3 个 → exists() 各自验证
#[tokio::test]
async fn tc03_init_scan() {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();

    // 预先创建 3 个文件
    let file_names = ["alpha.dat", "beta.dat", "gamma.dat"];
    for name in &file_names {
        let file_path = base_path.join(name);
        tokio::fs::write(&file_path, format!("content of {}", name))
            .await
            .unwrap();
    }

    // New(dir) 应扫描并建立索引
    let manager = StorageManager::New(base_path).await.unwrap();

    // list() 应返回 3 个文件 — 按 file_name 排序后比较
    let mut listed_names: Vec<String> = manager.list().await.unwrap()
        .into_iter()
        .map(|e| e.file_name)
        .collect();
    listed_names.sort();

    let mut expected: Vec<String> = file_names.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(listed_names, expected, "list() 应包含预创建的 3 个文件");

    // exists() 逐个验证
    for name in &file_names {
        assert!(
            manager.exists(name).await.unwrap(),
            "exists('{}') 应返回 true",
            name
        );
    }

    // 验证不存在的文件
    assert!(
        !manager.exists("nonexistent.dat").await.unwrap(),
        "不存在的文件 exists 应返回 false"
    );
}

/// TC-04: 大文件校验码一致性
///
/// 写入 1MB 文件 → 分别计算 XxHash64/Sha256/Blake3 → 重复读取验证一致
#[tokio::test]
async fn tc04_large_file_checksum_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let manager = StorageManager::New(temp_dir.path()).await.unwrap();

    // 写入 1MB 数据
    let data_1mb: Vec<u8> = (0u8..=255).cycle().take(1024 * 1024).collect();
    let (write_path, write_guard) = manager.acquire_write("large.bin").await.unwrap();
    tokio::fs::write(&write_path, &data_1mb).await.unwrap();
    drop(write_guard);

    let algorithms = [
        (ChecksumAlgorithm::XxHash64, "xxhash64:"),
        (ChecksumAlgorithm::Sha256, "sha256:"),
        (ChecksumAlgorithm::Blake3, "blake3:"),
    ];

    for (algo, expected_prefix) in &algorithms {
        let checksum_1 = manager
            .checksum("large.bin", Some(*algo))
            .await
            .unwrap();
        let checksum_2 = manager
            .checksum("large.bin", Some(*algo))
            .await
            .unwrap();

        assert!(
            checksum_1.starts_with(expected_prefix),
            "{:?} 校验码应以 '{}' 开头, 实际: {}",
            algo,
            expected_prefix,
            checksum_1
        );

        assert_eq!(
            checksum_1, checksum_2,
            "{:?} 两次计算的校验码应一致",
            algo
        );

        let hash_part = &checksum_1[expected_prefix.len()..];
        assert!(
            !hash_part.is_empty(),
            "{:?} 哈希值部分不应为空",
            algo
        );
    }

    // 三种算法的结果应互不相同
    let xxhash = manager
        .checksum("large.bin", Some(ChecksumAlgorithm::XxHash64))
        .await
        .unwrap();
    let sha256 = manager
        .checksum("large.bin", Some(ChecksumAlgorithm::Sha256))
        .await
        .unwrap();
    let blake3 = manager
        .checksum("large.bin", Some(ChecksumAlgorithm::Blake3))
        .await
        .unwrap();

    assert_ne!(xxhash, sha256, "XxHash64 和 Sha256 的校验码应不同");
    assert_ne!(xxhash, blake3, "XxHash64 和 Blake3 的校验码应不同");
    assert_ne!(sha256, blake3, "Sha256 和 Blake3 的校验码应不同");
}

/// TC-05: remove 竞争条件
///
/// 持有 ReadGuard → 另一任务 remove → 返回 InUse
/// → drop ReadGuard → 重新 remove → Ok
#[tokio::test]
async fn tc05_remove_race_condition() {
    let temp_dir = TempDir::new().unwrap();
    let manager = StorageManager::New(temp_dir.path()).await.unwrap();

    // 创建并写入文件
    let (write_path, write_guard) = manager.acquire_write("race.bin").await.unwrap();
    tokio::fs::write(&write_path, b"race_data").await.unwrap();
    drop(write_guard);

    // 1. 持有 ReadGuard
    let (_read_path, read_guard) = manager.acquire_read("race.bin").await.unwrap();

    // 2. 尝试 remove → 应返回 InUse
    let remove_result = manager.remove("race.bin").await;
    assert!(
        remove_result.is_err(),
        "持有 ReadGuard 时 remove 应失败"
    );
    match remove_result.unwrap_err() {
        StorageError::InUse(msg) => {
            assert!(
                msg.contains("race.bin"),
                "InUse 错误信息应包含文件名，实际: {}",
                msg
            );
        }
        other => panic!("期望 InUse 错误，实际: {:?}", other),
    }

    // 3. 确认文件仍然存在
    assert!(
        manager.exists("race.bin").await.unwrap(),
        "remove 失败后文件应仍然存在"
    );

    // 4. drop ReadGuard
    drop(read_guard);

    // 5. 重新 remove → 应成功
    manager.remove("race.bin").await.unwrap();
    assert!(
        !manager.exists("race.bin").await.unwrap(),
        "成功 remove 后 exists 应返回 false"
    );
    assert!(
        !temp_dir.path().join("race.bin").exists(),
        "成功 remove 后磁盘文件应已删除"
    );
}
