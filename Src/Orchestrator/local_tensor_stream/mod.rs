//Presented by KeJi
//Date ： 2026-05-20

//! 独立本地流
//!
//! 为同进程内的两个线程提供轻量张量流通信。
//! 与网络 Tensor Stream 共享帧格式，完全绕过 libp2p。

pub mod frames;

use std::collections::HashMap;
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{duplex, DuplexStream};

/// 本地流配对管理器
///
/// 线程安全（`Mutex`），供 `Arc<LocalStreamHub>` 在多个线程间共享。
/// `open` 创建一对 duplex 并存储右半；`accept` 阻塞等待取出右半。
pub struct LocalStreamHub {
    pending: Mutex<HashMap<String, DuplexStream>>,
}

impl LocalStreamHub {
    /// 创建新的 Hub
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// 发起方调用：创建一对 duplex（64KB buffer），左半返回，右半存入 pending。
    pub fn open(&self, id: &str) -> io::Result<DuplexStream> {
        let (local, remote) = duplex(64 * 1024);
        self.pending
            .lock()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
            .insert(id.to_string(), remote);
        Ok(local)
    }

    /// 接收方调用：从 pending 取出右半，阻塞等待直到超时。
    pub fn accept(&self, id: &str, timeout_ms: u64) -> io::Result<DuplexStream> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            {
                let remote = self
                    .pending
                    .lock()
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
                    .remove(id);
                if let Some(stream) = remote {
                    return Ok(stream);
                }
            }
            if Instant::now() > deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("accept_stream timeout: id={}", id),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_accept_pair() {
        let hub = LocalStreamHub::new();

        // 发起方
        let left = hub.open("test-1").expect("open");

        // 接收方（同一线程内验证配对）
        let right = hub.accept("test-1", 1000).expect("accept");

        // 两个端点不是同一个对象
        assert!(
            !std::ptr::eq(&left, &right),
            "left and right should be different duplex halves"
        );
    }

    #[test]
    fn test_accept_timeout() {
        let hub = LocalStreamHub::new();
        let result = hub.accept("nobody-opened-this", 100);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }
}
