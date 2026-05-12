//Presented by KeJi
//Date ： 2026-05-04

//! Scheduler Service — 统一入口，策略内部分发
//!
//! 策略在构造时指定，通过 `Scheduler_Service::New(strategy)` 创建。
//! - `Scheduler_Strategy::Uniform` — 均匀分配策略
//! - `Scheduler_Strategy::Weighted` — 综合成本加权分配策略
//!
//! 无可用 Worker 时退化为单机（所有层由 Coordinator 执行）。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::time::Duration;
use async_trait::async_trait;
use libp2p::PeerId;

use crate::ml_engine::pipeline::Model_Info;
use crate::peer_management::PeerInfo;
use super::capability::{
    Scheduler_Capability, Scheduler_Strategy, Scheduler_Input, Scheduler_Error,
    Pipeline_Plan, Worker_Assignment,
};

// ——— 常量 ————————————————————————————————————————————————

const KV_CACHE_CONTEXT: u64 = 1024;
const ACTIVATION_OVERHEAD_RATIO: f64 = 0.15;
/// 每层内存估算膨胀系数（CUDA 运行时开销、fragmentation 等）
const MEMORY_ESTIMATION_FACTOR: f64 = 1.2;

// ─── 实现结构体 ──────────────────────────────────────────────

/// Scheduler 服务实例
///
/// 所有计算逻辑为纯函数，策略通过 `Scheduler_Input.strategy` 选择。
pub struct Scheduler_Service;

impl Scheduler_Service {
    pub fn New() -> Self {
        Self
    }
}

