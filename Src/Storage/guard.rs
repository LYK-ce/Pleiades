//Presented by KeJi
//Created Date ： 2026-04-23
//Modified Date ： 2026-06-15

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard};
use std::fmt;

/// 读锁守卫 — 持有期间文件不会被 remove / acquire_write 修改
///
/// 多个 ReadGuard 可并发共存（共享读）。
/// Drop 时自动释放锁。
#[derive(Debug)]
pub struct ReadGuard {
    pub(crate) file_id: String,
    pub(crate) _guard: OwnedRwLockReadGuard<()>,
}

/// 写锁守卫 — 持有期间文件独占
///
/// 同一时刻只能有一个 WriteGuard（排他写）。
/// Drop 时先执行 on_drop 回调（stat 并更新文件 size），再释放写锁。
pub struct WriteGuard {
    pub(crate) file_id: String,
    pub(crate) on_drop: Option<Box<dyn FnOnce() + Send>>,
    pub(crate) _guard: OwnedRwLockWriteGuard<()>,
}

impl fmt::Debug for WriteGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteGuard")
            .field("file_id", &self.file_id)
            .finish()
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        // on_drop 先执行：stat + 更新 size（持有写锁保护）
        if let Some(cb) = self.on_drop.take() {
            cb();
        }
        // _guard 后析构 → 释放写锁
    }
}

impl ReadGuard {
    /// 返回受保护的 file_id
    pub fn file_id(&self) -> &str {
        &self.file_id
    }
}

impl WriteGuard {
    /// 返回受保护的 file_id
    pub fn file_id(&self) -> &str {
        &self.file_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn test_read_guard_file_id() {
        let lock = Arc::new(RwLock::new(()));
        let guard = lock.read_owned().await;
        let read_guard = ReadGuard {
            file_id: "test.txt".to_string(),
            _guard: guard,
        };
        assert_eq!(read_guard.file_id(), "test.txt");
    }

    #[tokio::test]
    async fn test_write_guard_file_id() {
        let lock = Arc::new(RwLock::new(()));
        let guard = Arc::clone(&lock).write_owned().await;
        let write_guard = WriteGuard {
            file_id: "test.txt".to_string(),
            _guard: guard,
            on_drop: None,
        };
        assert_eq!(write_guard.file_id(), "test.txt");
    }

    #[tokio::test]
    async fn test_guard_drop_releases_lock() {
        let lock = Arc::new(RwLock::new(()));
        let guard = Arc::clone(&lock).write_owned().await;
        let write_guard = WriteGuard {
            file_id: "test.txt".to_string(),
            _guard: guard,
            on_drop: None,
        };
        // 写锁存在时 try_write 应失败
        assert!(lock.try_write().is_err());
        drop(write_guard);
        // Drop 后 try_write 应成功
        assert!(lock.try_write().is_ok());
    }
}
