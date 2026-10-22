//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

//! 单条 WS 连接封装：连接 / 发送 / 收包 / 关闭码分类（task_31 §四.5.5）
//!
//! 为什么还要 `Split()`：`tokio::select!` 里没法同时 `Next()` 与 `Send_Json()`
//! （同一个 `&mut Link` 双借，直接编译不过）。所以连上后先发 `*_hello` 再分体：
//! `LinkSender` 归 supervisor（hello / motion_cmd），`LinkReceiver` 进收包 task。
//!
//! 收发纪律：
//! - 发送一律 `Message::Text`（世界侧收到二进制会 warning「二进制只下行」）
//! - `Ping`/`Pong`/`Frame` 一律跳过：世界 `HEARTBEAT_INTERVAL_S = 10` 会发 Ping，
//!   tungstenite 在读路径里自动排队 Pong——**只要 recv task 持续 poll `Next()`**，心跳就不断
//! - 关闭码要留全（1002/1008/1011/1001 分流在 supervisor），无 Close 帧 → 1005，
//!   读错误 → 1006

use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 一条连接上发生的事（已屏蔽 tungstenite 的心跳/raw frame 细节）
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    Text(String),
    Binary(Vec<u8>),
    Closed { code: u16, reason: String },
}

/// 连接建立后的两半：发送端归 supervisor，接收端进 recv task
pub struct Link {
    tx: LinkSender,
    rx: LinkReceiver,
}

impl Link {
    /// TCP + WS 握手共用**一个**超时（世界在本机，ECONNREFUSED 会立刻返回）
    pub async fn Connect(url: &str, timeout_ms: u64) -> Result<Self, String> {
        let timeout = Duration::from_millis(timeout_ms.max(1));
        let (ws, _resp) = tokio::time::timeout(timeout, connect_async(url))
            .await
            .map_err(|_| format!("连接 {url} 超时（{timeout_ms} ms）"))?
            .map_err(|e| format!("连接 {url} 失败：{e}"))?;
        let (tx, rx) = ws.split();
        Ok(Self { tx: LinkSender { tx }, rx: LinkReceiver { rx } })
    }

    /// 发一条 JSON 文本帧
    pub async fn Send_Json<T: serde::Serialize>(&mut self, v: &T) -> Result<(), String> {
        self.tx.Send_Json(v).await
    }

    /// 分体：`(发送端, 接收端)`
    pub fn Split(self) -> (LinkSender, LinkReceiver) {
        (self.tx, self.rx)
    }

    /// 直接收一条（分体前的便利入口；分体后由 [`LinkReceiver::Next`] 承担）
    #[allow(dead_code)] // 分体后由 LinkReceiver::Next 使用（见模块头 R5）
    pub async fn Next(&mut self) -> Option<LinkEvent> {
        self.rx.Next().await
    }

    /// 主动优雅关闭（发 Close 帧）
    #[allow(dead_code)] // supervisor 在 session 结束时靠 cancel + drop 收尾，本 step 未调用
    pub async fn Close(&mut self) {
        self.tx.Close().await;
    }
}

/// 发送半：supervisor 持有
pub struct LinkSender {
    tx: SplitSink<Ws, Message>,
}

impl LinkSender {
    pub async fn Send_Json<T: serde::Serialize>(&mut self, v: &T) -> Result<(), String> {
        let s = serde_json::to_string(v).map_err(|e| format!("序列化失败：{e}"))?;
        self.Send_Text(&s).await
    }

    pub async fn Send_Text(&mut self, s: &str) -> Result<(), String> {
        self.tx
            .send(Message::Text(s.to_string()))
            .await
            .map_err(|e| format!("发送失败：{e}"))
    }

    #[allow(dead_code)] // 同 Link::Close
    pub async fn Close(&mut self) {
        let _ = self.tx.send(Message::Close(None)).await;
        let _ = self.tx.flush().await;
    }
}

/// 接收半：每连接一个 recv task 持有并持续 poll（心跳靠它）
pub struct LinkReceiver {
    rx: SplitStream<Ws>,
}

impl LinkReceiver {
    /// `None` = 流结束（对端消失或连接已拆）
    pub async fn Next(&mut self) -> Option<LinkEvent> {
        loop {
            match self.rx.next().await {
                None => return None,
                Some(Ok(msg)) => {
                    if let Some(ev) = Classify(msg) {
                        return Some(ev);
                    }
                    // Ping/Pong/Frame：跳过继续 poll
                }
                Some(Err(e)) => {
                    return Some(LinkEvent::Closed { code: 1006, reason: e.to_string() });
                }
            }
        }
    }
}

/// 纯函数：tungstenite `Message` → [`LinkEvent`]；心跳 / raw frame → `None`（跳过）
fn Classify(msg: Message) -> Option<LinkEvent> {
    match msg {
        Message::Text(t) => Some(LinkEvent::Text(t)),
        Message::Binary(b) => Some(LinkEvent::Binary(b)),
        Message::Close(frame) => Some(match frame {
            Some(f) => LinkEvent::Closed { code: u16::from(f.code), reason: f.reason.to_string() },
            // 没有关闭码：1005 = "No Status Rcvd"（RFC 6455）
            None => LinkEvent::Closed { code: 1005, reason: String::new() },
        }),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;

    #[test]
    fn test_classify_text() {
        assert_eq!(
            Classify(Message::Text("{\"msg\":\"pose\"}".into())),
            Some(LinkEvent::Text("{\"msg\":\"pose\"}".into()))
        );
    }

    #[test]
    fn test_classify_binary() {
        assert_eq!(
            Classify(Message::Binary(vec![0u8, 1, 2, 3])),
            Some(LinkEvent::Binary(vec![0, 1, 2, 3]))
        );
    }

    #[test]
    fn test_classify_close_with_code() {
        let frame = CloseFrame { code: CloseCode::Protocol, reason: "hello timeout".into() };
        assert_eq!(
            Classify(Message::Close(Some(frame))),
            Some(LinkEvent::Closed { code: 1002, reason: "hello timeout".into() })
        );
    }

    #[test]
    fn test_classify_close_without_code() {
        assert_eq!(
            Classify(Message::Close(None)),
            Some(LinkEvent::Closed { code: 1005, reason: String::new() })
        );
    }

    #[test]
    fn test_classify_ping_skipped() {
        assert_eq!(Classify(Message::Ping(vec![1, 2, 3])), None);
        assert_eq!(Classify(Message::Pong(vec![1, 2, 3])), None);
    }
}
