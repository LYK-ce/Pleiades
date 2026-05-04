//Presented by KeJi
//Date ： 2026-05-04

//! Scheduler Service — 均匀分配策略实现
//!
//! 当前使用**均匀分配**策略：
//! - 总节点数 = 1 (Coordinator) + K (Workers)
//! - 每节点分配 N / (K+1) 层
//! - 余数全给 Coordinator（它通常有最好的设备）
//! - 拓扑为环形：Coordinator → W1 → W2 → ... → Wk → Coordinator
//!
//! 无可用 Worker 时退化为单机（所有层由 Coordinator 执行）。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use async_trait::async_trait;
use super::capability::{
    Scheduler_Capability, Scheduler_Input, Scheduler_Error,
    Pipeline_Plan, Worker_Assignment,
};

// ─── 实现结构体 ──────────────────────────────────────────────

/// Scheduler 服务实例 — 均匀分配策略
///
/// 无状态结构体，所有计算逻辑为纯函数。
pub struct Scheduler_Service;

impl Scheduler_Service {
    /// 创建 Scheduler 服务实例
    pub fn New() -> Self {
        Self
    }
}

#[async_trait]
impl Scheduler_Capability for Scheduler_Service {
    /// 规划 Pipeline 拓扑（均匀分配策略）
    ///
    /// 使用 GGUF 模型的完整层编号体系进行均分：
    ///   Layer 0 = embedding, Layer 1..=N = transformer blocks, Layer N+1 = output
    ///   总 GGUF 层数 = N + 2
    ///
    /// 均匀分配策略：
    /// - 总节点数 = 1 (Coordinator) + K (Workers)
    /// - 每节点分配 (N+2) / (K+1) 层
    /// - 余数全给 Coordinator（它通常有最好的设备）
    /// - 拓扑为环形：Coordinator → W1 → W2 → ... → Wk → Coordinator
    /// - 输出使用 inclusive end，直接对应 GGUF_Load_Model 的参数
    ///
    /// 无可用 Worker 时退化为单机（所有层由 Coordinator 执行）。
    async fn Plan_Pipeline(&self, input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error> {
        let num_layers = input.model_info.num_layers;
        if num_layers == 0 {
            return Err(Scheduler_Error::Zero_Layers);
        }

        // 总 GGUF 层数 = transformer blocks + embedding + output
        let total_gguf_layers = num_layers + 2;

        let workers = &input.available_peers;

        // 无可用 Worker → 退化为单机（Coordinator 处理全部层）
        if workers.is_empty() {
            return Ok(Pipeline_Plan {
                inference_id: input.inference_id,
                coord_layer_start: 0,
                coord_layer_end: total_gguf_layers - 1, // inclusive: 最后一层 = output
                coord_outbound_target: None,
                workers: vec![],
            });
        }

        let node_count = workers.len() + 1; // +1 for Coordinator
        let chunk = total_gguf_layers / node_count;
        let remainder = total_gguf_layers % node_count;

        // Coordinator 取前段 + 余数（exclusive end 用于内部计算）
        let coord_end_excl = chunk + remainder;

        // Workers 按顺序分配后续层
        let mut assignments = Vec::new();
        let mut cursor = coord_end_excl;

        for (i, peer) in workers.iter().enumerate() {
            let start = cursor;
            let end_excl = cursor + chunk;

            // 下一跳：最后一个 Worker 出站回 Coordinator
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
                layer_end: end_excl - 1, // inclusive end
                outbound_target,
                model_shard_id: None,
                device,
            });

            cursor = end_excl;
        }

        // Coordinator 出站目标 = 第一个 Worker
        let coord_outbound_target = Some(workers[0].peer_id);

        Ok(Pipeline_Plan {
            inference_id: input.inference_id,
            coord_layer_start: 0,
            coord_layer_end: coord_end_excl - 1, // inclusive end
            coord_outbound_target,
            workers: assignments,
        })
    }
}

// ─── 单元测试 ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::PeerId;
    use crate::peer_management::{PeerInfo, PeerCapability};
    use crate::ml_engine::ml_thread_engine_instruction::Model_Info;

    fn make_model_info(num_layers: usize) -> Model_Info {
        Model_Info {
            architecture: "test".to_string(),
            num_layers,
            has_input_head: true,
            has_output_head: true,
            has_tokenizer: true,
            eos_token_id: 0,
        }
    }

    fn make_peer(has_gpu: bool) -> PeerInfo {
        let mut peer = PeerInfo::new(PeerId::random(), vec![]);
        peer.capability = Some(PeerCapability {
            has_gpu,
            memory_mb: 8192,
            compute_score: 1.0,
            supported_models: vec![],
        });
        peer
    }

    #[tokio::test]
    async fn test_no_workers_degrades_to_single() {
        let svc = Scheduler_Service::New();
        let input = Scheduler_Input {
            model_info: make_model_info(32),
            available_peers: vec![],
            local_peer_id: PeerId::random(),
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
            model_info: make_model_info(32),
            available_peers: vec![worker],
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
            model_info: make_model_info(32),
            available_peers: vec![w1, w2],
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
        let input = Scheduler_Input {
            model_info: make_model_info(0),
            available_peers: vec![],
            local_peer_id: PeerId::random(),
            inference_id: 4,
        };
        assert!(svc.Plan_Pipeline(input).await.is_err());
    }
}
