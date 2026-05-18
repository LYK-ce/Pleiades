//Presented by KeJi
//Date ： 2026-05-17

//! Lua 能力函数桥接层
//!
//! 将 Rust 函数注册到 Lua 的 `caps` 表，供 Lua 脚本调用。
//! 支持同步函数（create_function）和异步函数（create_async_function）。

use std::sync::Arc;
use mlua::Lua;
use libp2p::PeerId;
use crate::storage::StorageCapability;
use crate::ml_engine::MlSession;
use crate::event_bus::{EventBus, event::Bus_Event};
use crate::network::DataType;
use crate::orchestrator::Capabilities;
use super::storage_handle::{StorageReadHandle, StorageWriteHandle};

// ============================================================
// 注册测试用能力函数
// ============================================================

/// 向 Lua 的 `caps` 表注册演示能力函数。
///
/// 用于 Phase 2 验证 Rust↔Lua 函数桥接通路。
pub fn register_caps(lua: &Lua) -> mlua::Result<()> {
    let caps = lua.create_table()?;

    // ─── echo: 同步函数，直接返回参数 ───────────────────
    caps.set(
        "echo",
        lua.create_function(|_, msg: String| Ok::<_, mlua::Error>(msg))?,
    )?;

    // ─── add: 同步函数，接收两个数字求和 ────────────────
    caps.set(
        "add",
        lua.create_function(|_, (a, b): (f64, f64)| Ok::<_, mlua::Error>(a + b))?,
    )?;

    // ─── ping: 异步函数，返回固定字符串 ─────────────────
    caps.set(
        "ping",
        lua.create_async_function(|_, (): ()| async move {
            Ok::<_, mlua::Error>("pong".to_string())
        })?,
    )?;

    // ─── table_sum: 接收 Lua table {a=?, b=?} 返回 a+b ──
    caps.set(
        "table_sum",
        lua.create_function(|_, table: mlua::Table| {
            let a: f64 = table.get("a")?;
            let b: f64 = table.get("b")?;
            Ok::<_, mlua::Error>(a + b)
        })?,
    )?;

    lua.globals().set("caps", caps)?;
    Ok(())
}

// ============================================================
// 注册 Storage 能力函数（完整版）
// ============================================================

/// 将 Storage 完整能力暴露给 Lua（全异步）。
pub fn register_storage_caps(
    lua: &Lua,
    storage: Arc<dyn StorageCapability>,
) -> mlua::Result<()> {
    let caps: mlua::Table = lua.globals().get("caps").unwrap_or_else(|_| lua.create_table().expect("create caps table"));

    // ─── storage_list ──────────────────────────────────
    let s = storage.clone();
    caps.set("storage_list", lua.create_async_function(move |lua, (): ()| {
        let s = s.clone();
        async move {
            let entries = s.list().await
                .map_err(|e| mlua::Error::runtime(format!("storage_list: {}", e)))?;
            let result = lua.create_table()?;
            for (i, entry) in entries.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("file_name", entry.file_name.clone())?;
                row.set("size", entry.size)?;
                result.set(i + 1, row)?;
            }
            Ok(result)
        }
    })?)?;

    // ─── storage_exists ────────────────────────────────
    let s = storage.clone();
    caps.set("storage_exists", lua.create_async_function(move |_, file_id: String| {
        let s = s.clone();
        async move {
            s.exists(&file_id).await
                .map_err(|e| mlua::Error::runtime(format!("storage_exists: {}", e)))
        }
    })?)?;

    // ─── storage_acquire_read ──────────────────────────
    let s = storage.clone();
    caps.set("storage_acquire_read", lua.create_async_function(move |_, file_id: String| {
        let s = s.clone();
        async move {
            s.acquire_read(&file_id).await
                .map(|(path, guard)| StorageReadHandle::new(path, guard))
                .map_err(|e| mlua::Error::runtime(format!("storage_acquire_read: {}", e)))
        }
    })?)?;

    // ─── storage_acquire_write ─────────────────────────
    let s = storage.clone();
    caps.set("storage_acquire_write", lua.create_async_function(move |_, file_id: String| {
        let s = s.clone();
        async move {
            s.acquire_write(&file_id).await
                .map(|(path, guard)| StorageWriteHandle::new(path, guard))
                .map_err(|e| mlua::Error::runtime(format!("storage_acquire_write: {}", e)))
        }
    })?)?;

    // ─── storage_remove ────────────────────────────────
    let s = storage.clone();
    caps.set("storage_remove", lua.create_async_function(move |_, file_id: String| {
        let s = s.clone();
        async move {
            s.remove(&file_id).await
                .map_err(|e| mlua::Error::runtime(format!("storage_remove: {}", e)))
        }
    })?)?;

    // ─── storage_checksum ──────────────────────────────
    let s = storage.clone();
    caps.set("storage_checksum", lua.create_async_function(move |_, (file_id, algo): (String, Option<String>)| {
        let s = s.clone();
        async move {
            let algo = match algo.as_deref() {
                Some("blake3") => Some(crate::storage::ChecksumAlgorithm::Blake3),
                Some("sha256") => Some(crate::storage::ChecksumAlgorithm::Sha256),
                Some("xxhash64") | None => None,
                _ => return Err(mlua::Error::runtime(format!("未知算法: {}", algo.unwrap()))),
            };
            s.checksum(&file_id, algo).await
                .map_err(|e| mlua::Error::runtime(format!("storage_checksum: {}", e)))
        }
    })?)?;

    // ─── storage_flush ─────────────────────────────────
    let s = storage.clone();
    caps.set("storage_flush", lua.create_async_function(move |lua, (): ()| {
        let s = s.clone();
        async move {
            let (added, removed) = s.flush().await
                .map_err(|e| mlua::Error::runtime(format!("storage_flush: {}", e)))?;
            let tbl = lua.create_table()?;
            tbl.set("added", added)?;
            tbl.set("removed", removed)?;
            Ok(tbl)
        }
    })?)?;

    lua.globals().set("caps", caps)?;
    Ok(())
}

