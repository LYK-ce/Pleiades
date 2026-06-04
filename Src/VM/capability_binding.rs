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
use crate::ml_engine::{MlSession, capability};
use crate::ml_engine::lua_tensor::{LuaTensor, bytes_to_tensor_str, tensor_to_bytes};
use crate::vm::network_stream::NetworkStream;
use crate::network::tensor_stream::protocol::{Send_Tensor_Frame, Receive_Tensor_Frame, Send_EOF, Tensor_Buffer};
use crate::event_bus::{EventBus, Bus_Event, NotifyLevel};
use crate::network::DataType;
use crate::orchestrator::Capabilities;
use super::storage_handle::{StorageReadHandle, StorageWriteHandle};

// ============================================================
// 注册独立本地流能力函数
// ============================================================

/// 将独立本地流暴露给 Lua，注册 `local_tensor` 全局表。
///
/// 与 `network.*` 平行，用于同进程线程间的张量流通信。
pub fn register_local_stream_caps(
    lua: &Lua,
    hub: Arc<crate::orchestrator::local_tensor_stream::LocalStreamHub>,
) -> mlua::Result<()> {
    crate::vm::local_stream::register_local_stream_caps(lua, hub)
}

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
/// - `sess:load_model(path, start, end)` → ()  — 只加载权重
/// - `sess:load_tokenizer(path)` → ()            — 只加载 tokenizer
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

    ml.set(
        "tensor_from_bytes",
        lua.create_function(|_, (bytes, device): (mlua::String, String)| {
            let tensor = bytes_to_tensor_str(bytes.as_bytes().as_ref(), &device)
                .map_err(|e| mlua::Error::runtime(e))?;
            Ok(LuaTensor(tensor))
        })?,
    )?;

    ml.set(
        "analyze_model",
        lua.create_async_function(move |lua, path: String| async move {
            let info = capability::analyze_model(std::path::Path::new(&path)).await
                .map_err(|e| mlua::Error::runtime(e))?;
            let t = lua.create_table()?;
            t.set("architecture", info.architecture)?;
            t.set("num_layers", info.num_layers)?;
            t.set("embedding_length", info.embedding_length)?;
            t.set("head_count", info.head_count)?;
            t.set("head_count_kv", info.head_count_kv)?;
            t.set("head_dim", info.head_dim)?;
            t.set("feed_forward_length", info.feed_forward_length)?;
            t.set("context_length", info.context_length)?;
            t.set("rms_norm_eps", info.rms_norm_eps)?;
            t.set("rope_freq_base", info.rope_freq_base)?;
            t.set("vocab_size", info.vocab_size)?;
            t.set("eos_token_id", info.eos_token_id)?;
            t.set("is_split", info.is_split)?;
            t.set("split_start", info.split_start)?;
            t.set("split_end", info.split_end)?;
            if let Some(id) = info.model_id {
                t.set("model_id", id)?;
            }

            // 原始 metadata（所有 GGUF key-value）
            let meta = lua.create_table()?;
            for (k, v) in &info.metadata_raw {
                meta.set(k.as_str(), v.as_str())?;
            }
            t.set("metadata_raw", meta)?;

            Ok(t)
        })?,
    )?;

    ml.set(
        "split_model",
        lua.create_async_function(move |_, (path, start, end, output_dir, keep_tokenizer):
            (String, usize, usize, String, bool)| async move {
            capability::split_model(
                std::path::Path::new(&path),
                start,
                end,
                std::path::Path::new(&output_dir),
                keep_tokenizer,
            ).await.map_err(|e| mlua::Error::runtime(e))
        })?,
    )?;

    ml.set(
        "offload_load",
        lua.create_function(|_, (file_id, device): (String, String)| {
            MlSession::offload_load(&file_id, &device)
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
            event_bus.Publish(Bus_Event::Notify {
                level: NotifyLevel::Info,
                message: msg,
            });
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

    // ─── get_all_peers ──────────────────────────────────
    let caps_all = capabilities.clone();
    network.set("get_all_peers", lua.create_async_function(move |lua, (): ()| {
        let caps_all = caps_all.clone();
        async move {
            let peers = caps_all.peer_manager.Get_All_Peers().await
                .map_err(|e| mlua::Error::runtime(format!("get_all_peers: {}", e)))?;
            let result = lua.create_table()?;
            for (i, p) in peers.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("name", p.name.clone())?;
                entry.set("peer_id", p.peer_id.to_base58())?;
                result.set(i + 1, entry)?;
            }
            Ok(result)
        }
    })?)?;

    // ─── list_model_peers ───────────────────────────────
    let caps_mp = capabilities.clone();
    let caps_st = capabilities.storage.clone();
    network.set("list_model_peers", lua.create_async_function(move |lua, model_id: u32| {
        let caps_mp = caps_mp.clone();
        let caps_st = caps_st.clone();
        async move {
            // helper: 从文件名解析 _split_S_E.pgguf → (S, E) in pipeline 坐标
            fn parse_split_range(name: &str) -> Option<(u32, u32)> {
                let name = name.strip_suffix(".pgguf").unwrap_or(name);
                let marker = "_split_";
                if let Some(pos) = name.rfind(marker) {
                    let range_part = &name[pos + marker.len()..];
                    let parts: Vec<&str> = range_part.split('_').collect();
                    if parts.len() == 2 {
                        let s = parts[0].parse::<u32>().ok()?;
                        let e = parts[1].parse::<u32>().ok()?;
                        return Some((s, e));
                    }
                }
                None
            }

            // helper: bitmap → (min_bit, max_bit)
            fn bitmap_range(bitmap: &[u8; 32]) -> (u32, u32) {
                let mut min: Option<u32> = None;
                let mut max: Option<u32> = None;
                for layer in 0..256u32 {
                    if bitmap[layer as usize / 8] & (1 << (layer % 8)) != 0 {
                        if min.is_none() { min = Some(layer); }
                        max = Some(layer);
                    }
                }
                (min.unwrap_or(0), max.unwrap_or(0))
            }

            // helper: 修正 layer_start/layer_end 到 pipeline 坐标
            // pipeline 坐标: 0=embedding, 1..N=blocks, N+1=output
            fn adjust_range(
                min_bit: u32, max_bit: u32,
                file_name: &str,
                num_layers: Option<u32>,
            ) -> (u32, u32) {
                // 优先从文件名解析 split 范围（已是 pipeline 坐标）
                if let Some((s, e)) = parse_split_range(file_name) {
                    return (s, e);
                }
                // 非 split 模型：blk 位全覆盖 → layer_start=0, layer_end=N+1
                if let Some(n) = num_layers {
                    if n > 0 && (max_bit - min_bit + 1) >= n {
                        return (0, n + 1);
                    }
                }
                (min_bit, max_bit)
            }

            // ── 1. 收集 Storage 中匹配 model_id 的文件 ──
            let storage_files: std::collections::HashMap<String, u32> = {
                let mut map = std::collections::HashMap::new();
                if let Ok(entries) = caps_st.list().await {
                    for e in entries {
                        if e.model_id == Some(model_id) {
                            map.insert(e.file_name, e.num_layers.unwrap_or(0));
                        }
                    }
                }
                map
            };

            // ── 2. 收集所有 peer 条目 ────────────────────
            #[derive(Clone)]
            struct RawEntry {
                peer_id: String,
                name: String,
                file_name: String,
                model_id: u32,
                min_bit: u32,
                max_bit: u32,
                num_layers: Option<u32>,
                latency_ms: Option<u64>,
            }

            let peers = caps_mp.peer_manager.Get_All_Peers().await
                .map_err(|e| mlua::Error::runtime(format!("list_model_peers: {}", e)))?;

            let mut entries: Vec<RawEntry> = Vec::new();
            let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();

            for p in &peers {
                for m in &p.supported_models {
                    if m.id != model_id { continue; }
                    let (min_bit, max_bit) = bitmap_range(&m.layer_bitmap);
                    let nl = storage_files.get(&m.file_name).copied();
                    entries.push(RawEntry {
                        peer_id: p.peer_id.to_base58(),
                        name: p.name.clone(),
                        file_name: m.file_name.clone(),
                        model_id: m.id,
                        min_bit, max_bit,
                        num_layers: nl,
                        latency_ms: p.profile.latency_ms,
                    });
                    seen_files.insert(m.file_name.clone());
                }
            }

            // ── 3. 补充 Storage 本地文件（未在 peer 条目中出现的） ──
            if let Ok(local) = caps_mp.peer_manager.Get_Local_Peer().await {
                if let Ok(file_entries) = caps_st.list().await {
                    for fe in &file_entries {
                        let file_name = &fe.file_name;
                        if seen_files.contains(file_name) { continue; }
                        if fe.model_id != Some(model_id) { continue; }
                        let (min_bit, max_bit) = fe.layer_bitmap.as_ref()
                            .map(|b| bitmap_range(b))
                            .unwrap_or((0, 0));
                        let nl = fe.num_layers;
                        entries.push(RawEntry {
                            peer_id: local.peer_id.to_base58(),
                            name: local.name.clone(),
                            file_name: file_name.clone(),
                            model_id,
                            min_bit, max_bit,
                            num_layers: nl,
                            latency_ms: None,
                        });
                    }
                }
            }

            // ── 3.5. 合并同一 peer 的重叠条目 ──────────────────
            // 先计算 pipeline 坐标，按 peer_id 分组，
            // 重叠条目取并集范围（保留最宽分片的 file_name）
            {
                use std::collections::HashMap;
                // 计算 pipeline 坐标，附带原始信息
                struct Ranged {
                    peer_id: String, name: String, file_name: String,
                    model_id: u32, start: u32, end: u32, latency_ms: Option<u64>,
                }
                let mut ranged: Vec<Ranged> = entries.iter().map(|re| {
                    let (s, e) = adjust_range(re.min_bit, re.max_bit, &re.file_name, re.num_layers);
                    Ranged {
                        peer_id: re.peer_id.clone(), name: re.name.clone(),
                        file_name: re.file_name.clone(), model_id: re.model_id,
                        start: s, end: e, latency_ms: re.latency_ms,
                    }
                }).collect();

                // 按 peer_id 分组
                let mut groups: HashMap<String, Vec<Ranged>> = HashMap::new();
                for r in ranged {
                    groups.entry(r.peer_id.clone()).or_default().push(r);
                }

                // 合并每组的重叠条目
                let mut merged: Vec<RawEntry> = Vec::new();
                for (_pid, mut group) in groups {
                    // 按 layer_start 升序，同起点按范围降序（宽优先）
                    group.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));

                    let mut i = 0;
                    while i < group.len() {
                        let mut cur_end = group[i].end;
                        let mut best = i; // 当前合并块中范围最宽的条目索引
                        let mut best_width = cur_end as i64 - group[i].start as i64;
                        let mut j = i + 1;
                        while j < group.len() && group[j].start <= cur_end {
                            // 重叠：扩展并集终点
                            if group[j].end > cur_end {
                                cur_end = group[j].end;
                            }
                            let w = group[j].end as i64 - group[j].start as i64;
                            if w > best_width {
                                best = j;
                                best_width = w;
                            }
                            j += 1;
                        }
                        // 以最宽分片的 file_name 代表合并后的条目
                        merged.push(RawEntry {
                            peer_id: group[i].peer_id.clone(),
                            name: group[i].name.clone(),
                            file_name: group[best].file_name.clone(),
                            model_id: group[i].model_id,
                            min_bit: group[i].start,
                            max_bit: cur_end,
                            num_layers: None,
                            latency_ms: group[i].latency_ms,
                        });
                        i = j;
                    }
                }
                entries = merged;
            }

            // ── 4. 构建 Lua 结果表 ────────────────────────
            // 注：min_bit/max_bit 此时已存储 pipeline 坐标 (经由步骤 3.5 的 adjust_range)
            let result = lua.create_table()?;
            for (i, e) in entries.iter().enumerate() {
                let layer_start = e.min_bit;
                let layer_end = e.max_bit;

                let entry = lua.create_table()?;
                entry.set("peer_id", e.peer_id.clone())?;
                entry.set("name", e.name.clone())?;
                entry.set("layer_start", layer_start)?;
                entry.set("layer_end", layer_end)?;
                entry.set("file_name", e.file_name.clone())?;
                entry.set("model_id", e.model_id)?;

                // devices: 所有节点固定 2 GPU
                let devices = lua.create_table()?;
                devices.set(1, "cuda:0")?;
                devices.set(2, "cuda:1")?;
                entry.set("devices", devices)?;

                // profile
                let profile = lua.create_table()?;
                if let Some(lat) = e.latency_ms {
                    profile.set("latency_ms", lat)?;
                }
                entry.set("profile", profile)?;

                result.set(i + 1, entry)?;
            }
            Ok(result)
        }
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

    // ─── open_tensor_stream ──────────────────────────────
    let caps_net = capabilities.clone();
    network.set("open_tensor_stream", lua.create_async_function(move |_, (peer_str, inference_id): (String, u64)| {
        let caps_net = caps_net.clone();
        async move {
            let peer = peer_str.parse::<PeerId>()
                .map_err(|e| mlua::Error::runtime(format!("无效 PeerId: {}", e)))?;
            let stream = caps_net.network.open_tensor_stream(peer, inference_id).await
                .map_err(|e| mlua::Error::runtime(format!("open_tensor_stream: {}", e)))?;
            Ok(NetworkStream::new(stream, caps_net))
        }
    })?)?;

    // ─── accept_tensor_stream ────────────────────────────
    let caps_net = capabilities.clone();
    network.set("accept_tensor_stream", lua.create_async_function(move |_, (inference_id, timeout): (u64, u64)| {
        let caps_net = caps_net.clone();
        async move {
            let stream = caps_net.network.accept_tensor_stream(inference_id, timeout).await
                .map_err(|e| mlua::Error::runtime(format!("accept_tensor_stream: {}", e)))?;
            Ok(NetworkStream::new(stream, caps_net))
        }
    })?)?;

    // ─── send_tensor ─────────────────────────────────────
    network.set("send_tensor", lua.create_async_function(move |_, (stream, tensor, offset): (mlua::AnyUserData, mlua::AnyUserData, u64)| {
        async move {
            let t = tensor.borrow::<LuaTensor>()
                .map_err(|e| mlua::Error::runtime(format!("send_tensor: {e}")))?;
            let bytes = tensor_to_bytes(&t)
                .map_err(|e| mlua::Error::runtime(e))?;
            let stream_ud = stream.borrow::<NetworkStream>()
                .map_err(|e| mlua::Error::runtime(format!("send_tensor: {e}")))?;
            let mut guard = stream_ud.stream.lock().unwrap_or_else(|e| e.into_inner());
            Send_Tensor_Frame(&mut *guard, offset, &bytes).await
                .map_err(|e| mlua::Error::runtime(format!("send_tensor: {e}")))?;
            Ok(())
        }
    })?)?;

    // ─── recv_tensor ─────────────────────────────────────
    network.set("recv_tensor", lua.create_async_function(move |_, (stream, device): (mlua::AnyUserData, String)| {
        async move {
            let stream_ud = stream.borrow::<NetworkStream>()
                .map_err(|e| mlua::Error::runtime(format!("recv_tensor: {e}")))?;
            let mut guard = stream_ud.stream.lock().unwrap_or_else(|e| e.into_inner());
            let mut buffer = Tensor_Buffer::New(256 * 1024);  // 256KB, auto-grows for prefill
            let offset = Receive_Tensor_Frame(&mut *guard, &mut buffer).await
                .map_err(|e| mlua::Error::runtime(format!("recv_tensor: {e}")))?;
            drop(guard);
            drop(stream_ud);
            if offset == crate::network::tensor_stream::protocol::TENSOR_EOF_OFFSET {
                return Err(mlua::Error::runtime("recv_tensor: received EOF"));
            }
            let tensor = bytes_to_tensor_str(buffer.As_Slice(), &device)
                .map_err(|e| mlua::Error::runtime(e))?;
            Ok((LuaTensor(tensor), offset))
        }
    })?)?;

    // ─── send_eof ────────────────────────────────────────
    network.set("send_eof", lua.create_async_function(move |_, stream: mlua::AnyUserData| {
        async move {
            let stream_ud = stream.borrow::<NetworkStream>()
                .map_err(|e| mlua::Error::runtime(format!("send_eof: {e}")))?;
            let mut guard = stream_ud.stream.lock().unwrap_or_else(|e| e.into_inner());
            Send_EOF(&mut *guard).await
                .map_err(|e| mlua::Error::runtime(format!("send_eof: {e}")))?;
            Ok(())
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
    use crate::vm::engine::LuaContext;

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
            local t = sess:tensorize({1, 2, 3, 4, 5})
            return t:dims()
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
            local t = sess:tensorize(ids)
            return t:dims()
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
    fn test_ml_forward_with_lua_tensor() {
        let lua = LuaContext::new().expect("create lua");
        register_ml_caps(&lua).expect("register ml caps");

        // tensorize 返回 LuaTensor → 传入 forward
        // 空壳无模型，forward 内部报 "no model loaded"，但证明 LuaTensor 类型桥接正确
        let result = lua.load(r#"
            local sess = ml.new("cpu")
            local t = sess:tensorize({1, 2, 3})
            sess:forward(t, 0)
        "#).eval::<mlua::Value>();

        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("no model"),
                    "期望 'no model' 错误（无模型已加载），实际: {msg}"
                );
            }
            Ok(_) => panic!("空壳 forward 应该报错"),
        }
    }

    #[test]
    fn test_ml_script_calls_multiple_methods() {
        let lua = LuaContext::new().expect("create lua");
        register_ml_caps(&lua).expect("register ml caps");

        let script = r#"
            COMMAND = "ml_demo"
            function execute(params)
                local sess = ml.new(params.device or "cpu")
                local t = sess:tensorize({1, 2, 3})
                local dims = t:dims()

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
