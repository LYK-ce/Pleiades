//Presented by KeJi
//Created Date ： 2026-09-08
//Modified Date ： 2026-11-04

//! YOLO 检测结果 WebSocket 画面流服务
//!
//! - `ensure_webui_server(bind)`：幂等启动服务（已启动直接返回已绑定端口，不重复 bind）
//! - `publish_detection(jpeg, 元数据, dets)`：推一帧「图 + 框」；同步、非阻塞、无订阅者即丢
//! - `webui_publish(jpeg, dets_json)`：Lua `caps.webui.send` 的旧入口，契约不变
//! - 每个浏览器连接订阅广播通道，服务把每帧以二进制消息转发
//!
//! 消息格式（一条二进制消息 = 一帧；`Tool/yolo_viewer.html` 一行不改）：
//! `[4B header_len LE][header JSON][JPEG 原始字节]`
//! header = `{seq, frame_ts_ms, infer_ms, width, height,
//! dets:[{class_name, conf, xmin, ymin, xmax, ymax}]}`

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use tokio::sync::broadcast;

use super::server::find_available_port;

/// 全局广播通道（服务未启动时为 None，发布口静默丢弃）
static FRAME_TX: OnceLock<broadcast::Sender<Vec<u8>>> = OnceLock::new();

/// 已绑定地址（服务未启动为 None）
static SERVER: OnceLock<SocketAddr> = OnceLock::new();

/// 串行化首次启动，防并发首次调用双 bind
static ENSURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `publish_detection` 调用计数（可观测钩子，不属于业务 API）
static PUBLISH_COUNT: AtomicU64 = AtomicU64::new(0);

/// 一帧里的一条检测框（`publish_detection` 的入参，纯数据、可 Copy）。
#[derive(Debug, Clone, Copy)]
pub struct StreamDet {
    pub class_name: &'static str,
    pub conf: f32,
    pub xmin: f32,
    pub ymin: f32,
    pub xmax: f32,
    pub ymax: f32,
}

/// 幂等启动 WebSocket 画面流服务，返回实际绑定端口。
///
/// 已启动 → 直接返回已绑定端口（不重复 bind，避免 `address in use`）。
/// `bind.port() == 0` 时由内核分配端口，用 `listener.local_addr()` 回填。
pub async fn ensure_webui_server(bind: SocketAddr) -> Result<u16, String> {
    let _guard = ENSURE_LOCK.lock().await;
    if let Some(addr) = SERVER.get() {
        if bind.port() != 0 && bind.port() != addr.port() {
            tracing::warn!("WebUI server 已在 {addr} 运行，忽略本次请求的 {bind}");
        }
        return Ok(addr.port());
    }

    let app = Router::new().route("/ws", get(ws_handler));
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| format!("bind {bind}: {e}"))?;
    let local = listener
        .local_addr()
        .map_err(|e| format!("local_addr {bind}: {e}"))?;

    // bind 成功后才建广播通道：失败时不留任何全局状态
    if FRAME_TX.get().is_none() {
        let (tx, _rx) = broadcast::channel(16);
        let _ = FRAME_TX.set(tx);
    }

    tracing::info!("WebUI server listening on ws://{local}/ws");

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("WebUI server exited: {e}");
        }
    });

    let _ = SERVER.set(local);
    Ok(local.port())
}

/// 旧入口（TUI `webui [port]` 仍在用），内部转 `ensure_webui_server`。
///
/// `None` = 从 9010 起找可用端口，绑 `0.0.0.0`。
pub async fn spawn_webui_server(port: Option<u16>) -> Result<u16, String> {
    let port = match port {
        Some(p) => p,
        None => find_available_port(9010)?,
    };
    ensure_webui_server(SocketAddr::from(([0, 0, 0, 0], port))).await
}

/// 组 header JSON（纯函数，便于单测）。`dets[]` 键名与 `Tool/yolo_viewer.html` 一致。
pub fn build_header_json(
    width: u32,
    height: u32,
    seq: u64,
    frame_ts_ms: u64,
    infer_ms: u64,
    dets: &[StreamDet],
) -> String {
    let dets: Vec<serde_json::Value> = dets
        .iter()
        .map(|d| {
            serde_json::json!({
                "class_name": d.class_name,
                "conf": d.conf,
                "xmin": d.xmin,
                "ymin": d.ymin,
                "xmax": d.xmax,
                "ymax": d.ymax,
            })
        })
        .collect();
    serde_json::json!({
        "seq": seq,
        "frame_ts_ms": frame_ts_ms,
        "infer_ms": infer_ms,
        "width": width,
        "height": height,
        "dets": dets,
    })
    .to_string() // Value::to_string 不会失败；NaN/Inf 序列化为 null，不 panic
}

/// 打包一帧：`[4B header_len LE][header JSON][JPEG 原始字节]`（纯函数，两条入口共用）。
pub fn pack_frame(header_json: &str, jpeg: &[u8]) -> Vec<u8> {
    let n = header_json.len();
    let mut payload = Vec::with_capacity(4 + n + jpeg.len());
    payload.extend_from_slice(&(n as u32).to_le_bytes());
    payload.extend_from_slice(header_json.as_bytes());
    payload.extend_from_slice(jpeg);
    payload
}