// ============================================================
// 注册 ML Engine 能力函数
// ============================================================

/// 将 ML Engine 能力暴露给 Lua。
///
/// 注册 `ml` 函数表：
/// - `ml.new(device)` → 返回 MlSession userdata（空壳）
///
/// MlSession userdata 自带方法（已在 context.rs 注册）：
/// - `sess:has_model()` → bool
/// - `sess:get_eos()` → u32
/// - `sess:get_offset()` → usize
/// - `sess:tensorize(token_ids)` → {dim0, dim1, ...}
/// - `sess:load_model(path, start, end)` → ()
/// - `sess:unload()` → ()
pub fn register_ml_caps(lua: &Lua) -> mlua::Result<()> {
    let ml = lua.create_table()?;

    ml.set(
        "new",
        lua.create_function(|_, device: String| {
            MlSession::new(&device)
                .map_err(|e| mlua::Error::runtime(e))
        })?,
    )?;

    lua.globals().set("ml", ml)?;
    Ok(())
}

// ============================================================
// 注册日志能力函数 (tracing + EventBus)
// ============================================================

/// 将日志输出能力暴露给 Lua。
///
/// 注册 `caps.print(msg)` — 同时写入 tracing 日志文件 和 TUI 日志面板。
pub fn register_logging_caps(lua: &Lua, event_bus: Arc<EventBus>) -> mlua::Result<()> {
    let caps: mlua::Table = lua.globals().get("caps").unwrap_or_else(|_| lua.create_table().expect("create caps table"));

    caps.set(
        "print",
        lua.create_function(move |_, msg: String| {
            tracing::info!(target: "lua", "{}", msg);
            event_bus.Publish(Bus_Event::Log { message: msg });
            Ok::<_, mlua::Error>(())
        })?,
    )?;

    lua.globals().set("caps", caps)?;
    Ok(())
}

// ============================================================
// 注册 Network 能力函数（非流方法）
// ============================================================

