//Presented by KeJi
//Date ： 2026-04-29

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 预留表共享类型别名 — StorageManager 和 Reservation 共同持有
pub(crate) type ReservationMap = Arc<Mutex<HashMap<String, u64>>>;

/// 空间预留令牌 — 持有即占用配额，Drop 时自动释放预留
///
/// 生命周期:
///   reserve() → acquire_write() → 写入文件 → commit()
///
/// 异常路径:
///   reserve() → Drop（自动释放预留，不修改磁盘）
pub struct Reservation {
    /// 预留的文件 ID
    pub(crate) file_id: String,
    /// 预留的字节数
    pub(crate) size: u64,
    /// 是否已提交（commit 后置为 true，阻止 Drop 重复释放）
    committed: AtomicBool,
    /// 共享预留表引用（与 StorageManager 共享）
    pub(crate) map: ReservationMap,
}

impl Reservation {
    /// 创建新的预留令牌
    pub(crate) fn New(file_id: String, size: u64, map: ReservationMap) -> Self {
        Self {
            file_id,
            size,
            committed: AtomicBool::new(false),
            map,
        }
    }

    /// 返回预留的文件 ID
    pub fn File_Id(&self) -> &str {
        &self.file_id
    }

    /// 返回预留的字节数
    pub fn Size(&self) -> u64 {
        self.size
    }

    /// 标记为已提交，阻止 Drop 释放预留
    pub(crate) fn Mark_Committed(&self) {
        self.committed.store(true, Ordering::Release);
    }

    /// 检查是否已提交
    pub fn Is_Committed(&self) -> bool {
        self.committed.load(Ordering::Acquire)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.committed.load(Ordering::Acquire) {
            // 异常路径：预留未提交，归还配额
            if let Ok(mut map) = self.map.lock() {
                map.remove(&self.file_id);
            }
        }
    }
}

// Reservation 需要 Send + Sync 以便跨异步边界传递
// AtomicBool 和 Arc<Mutex<_>> 都是 Send + Sync 的
// SAFETY: 所有内部字段均为 Send + Sync
unsafe impl Send for Reservation {}
unsafe impl Sync for Reservation {}

impl std::fmt::Debug for Reservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reservation")
            .field("file_id", &self.file_id)
            .field("size", &self.size)
            .field("committed", &self.committed.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reservation_file_id_and_size() {
        let map: ReservationMap = Arc::new(Mutex::new(HashMap::new()));
        map.lock().unwrap().insert("test.gguf".to_string(), 1024);
        let reservation = Reservation::New("test.gguf".to_string(), 1024, Arc::clone(&map));
        assert_eq!(reservation.File_Id(), "test.gguf");
        assert_eq!(reservation.Size(), 1024);
        assert!(!reservation.Is_Committed());
    }

    #[test]
    fn test_reservation_mark_committed() {
        let map: ReservationMap = Arc::new(Mutex::new(HashMap::new()));
        map.lock().unwrap().insert("test.gguf".to_string(), 1024);
        let reservation = Reservation::New("test.gguf".to_string(), 1024, Arc::clone(&map));
        reservation.Mark_Committed();
        assert!(reservation.Is_Committed());
        // Drop 后不应移除条目
        drop(reservation);
        assert!(map.lock().unwrap().contains_key("test.gguf"));
    }

    #[test]
    fn test_reservation_drop_releases_uncommitted() {
        let map: ReservationMap = Arc::new(Mutex::new(HashMap::new()));
        map.lock().unwrap().insert("model.gguf".to_string(), 4_000_000_000);
        {
            let _reservation = Reservation::New(
                "model.gguf".to_string(),
                4_000_000_000,
                Arc::clone(&map),
            );
            assert!(map.lock().unwrap().contains_key("model.gguf"));
            // _reservation drops here without commit
        }
        // Drop 后条目应被移除
        assert!(!map.lock().unwrap().contains_key("model.gguf"));
    }

    #[test]
    fn test_reservation_drop_committed_does_not_release() {
        let map: ReservationMap = Arc::new(Mutex::new(HashMap::new()));
        map.lock().unwrap().insert("model.gguf".to_string(), 2048);
        {
            let reservation = Reservation::New(
                "model.gguf".to_string(),
                2048,
                Arc::clone(&map),
            );
            reservation.Mark_Committed();
            // donation drops here after commit
        }
        // 已提交的预留不应被 Drop 移除
        assert!(map.lock().unwrap().contains_key("model.gguf"));
    }

    #[test]
    fn test_reservation_debug_format() {
        let map: ReservationMap = Arc::new(Mutex::new(HashMap::new()));
        let reservation = Reservation::New("test.gguf".to_string(), 512, map);
        let debug_str = format!("{:?}", reservation);
        assert!(debug_str.contains("test.gguf"));
        assert!(debug_str.contains("512"));
    }
}