/// 推一帧检测结果给所有订阅者：**同步、非阻塞、返回 `()`**。
///
/// 服务未启动 / 无订阅者 = 静默丢弃，绝不影响推理循环。
/// 每帧都推：`dets` 为空也照发（不设 `only_dets` 之类跳过条件）。
pub fn publish_detection(
    jpeg: &[u8],
    width: u32,
    height: u32,
    seq: u64,
    frame_ts_ms: u64,
    infer_ms: u64,
    dets: &[StreamDet],
) {
    // 计数先加：无服务也算「推了一次」
    PUBLISH_COUNT.fetch_add(1, Ordering::Relaxed);
    let Some(tx) = FRAME_TX.get() else {
        return;
    };
    let header = build_header_json(width, height, seq, frame_ts_ms, infer_ms, dets);
    let _ = tx.send(pack_frame(&header, jpeg));
}

/// 已调用 `publish_detection` 的次数（device 单测用，不属于业务 API）。
#[doc(hidden)]
pub fn publish_count() -> u64 {
    PUBLISH_COUNT.load(Ordering::Relaxed)
}

/// 服务是否已成功 bind（device 单测用，不属于业务 API）。
#[doc(hidden)]
pub fn is_server_started() -> bool {
    SERVER.get().is_some()
}

/// 打包一帧（`[4B header_len LE][JSON][JPEG]`）并广播给所有浏览器连接。
/// Lua `caps.webui.send` 的旧入口：契约不变（收 `&str`、服务未启动时静默丢弃）。
pub fn webui_publish(jpeg: &[u8], dets_json: &str) {
    let Some(tx) = FRAME_TX.get() else {
        return;
    };
    let _ = tx.send(pack_frame(dets_json, jpeg));
}

/// WebSocket 升级入口。
async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

/// 每连接转发 task：订阅广播，把每帧以二进制消息发给浏览器。
async fn handle_socket(mut socket: WebSocket) {
    let Some(tx) = FRAME_TX.get() else {
        return;
    };
    let mut rx = tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(payload) => {
                if socket.send(Message::Binary(payload)).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue, // 浏览器慢，丢帧
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// header_len 是 **LE**，JSON 在 `[4..4+len)`，JPEG 紧随其后。
    #[test]
    fn test_pack_frame_layout() {
        let header = "{\"dets\":[]}";
        let n = header.len();
        let jpeg = b"\xff\xd8JPEG\xff\xd9";
        let payload = pack_frame(header, jpeg);

        assert_eq!(payload.len(), 4 + n + jpeg.len());
        // LE：低字节在前；若误写 BE，第 0 字节会是 0
        assert_eq!(&payload[0..4], &[n as u8, (n >> 8) as u8, 0, 0]);
        assert_eq!(&payload[4..4 + n], header.as_bytes());
        assert_eq!(&payload[4 + n..], jpeg);
    }

    /// header 含全部键，且 `dets[]` 键名与查看器一致（`conf`，不是 `confidence`）。
    #[test]
    fn test_build_header_json_keys() {
        let dets = [StreamDet {
            class_name: "person",
            conf: 0.87,
            xmin: 1.0,
            ymin: 2.0,
            xmax: 3.0,
            ymax: 4.0,
        }];
        let header = build_header_json(1280, 720, 42, 1700000000000, 37, &dets);
        let v: Value = serde_json::from_str(&header).expect("header 必须是合法 JSON");

        assert_eq!(v["seq"].as_u64(), Some(42));
        assert_eq!(v["frame_ts_ms"].as_u64(), Some(1700000000000));
        assert_eq!(v["infer_ms"].as_u64(), Some(37));
        assert_eq!(v["width"].as_u64(), Some(1280));
        assert_eq!(v["height"].as_u64(), Some(720));

        let d = &v["dets"][0];
        assert_eq!(d["class_name"].as_str(), Some("person"));
        for key in ["class_name", "conf", "xmin", "ymin", "xmax", "ymax"] {
            assert!(d.get(key).is_some(), "dets[] 缺键 {key}");
        }
        assert!(d.get("confidence").is_none(), "新入口用 conf，不用 confidence");

        // 0 框也是合法 JSON
        let empty = build_header_json(1, 1, 0, 0, 0, &[]);
        let ev: Value = serde_json::from_str(&empty).expect("空 dets 也必须是合法 JSON");
        assert_eq!(ev["dets"].as_array().map(|a| a.len()), Some(0));
    }

    /// 连调两次 → 同一端口（幂等，不重复 bind）。
    #[tokio::test]
    async fn test_ensure_webui_server_idempotent() {
        let bind: SocketAddr = "127.0.0.1:0".parse().expect("测试地址常量");
        let p1 = ensure_webui_server(bind).await.expect("首次启动应成功");
        assert_ne!(p1, 0, "端口 0 必须由 local_addr 回填成真实端口");
        assert!(is_server_started(), "启动后 is_server_started 应为 true");

        let p2 = ensure_webui_server(bind).await.expect("二次启动应成功");
        assert_eq!(p2, p1, "重复调用必须返回同一端口");
    }

    /// 服务未启动 / 无订阅者：两个入口都不 panic；publish_detection 计数 +1，webui_publish 不计数。
    #[test]
    fn test_publish_without_server_no_panic() {
        let before = publish_count();
        publish_detection(b"no-jpeg", 1, 1, 0, 0, 0, &[]);
        webui_publish(b"no-jpeg", "{\"dets\":[]}");
        assert_eq!(publish_count(), before + 1);
    }
}
