//Presented by KeJi
//Date ： 2026-05-24

//! 独立本地流

pub mod frames;

use std::collections::HashMap;
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{duplex, DuplexStream};
use tokio::sync::oneshot;

pub struct LocalStreamHub {
    pending: Mutex<HashMap<String, DuplexStream>>,
    notifiers: Mutex<HashMap<String, oneshot::Sender<DuplexStream>>>,
}

impl LocalStreamHub {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            notifiers: Mutex::new(HashMap::new()),
        }
    }

    /// 发起方调用：优先通知异步等的人，否则存 pending。
    pub fn open(&self, id: &str) -> io::Result<DuplexStream> {
        let (local, remote) = duplex(16 * 1024 * 1024);
        // 优先：有人在等异步？
        let mut notifiers = self.notifiers.lock()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        if let Some(tx) = notifiers.remove(id) {
            let _ = tx.send(remote);
            tracing::info!("LocalStream open '{}': paired via async notifier", id);
            return Ok(local);
        }
        drop(notifiers);

        // 没人等 → 存 pending
        self.pending
            .lock()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
            .insert(id.to_string(), remote);
        tracing::info!("LocalStream open '{}': stored in pending", id);
        Ok(local)
    }

    /// 阻塞等待（兼容旧代码）
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

    /// 异步等待（不阻塞 tokio 线程）
    pub async fn accept_async(&self, id: &str, timeout_ms: u64) -> io::Result<DuplexStream> {
        // 先检查 pending
        {
            let mut pending = self.pending.lock()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            if let Some(stream) = pending.remove(id) {
                tracing::info!("LocalStream accept_async '{}': found in pending", id);
                return Ok(stream);
            }
        }

        // 注册 oneshot，等 open() 通知
        let (tx, rx) = oneshot::channel();
        self.notifiers.lock()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
            .insert(id.to_string(), tx);
        tracing::info!("LocalStream accept_async '{}': registered notifier, waiting", id);

        match tokio::time::timeout(
            Duration::from_millis(timeout_ms), rx,
        ).await {
            Ok(Ok(stream)) => {
                tracing::info!("LocalStream accept_async '{}': paired", id);
                Ok(stream)
            }
            Ok(Err(_)) => Err(io::Error::new(io::ErrorKind::Other, "sender dropped")),
            Err(_) => {
                // 超时，清理 notifier
                self.notifiers.lock()
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
                    .remove(id);
                tracing::warn!("LocalStream accept_async '{}': timeout", id);
                Err(io::Error::new(io::ErrorKind::TimedOut,
                    format!("accept_async timeout: id={}", id)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_accept_pair() {
        let hub = LocalStreamHub::new();
        let left = hub.open("test-1").expect("open");
        let right = hub.accept("test-1", 1000).expect("accept");
        assert!(!std::ptr::eq(&left, &right));
    }

    #[test]
    fn test_accept_timeout() {
        let hub = LocalStreamHub::new();
        let result = hub.accept("nobody-opened-this", 100);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn test_accept_async_pair() {
        let hub = LocalStreamHub::new();
        let left = hub.open("test-async-1").expect("open");
        let right = hub.accept_async("test-async-1", 1000).await.expect("accept_async");
        assert!(!std::ptr::eq(&left, &right));
    }

    #[tokio::test]
    async fn test_accept_async_timeout() {
        let hub = LocalStreamHub::new();
        let result = hub.accept_async("nobody", 100).await;
        assert!(result.is_err());
    }
}