/// 将 Network 能力暴露给 Lua（全异步）。
pub fn register_network_caps(
    lua: &Lua,
    capabilities: Arc<Capabilities>,
) -> mlua::Result<()> {
    let caps: mlua::Table = lua.globals().get("caps")
        .unwrap_or_else(|_| lua.create_table().expect("create caps table"));
    let network = lua.create_table()?;

    // ─── send_data ──────────────────────────────────────
    let caps_net = capabilities.clone();
    network.set("send_data", lua.create_async_function(move |lua, (peer_str, data_type_str, payload): (String, String, String)| {
        let caps_net = caps_net.clone();
        async move {
            let peer = peer_str.parse::<PeerId>()
                .map_err(|e| mlua::Error::runtime(format!("无效 PeerId: {}", e)))?;
            let dt = match data_type_str.as_str() {
                "Command" => DataType::Command,
                "Data" => DataType::Data,
                "File" => DataType::File,
                "Info" => DataType::Info,
                _ => return Err(mlua::Error::runtime(format!("未知 DataType: {}", data_type_str))),
            };
            let result = caps_net.network.send_data(peer, dt, payload.into_bytes()).await
                .map_err(|e| mlua::Error::runtime(format!("send_data: {}", e)))?;
            let tbl = lua.create_table()?;
            tbl.set("payload", String::from_utf8_lossy(&result.payload).to_string())?;
            Ok(tbl)
        }
    })?)?;

    // ─── get_local_peer_id ──────────────────────────────
    let caps_net = capabilities.clone();
    network.set("get_local_peer_id", lua.create_function(move |_, (): ()| {
        Ok::<_, mlua::Error>(caps_net.network.get_local_peer_id().to_base58())
    })?)?;

    // ─── dial ──────────────────────────────────────────
    let caps_net = capabilities.clone();
    network.set("dial", lua.create_async_function(move |_, addr: String| {
        let caps_net = caps_net.clone();
        async move {
            let ma = addr.parse::<libp2p::Multiaddr>()
                .map_err(|e| mlua::Error::runtime(format!("无效 Multiaddr: {}", e)))?;
            caps_net.network.dial(ma).await
                .map_err(|e| mlua::Error::runtime(format!("dial: {}", e)))
        }
    })?)?;

    // ─── disconnect ─────────────────────────────────────
    let caps_net = capabilities.clone();
    network.set("disconnect", lua.create_async_function(move |_, peer_str: String| {
        let caps_net = caps_net.clone();
        async move {
            let peer = peer_str.parse::<PeerId>()
                .map_err(|e| mlua::Error::runtime(format!("无效 PeerId: {}", e)))?;
            caps_net.network.disconnect(peer).await
                .map_err(|e| mlua::Error::runtime(format!("disconnect: {}", e)))
        }
    })?)?;

    // ─── test_bandwidth ─────────────────────────────────
    let caps_net = capabilities.clone();
    network.set("test_bandwidth", lua.create_async_function(move |_, peer_str: String| {
        let caps_net = caps_net.clone();
        async move {
            let peer = peer_str.parse::<PeerId>()
                .map_err(|e| mlua::Error::runtime(format!("无效 PeerId: {}", e)))?;
            caps_net.network.test_bandwidth(peer).await
                .map_err(|e| mlua::Error::runtime(format!("test_bandwidth: {}", e)))
        }
    })?)?;

    // ─── send_file ──────────────────────────────────────
    let caps_net = capabilities.clone();
    network.set("send_file", lua.create_async_function(move |_, (peer_str, file_path): (String, String)| {
        let caps_net = caps_net.clone();
        async move {
            let peer = peer_str.parse::<PeerId>()
                .map_err(|e| mlua::Error::runtime(format!("无效 PeerId: {}", e)))?;
            caps_net.network.send_file(peer, std::path::Path::new(&file_path)).await
                .map_err(|e| mlua::Error::runtime(format!("send_file: {}", e)))
        }
    })?)?;

    caps.set("network", network)?;
    lua.globals().set("caps", caps)?;
    Ok(())
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::engine::LuaContext;

    #[test]
    fn test_sync_function_echo() {
        let lua = LuaContext::new().expect("create lua");
        register_caps(&lua).expect("register caps");

        let result: String = lua
            .load(r#"return caps.echo("hi")"#)
            .eval()
            .expect("call echo");
        assert_eq!(result, "hi");
    }

    #[test]
    fn test_sync_function_add() {
        let lua = LuaContext::new().expect("create lua");
        register_caps(&lua).expect("register caps");

        let result: f64 = lua
            .load(r#"return caps.add(3.0, 4.0)"#)
            .eval()
            .expect("call add");
        assert_eq!(result, 7.0);
    }

    #[test]
    fn test_async_function_ping() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let lua = LuaContext::new().expect("create lua");
        register_caps(&lua).expect("register caps");

        let result: String = rt.block_on(async {
            lua.load(r#"return caps.ping()"#)
                .eval_async()
                .await
                .expect("call ping")
        });
        assert_eq!(result, "pong");
    }

    #[test]
    fn test_table_sum() {
        let lua = LuaContext::new().expect("create lua");
        register_caps(&lua).expect("register caps");

        let result: f64 = lua
            .load(r#"
                local t = {a = 10, b = 20}
                return caps.table_sum(t)
            "#)
            .eval()
            .expect("call table_sum");
        assert_eq!(result, 30.0);
    }

    #[test]
    fn test_lua_loads_script_and_calls_caps() {
        let lua = LuaContext::new().expect("create lua");
        register_caps(&lua).expect("register caps");

        // 模拟脚本调用：构造 table 传入 Rust，Rust 返回 a+b
        let script = r#"
            function execute(params)
                return caps.table_sum(params)
            end
        "#;
        lua.load(script).eval::<()>().expect("eval script");

        let params = lua.create_table().expect("params");
        params.set("a", 100).expect("a");
        params.set("b", 200).expect("b");

        let execute: mlua::Function = lua.globals().get("execute").expect("execute");
        let result: f64 = execute.call(params).expect("call execute");
        assert_eq!(result, 300.0);
    }

    // ─── Storage 能力测试 ────────────────────────────────

    #[test]
    fn test_storage_caps_list_and_exists() {
        // 1. 创建 tempdir + StorageManager
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let storage: Arc<dyn StorageCapability> = Arc::new(
            rt.block_on(crate::storage::StorageManager::New(temp_dir.path()))
                .expect("StorageManager::New")
        );

        // 2. 在 tempdir 中创建文件 "hello.txt"
        std::fs::write(temp_dir.path().join("hello.txt"), b"hello world").expect("write file");

        // 3. flush 让 Storage 发现文件
        rt.block_on(storage.flush()).expect("flush");

        // 4. 创建 Lua + 注册 storage caps
        let lua = LuaContext::new().expect("create lua");
        register_storage_caps(&lua, storage).expect("register storage caps");

        // 5. Lua 脚本调用 storage_list → 验证文件列表
        let result: mlua::Value = rt.block_on(lua.load(r#"
            local files = caps.storage_list()
            if #files == 0 then
                return "empty"
            end
            local f = files[1]
            return f.file_name .. ":" .. tostring(f.size)
        "#).eval_async()).expect("call storage_list");

        let result_str = result.to_string().expect("to_string");
        assert_eq!(result_str, "hello.txt:11");

        // 6. Lua 脚本调用 storage_exists
        let exists: bool = rt.block_on(lua.load(r#"return caps.storage_exists("hello.txt")"#)
            .eval_async()).expect("call exists");
        assert!(exists);

        let missing: bool = rt.block_on(lua.load(r#"return caps.storage_exists("nope.txt")"#)
            .eval_async()).expect("call exists");
        assert!(!missing);
    }

    // ─── ML Engine 能力测试 ────────────────────────────

    #[test]
    fn test_ml_session_empty_shell() {
        let lua = LuaContext::new().expect("create lua");
        register_ml_caps(&lua).expect("register ml caps");

        let script = r#"
            local sess = ml.new("cpu")
            return {
                has_model = sess:has_model(),
                eos = sess:get_eos(),
                offset = sess:get_offset(),
            }
        "#;
        let result: mlua::Table = lua.load(script).eval().expect("eval script");

        assert_eq!(result.get::<bool>("has_model").expect("has_model"), false);
        assert_eq!(result.get::<u32>("eos").expect("eos"), 151645);
        assert_eq!(result.get::<usize>("offset").expect("offset"), 0);
    }

    #[test]
    fn test_ml_tensorize_shape() {
        let lua = LuaContext::new().expect("create lua");
        register_ml_caps(&lua).expect("register ml caps");

        let result: mlua::Table = lua.load(r#"
            local sess = ml.new("cpu")
            return sess:tensorize({1, 2, 3, 4, 5})
        "#).eval().expect("call tensorize");

        assert_eq!(result.get::<usize>(1).expect("dim0"), 1);  // batch
        assert_eq!(result.get::<usize>(2).expect("dim1"), 5);  // seq_len
    }

    #[test]
    fn test_ml_tensorize_large_input() {
        let lua = LuaContext::new().expect("create lua");
        register_ml_caps(&lua).expect("register ml caps");

        let result: mlua::Table = lua.load(r#"
            local sess = ml.new("cpu")
            local ids = {}
            for i = 1, 1024 do
                table.insert(ids, i % 32000)
            end
            return sess:tensorize(ids)
        "#).eval().expect("call tensorize large");

        assert_eq!(result.get::<usize>(1).expect("dim0"), 1);    // batch
        assert_eq!(result.get::<usize>(2).expect("dim1"), 1024); // seq_len
    }

    #[test]
    fn test_ml_tensorize_empty_errors() {
        let lua = LuaContext::new().expect("create lua");
        register_ml_caps(&lua).expect("register ml caps");

        let result = lua.load(r#"
            local sess = ml.new("cpu")
            return sess:tensorize({})
        "#).eval::<mlua::Value>();

        assert!(result.is_err(), "空输入应返回错误");
    }

    #[test]
    fn test_ml_script_calls_multiple_methods() {
        let lua = LuaContext::new().expect("create lua");
        register_ml_caps(&lua).expect("register ml caps");

        let script = r#"
            COMMAND = "ml_demo"
            function execute(params)
                local sess = ml.new(params.device or "cpu")
                local dims = sess:tensorize({1, 2, 3})

                return {
                    has_model = sess:has_model(),
                    eos = sess:get_eos(),
                    tensor_shape = dims,
                }
            end
        "#;
        lua.load(script).eval::<()>().expect("eval script");

        let params = lua.create_table().expect("params");
        params.set("device", "cpu").expect("set device");

        let execute: mlua::Function = lua.globals().get("execute").expect("execute");
        let result: mlua::Table = execute.call(params).expect("call execute");

        assert_eq!(result.get::<bool>("has_model").expect("has_model"), false);
        assert_eq!(result.get::<u32>("eos").expect("eos"), 151645);
        let shape: mlua::Table = result.get("tensor_shape").expect("tensor_shape");
        assert_eq!(shape.get::<usize>(2).expect("dim1"), 3);
    }
}