#[async_trait]
impl Scheduler_Capability for Scheduler_Service {
    async fn Plan_Pipeline(&self, input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error> {
        match input.strategy {
            Scheduler_Strategy::Uniform => self.plan_uniform(input),
            Scheduler_Strategy::Weighted => self.plan_weighted(input),
        }
    }
}

// ─── Uniform 策略 ——————————————————————————————————————————

impl Scheduler_Service {
    fn plan_uniform(&self, input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error> {
        let num_layers = input.model_info.num_layers;
        if num_layers == 0 {
            return Err(Scheduler_Error::Zero_Layers);
        }

        let total_gguf_layers = num_layers + 2;
        let workers = &input.available_peers;

        if workers.is_empty() {
            return Ok(single_node_plan(&input, total_gguf_layers));
        }

        let node_count = workers.len() + 1;
        let chunk = total_gguf_layers / node_count;
        let remainder = total_gguf_layers % node_count;
        let coord_end_excl = chunk + remainder;

        let mut assignments = Vec::new();
        let mut cursor = coord_end_excl;

        for (i, peer) in workers.iter().enumerate() {
            let start = cursor;
            let end_excl = cursor + chunk;

            let outbound_target = if i + 1 < workers.len() {
                workers[i + 1].peer_id
            } else {
                input.local_peer_id
            };

            let device = if peer.capability.as_ref().map_or(false, |c| c.has_gpu) {
                "cuda".to_string()
            } else {
                "cpu".to_string()
            };

            assignments.push(Worker_Assignment {
                peer_id: peer.peer_id,
                layer_start: start,
                layer_end: end_excl - 1,
                outbound_target,
                model_shard_id: None,
                device,
            });

            cursor = end_excl;
        }

        Ok(Pipeline_Plan {
            inference_id: input.inference_id,
            coord_layer_start: 0,
            coord_layer_end: coord_end_excl - 1,
            coord_outbound_target: Some(workers[0].peer_id),
            workers: assignments,
        })
    }
}

// ─── Weighted 策略 ——————————————————————————————————————————

impl Scheduler_Service {
    fn plan_weighted(&self, input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error> {
        let num_layers = input.model_info.num_layers;
        if num_layers == 0 {
            return Err(Scheduler_Error::Zero_Layers);
        }

        let total_gguf_layers = num_layers + 2;

        // Step 0: 过滤无 profile 数据的节点
        let mut eligible: Vec<PeerInfo> = Vec::new();
        for peer in &input.available_peers {
            if has_model_profile(peer, &input.model_id) {
                eligible.push(peer.clone());
            }
        }

        if eligible.is_empty() {
            tracing::info!("[weighted] Step 0: 无 eligible 节点, 退化为单机");
            return Ok(single_node_plan(&input, total_gguf_layers));
        }
        tracing::info!(
            "[weighted] Step 0: eligible={}, peers=[{}]",
            eligible.len(),
            eligible.iter().map(|p| format!("{}", p.peer_id)).collect::<Vec<_>>().join(", ")
        );

        // Step 1: 内存容量检查
        let model_id = &input.model_id;
        let coordinator_cap = input.coordinator_info.capability.as_ref();

        let coord_max_layers = if let Some(cap) = coordinator_cap {
            find_max_layers(&input.model_info, cap.memory_mb, 0)
        } else {
            total_gguf_layers
        };
        if coord_max_layers < 1 {
            return Err(Scheduler_Error::Coordinator_Memory_Insufficient {
                required_bytes: 0,
                available_bytes: coordinator_cap.map(|c| c.memory_mb).unwrap_or(0),
            });
        }

        let mut node_max_layers: Vec<usize> = Vec::with_capacity(eligible.len());
        let mut valid_peers: Vec<PeerInfo> = Vec::with_capacity(eligible.len());
        for peer in &eligible {
            let cap = peer.capability.as_ref().unwrap();
            let ml = find_max_layers(&input.model_info, cap.memory_mb, 1);
            if ml >= 1 {
                node_max_layers.push(ml);
                valid_peers.push(peer.clone());
            }
        }

        if valid_peers.is_empty() {
            return Err(Scheduler_Error::No_Eligible_Peers);
        }

        tracing::info!(
            "[weighted] Step 1: coord_max_layers={} (mem={}MB), node_max_layers={:?}",
            coord_max_layers,
            coordinator_cap.map(|c| c.memory_mb).unwrap_or(0),
            node_max_layers,
        );

        // Step 2: 综合成本加权分配
        let coord_lt = coordinator_cap
            .and_then(|c| c.layer_time.get(model_id))
            .copied()
            .unwrap_or(Duration::from_secs_f64(0.1));

        let estimated_layers = total_gguf_layers / (valid_peers.len() + 1);

        // 计算每个 worker 的 effective_cost
        let mut worker_cost_info: Vec<(PeerId, f64)> = Vec::new();
        for peer in &valid_peers {
            let lt = peer.capability.as_ref()
                .and_then(|c| c.layer_time.get(model_id))
                .copied()
                .unwrap_or(Duration::from_secs_f64(0.1));
            let comm = compute_comm_simple(peer, &input.coordinator_info, &input.model_info);
            let eff = lt.as_secs_f64() + comm / estimated_layers as f64;
            worker_cost_info.push((peer.peer_id, eff));
        }
        worker_cost_info.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        let coord_comm = compute_coord_comm(&input.coordinator_info, &valid_peers[0], &input.model_info);
        let coord_eff = coord_lt.as_secs_f64() + coord_comm / estimated_layers as f64;

        // 构建权重
        let mut weights: Vec<f64> = Vec::with_capacity(valid_peers.len() + 1);
        let mut max_layers_list: Vec<usize> = Vec::with_capacity(valid_peers.len() + 1);

        weights.push(1.0 / coord_eff.max(1e-9));
        max_layers_list.push(coord_max_layers);

        for peer in &valid_peers {
            let (_, eff) = worker_cost_info.iter().find(|(pid, _)| *pid == peer.peer_id).unwrap();
            weights.push(1.0 / eff.max(1e-9));
            let idx = valid_peers.iter().position(|p| p.peer_id == peer.peer_id).unwrap();
            max_layers_list.push(node_max_layers[idx]);
        }

        let weight_sum: f64 = weights.iter().sum();

        let mut allocated: Vec<usize> = Vec::with_capacity(weights.len());
        let mut total_allocated = 0usize;
        for i in 0..weights.len() {
            let raw = (total_gguf_layers as f64 * weights[i] / weight_sum).floor() as usize;
            let clamped = raw.min(max_layers_list[i]).max(1);
            allocated.push(clamped);
            total_allocated += clamped;
        }

        tracing::info!(
            "[weighted] Step 2: weights={:?}, max_layers={:?}, allocated={:?}, total={}/{}, remaining={}",
            weights, max_layers_list, allocated, total_allocated, total_gguf_layers,
            if total_allocated < total_gguf_layers { total_gguf_layers - total_allocated } else { 0 }
        );

        // 余数分配
        let mut remaining = if total_allocated < total_gguf_layers {
            total_gguf_layers - total_allocated
        } else {
            0
        };
        if remaining > 0 {
            let mut sorted_indices: Vec<usize> = (0..allocated.len()).collect();
            sorted_indices.sort_by(|a, b| weights[*b].partial_cmp(&weights[*a]).unwrap());
            while remaining > 0 {
                let mut distributed = false;
                for idx in sorted_indices.iter() {
                    if remaining == 0 { break; }
                    if allocated[*idx] < max_layers_list[*idx] {
                        allocated[*idx] += 1;
                        remaining -= 1;
                        distributed = true;
                    }
                }
                if !distributed { break; }
            }
        }
        // 若仍有余数，说明所有节点已到内存上限，总内存不足
        if remaining > 0 {
            return Err(Scheduler_Error::Insufficient_Memory {
                total_required_bytes: estimate_runtime_memory(&input.model_info, 0, total_gguf_layers - 1),
            });
        }

        // Step 3: 拓扑排序 + layer range
        let coord_layers = allocated[0];
        let coord_layer_start = 0;
        let coord_layer_end = coord_layers - 1;

        let mut workers_sorted: Vec<(&PeerInfo, usize)> = Vec::new();
        for (i, peer) in valid_peers.iter().enumerate() {
            workers_sorted.push((peer, allocated[i + 1]));
        }
        workers_sorted.sort_by(|a, b| {
            let ea = worker_cost_info.iter().find(|(pid, _)| *pid == a.0.peer_id).map(|(_, c)| c).unwrap_or(&999.0);
            let eb = worker_cost_info.iter().find(|(pid, _)| *pid == b.0.peer_id).map(|(_, c)| c).unwrap_or(&999.0);
            ea.partial_cmp(eb).unwrap()
        });

        let mut cursor = coord_layers;
        let mut assignments = Vec::new();
        let worker_count = workers_sorted.len();

        for (i, (peer, layers)) in workers_sorted.iter().enumerate() {
            let start = cursor;
            let end = cursor + layers - 1;

            let outbound_target = if i + 1 < worker_count {
                workers_sorted[i + 1].0.peer_id
            } else {
                input.local_peer_id
            };

            let device = if peer.capability.as_ref().map_or(false, |c| c.has_gpu) {
                "cuda".to_string()
            } else {
                "cpu".to_string()
            };

            assignments.push(Worker_Assignment {
                peer_id: peer.peer_id,
                layer_start: start,
                layer_end: end,
                outbound_target,
                model_shard_id: None,
                device,
            });

            cursor = end + 1;
        }

        let coord_outbound_target = if workers_sorted.is_empty() {
            None
        } else {
            Some(workers_sorted[0].0.peer_id)
        };

        let plan = Pipeline_Plan {
            inference_id: input.inference_id,
            coord_layer_start,
            coord_layer_end,
            coord_outbound_target,
            workers: assignments,
        };

        let workers_desc: Vec<String> = plan.workers.iter()
            .map(|w| format!("{}(layers {}-{})", w.peer_id, w.layer_start, w.layer_end))
            .collect();
        let workers_str = if workers_desc.is_empty() {
            "none".to_string()
        } else {
            workers_desc.join(", ")
        };
        tracing::info!(
            "[weighted] Plan: coord=[{},{}] ({} layers), workers=[{}], total={}",
            plan.coord_layer_start,
            plan.coord_layer_end,
            plan.coord_layer_end - plan.coord_layer_start + 1,
            workers_str,
            plan.coord_layer_end + 1 + plan.workers.iter().map(|w| w.layer_end - w.layer_start + 1).sum::<usize>()
        );

        Ok(plan)
    }
}

// ─── 辅助函数 ————————————————————————————————————————————

fn single_node_plan(input: &Scheduler_Input, total_gguf_layers: usize) -> Pipeline_Plan {
    Pipeline_Plan {
        inference_id: input.inference_id,
        coord_layer_start: 0,
        coord_layer_end: total_gguf_layers - 1,
        coord_outbound_target: None,
        workers: vec![],
    }
}

fn has_model_profile(peer: &PeerInfo, model_id: &str) -> bool {
    peer.capability.as_ref()
        .map_or(false, |c| c.layer_time.contains_key(model_id))
}

fn estimate_runtime_memory(model_info: &Model_Info, start: usize, end: usize) -> u64 {
    let mut weight_bytes: u64 = 0;
    for l in start..=end {
        if l < model_info.layer_sizes_bytes.len() {
            weight_bytes += model_info.layer_sizes_bytes[l] as u64;
        }
    }

    let block_start = start.max(1);
    let block_end = end.min(model_info.num_layers);
    let num_blocks = if block_start <= block_end { block_end - block_start + 1 } else { 0 };
    let kv_cache_bytes = 2u64
        * num_blocks as u64
        * model_info.num_kv_heads as u64
        * model_info.head_dim as u64
        * KV_CACHE_CONTEXT
        * 4u64;

    let activation_overhead = (weight_bytes as f64 * ACTIVATION_OVERHEAD_RATIO) as u64;

    weight_bytes + kv_cache_bytes + activation_overhead
}

fn find_max_layers(model_info: &Model_Info, memory_mb: u64, layer_start: usize) -> usize {
    let max_layer = model_info.num_layers + 1;
    if layer_start > max_layer {
        return 0;
    }
    let memory_bytes = (memory_mb * 1024 * 1024) as f64;
    for end in layer_start..=max_layer {
        let estimated = estimate_runtime_memory(model_info, layer_start, end) as f64 * MEMORY_ESTIMATION_FACTOR;
        if estimated > memory_bytes {
            if end == layer_start {
                return 0;
            }
            return end - layer_start;
        }
    }
    max_layer - layer_start + 1
}

fn compute_comm_simple(peer: &PeerInfo, coord: &PeerInfo, model_info: &Model_Info) -> f64 {
    let hidden_bytes = (model_info.embedding_length * 4) as f64;
    let bw_coord = coord.bandwidth_mbps.unwrap_or(1000) as f64;
    let bw_peer = peer.bandwidth_mbps.unwrap_or(1000) as f64;
    let lat_coord = coord.latency_ms.unwrap_or(0) as f64 / 1000.0;
    let lat_peer = peer.latency_ms.unwrap_or(0) as f64 / 1000.0;

    let bw = bw_coord.min(bw_peer);
    let lat = (lat_coord + lat_peer) / 2.0;
    let transfer_time = if bw > 0.0 {
        hidden_bytes / (bw * 1_000_000.0 / 8.0)
    } else {
        0.0
    };

    (transfer_time + lat) * 2.0
}

fn compute_coord_comm(coord: &PeerInfo, first_worker: &PeerInfo, model_info: &Model_Info) -> f64 {
    let hidden_bytes = (model_info.embedding_length * 4) as f64;
    let bw_coord = coord.bandwidth_mbps.unwrap_or(1000) as f64;
    let bw_peer = first_worker.bandwidth_mbps.unwrap_or(1000) as f64;
    let lat_coord = coord.latency_ms.unwrap_or(0) as f64 / 1000.0;
    let lat_peer = first_worker.latency_ms.unwrap_or(0) as f64 / 1000.0;

    let bw = bw_coord.min(bw_peer);
    let lat = (lat_coord + lat_peer) / 2.0;
    if bw > 0.0 {
        hidden_bytes / (bw * 1_000_000.0 / 8.0) + lat
    } else {
        lat
    }
}

// ─── 单元测试 ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use libp2p::PeerId;
    use crate::peer_management::{PeerInfo, PeerCapability};
    use crate::ml_engine::pipeline::Model_Info;
    use super::super::capability::Scheduler_Strategy;

