//Presented by KeJi
//Date ： 2026-05-21

//! Lua 能力注册 — 独立本地流
//!
//! 向 Lua 注册 `local_tensor` 全局表，提供同进程线程间的张量流通信。

use std::sync::{Arc, Mutex};

use mlua::{Lua, UserData};
use tokio::io::DuplexStream;

use crate::orchestrator::local_tensor_stream::frames;
use crate::orchestrator::local_tensor_stream::LocalStreamHub;
use crate::network::tensor_stream::protocol::Tensor_Buffer;

/// Lua 可见的本地流句柄
pub struct LocalTensorStream {
    pub stream: Mutex<DuplexStream>,
}

impl UserData for LocalTensorStream {}

/// 向 Lua 的 `local_tensor` 表注册函数。
pub fn register_local_stream_caps(
    lua: &Lua,
    hub: Arc<LocalStreamHub>,
) -> mlua::Result<()> {
    let local = lua.create_table()?;

    // ─── open_stream ──────────────────────────────────
    {
        let h = hub.clone();
        local.set(
            "open_stream",
            lua.create_function(move |_, id: String| {
                let stream = h
                    .open(&id)
                    .map_err(|e| mlua::Error::runtime(format!("open_stream: {}", e)))?;
                Ok(LocalTensorStream {
                    stream: Mutex::new(stream),
                })
            })?,
        )?;
    }

    // ─── accept_stream ────────────────────────────────
    {
        let h = hub.clone();
        local.set(
            "accept_stream",
            lua.create_function(move |_, (id, timeout_ms): (String, u64)| {
                let stream = h
                    .accept(&id, timeout_ms)
                    .map_err(|e| mlua::Error::runtime(format!("accept_stream: {}", e)))?;
                Ok(LocalTensorStream {
                    stream: Mutex::new(stream),
                })
            })?,
        )?;
    }

    // ─── send_tensor ──────────────────────────────────
    local.set(
        "send_tensor",
        lua.create_async_function(
            move |_, (stream, data, offset): (mlua::AnyUserData, String, u64)| async move {
                let stream_ud = stream
                    .borrow::<LocalTensorStream>()
                    .map_err(|e| mlua::Error::runtime(format!("send_tensor: {}", e)))?;
                let mut guard = stream_ud
                    .stream
                    .lock()
                    .map_err(|e| mlua::Error::runtime(format!("send_tensor: {}", e)))?;
                frames::local_send_frame(&mut *guard, offset, data.as_bytes())
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("send_tensor: {}", e)))?;
                Ok(())
            },
        )?,
    )?;

    // ─── recv_tensor ──────────────────────────────────
    local.set(
        "recv_tensor",
        lua.create_async_function(
            move |lua, stream: mlua::AnyUserData| async move {
                let stream_ud = stream
                    .borrow::<LocalTensorStream>()
                    .map_err(|e| mlua::Error::runtime(format!("recv_tensor: {}", e)))?;
                let mut guard = stream_ud
                    .stream
                    .lock()
                    .map_err(|e| mlua::Error::runtime(format!("recv_tensor: {}", e)))?;
                let mut buf = Tensor_Buffer::New(16 * 1024 * 1024);
                let offset = frames::local_recv_frame(&mut *guard, &mut buf)
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("recv_tensor: {}", e)))?;
                drop(guard);
                drop(stream_ud);
                let data = String::from_utf8_lossy(buf.As_Slice()).to_string();
                let tbl = lua.create_table()?;
                tbl.set("offset", offset)?;
                tbl.set("data", data)?;
                Ok(tbl)
            },
        )?,
    )?;

    // ─── send_eof ─────────────────────────────────────
    local.set(
        "send_eof",
        lua.create_async_function(move |_, stream: mlua::AnyUserData| async move {
            let stream_ud = stream
                .borrow::<LocalTensorStream>()
                .map_err(|e| mlua::Error::runtime(format!("send_eof: {}", e)))?;
            let mut guard = stream_ud
                .stream
                .lock()
                .map_err(|e| mlua::Error::runtime(format!("send_eof: {}", e)))?;
            frames::local_send_eof(&mut *guard)
                .await
                .map_err(|e| mlua::Error::runtime(format!("send_eof: {}", e)))?;
            Ok(())
        })?,
    )?;

    lua.globals().set("local_tensor", local)?;
    Ok(())
}