    fn make_model_info(num_layers: usize) -> Model_Info {
        let total_gguf_layers = num_layers + 2;
        Model_Info {
            architecture: "test".to_string(),
            num_layers,
            embedding_length: 1024,
            has_input_head: true,
            has_output_head: true,
            has_tokenizer: true,
            eos_token_id: 0,
            layer_sizes_bytes: vec![4096; total_gguf_layers],
            num_kv_heads: 4,
            head_dim: 128,
            context_length: 1024,
            vocab_size: 32000,
        }
    }

    fn make_peer(has_gpu: bool) -> PeerInfo {
        let mut peer = PeerInfo::new(PeerId::random(), vec![]);
        peer.capability = Some(PeerCapability {
            has_gpu,
            memory_mb: 8192,
            compute_score: 1.0,
            supported_models: vec![],
            layer_time: HashMap::new(),
        });
        peer
    }

    #[tokio::test]
    async fn test_no_workers_degrades_to_single() {
        let svc = Scheduler_Service::New();
        let local = PeerId::random();
        let input = Scheduler_Input {
            strategy: Scheduler_Strategy::Uniform,
            model_info: make_model_info(32),
            model_id: "test.gguf".to_string(),
            available_peers: vec![],
            coordinator_info: PeerInfo::new(local, vec![]),
            local_peer_id: local,
            inference_id: 1,
        };
        let plan = svc.Plan_Pipeline(input).await.unwrap();
        // total_gguf_layers = 32+2 = 34, 单机 → coord 持有全部 [0, 33]
        assert_eq!(plan.workers.len(), 0);
        assert_eq!(plan.coord_layer_start, 0);
        assert_eq!(plan.coord_layer_end, 33); // inclusive: output layer = N+1 = 33
        assert!(plan.coord_outbound_target.is_none());
    }

    #[tokio::test]
    async fn test_one_worker_splits_evenly() {
        let svc = Scheduler_Service::New();
        let local = PeerId::random();
        let worker = make_peer(false);
        let worker_id = worker.peer_id;

        let input = Scheduler_Input {
            strategy: Scheduler_Strategy::Uniform,
            model_info: make_model_info(32),
            model_id: "test.gguf".to_string(),
            available_peers: vec![worker],
            coordinator_info: PeerInfo::new(local, vec![]),
            local_peer_id: local,
            inference_id: 2,
        };
        let plan = svc.Plan_Pipeline(input).await.unwrap();

        // total_gguf_layers = 34, chunk = 34/2 = 17, remainder = 0
        // Coordinator: [0, 16] (17 layers: embedding + blk.0-15)
        // Worker:      [17, 33] (17 layers: blk.16-31 + output)
        assert_eq!(plan.coord_layer_start, 0);
        assert_eq!(plan.coord_layer_end, 16);
        assert_eq!(plan.workers.len(), 1);
        assert_eq!(plan.workers[0].layer_start, 17);
        assert_eq!(plan.workers[0].layer_end, 33);
        assert_eq!(plan.workers[0].outbound_target, local);
        assert_eq!(plan.coord_outbound_target, Some(worker_id));
    }

    #[tokio::test]
    async fn test_two_workers_with_remainder() {
        let svc = Scheduler_Service::New();
        let local = PeerId::random();
        let w1 = make_peer(true);
        let w2 = make_peer(false);
        let w1_id = w1.peer_id;
        let w2_id = w2.peer_id;

        let input = Scheduler_Input {
            strategy: Scheduler_Strategy::Uniform,
            model_info: make_model_info(32),
            model_id: "test.gguf".to_string(),
            available_peers: vec![w1, w2],
            coordinator_info: PeerInfo::new(local, vec![]),
            local_peer_id: local,
            inference_id: 3,
        };
        let plan = svc.Plan_Pipeline(input).await.unwrap();

        // total_gguf_layers = 34, chunk = 34/3 = 11, remainder = 1
        // Coordinator: [0, 11] (12 layers: embedding + blk.0-10)
        // Worker 0:    [12, 22] (11 layers: blk.11-21)
        // Worker 1:    [23, 33] (11 layers: blk.22-31 + output)
        assert_eq!(plan.coord_layer_end, 11);
        assert_eq!(plan.workers[0].layer_start, 12);
        assert_eq!(plan.workers[0].layer_end, 22);
        assert_eq!(plan.workers[0].outbound_target, w2_id);
        assert_eq!(plan.workers[0].device, "cuda"); // has_gpu = true
        assert_eq!(plan.workers[1].layer_start, 23);
        assert_eq!(plan.workers[1].layer_end, 33);
        assert_eq!(plan.workers[1].outbound_target, local);
        assert_eq!(plan.workers[1].device, "cpu"); // has_gpu = false
        assert_eq!(plan.coord_outbound_target, Some(w1_id));
    }

    #[tokio::test]
    async fn test_zero_layers_error() {
        let svc = Scheduler_Service::New();
        let local = PeerId::random();
        let input = Scheduler_Input {
            strategy: Scheduler_Strategy::Uniform,
            model_info: make_model_info(0),
            model_id: "test.gguf".to_string(),
            available_peers: vec![],
            coordinator_info: PeerInfo::new(local, vec![]),
            local_peer_id: local,
            inference_id: 4,
        };
        assert!(svc.Plan_Pipeline(input).await.is_err());
    }
}
