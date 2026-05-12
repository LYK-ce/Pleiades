//Presented by KeJi
//Date : 2026-05-12

//! Scheduler_Weighted — 综合成本加权分配的便捷包装
//!
//! 固定使用 `Scheduler_Strategy::Weighted` 策略，内部委托给 `Scheduler_Service`。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use async_trait::async_trait;
use super::capability::{
    Scheduler_Capability, Scheduler_Strategy, Scheduler_Input,
    Scheduler_Error, Pipeline_Plan,
};
use super::service::Scheduler_Service;

pub struct Scheduler_Weighted {
    inner: Scheduler_Service,
}

impl Scheduler_Weighted {
    pub fn New() -> Self {
        Self {
            inner: Scheduler_Service::New(),
        }
    }
}

#[async_trait]
impl Scheduler_Capability for Scheduler_Weighted {
    async fn Plan_Pipeline(&self, mut input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error> {
        input.strategy = Scheduler_Strategy::Weighted;
        self.inner.Plan_Pipeline(input).await
    }
}

// ─── 单元测试 ————————————————————————————————————————————

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::PeerId;
    use std::collections::HashMap;
    use std::time::Duration;
    use crate::peer_management::{PeerInfo, PeerCapability};
    use crate::ml_engine::pipeline::Model_Info;

    fn make_model_info(num_layers: usize) -> Model_Info {
        let total = num_layers + 2;
        Model_Info {
            architecture: "test".to_string(),
            num_layers,
            embedding_length: 1024,
            has_input_head: true,
            has_output_head: true,
            has_tokenizer: true,
            eos_token_id: 0,
            layer_sizes_bytes: vec![1048576; total],
            num_kv_heads: 4,
            head_dim: 128,
            context_length: 1024,
            vocab_size: 32000,
        }
    }

    fn make_peer(model_id: &str, has_gpu: bool, layer_time_secs: f64, memory_mb: u64, bandwidth_mbps: u64) -> PeerInfo {
        let mut peer = PeerInfo::new(PeerId::random(), vec![]);
        let mut layer_time = HashMap::new();
        layer_time.insert(model_id.to_string(), Duration::from_secs_f64(layer_time_secs));
        peer.capability = Some(PeerCapability {
            has_gpu,
            memory_mb,
            compute_score: 1.0,
            supported_models: vec![],
            layer_time,
        });
        peer.bandwidth_mbps = Some(bandwidth_mbps);
        peer
    }

    #[tokio::test]
    async fn test_weighted_filters_unprofiled() {
        let svc = Scheduler_Weighted::New();
        let local = PeerId::random();
        let model_id = "test_model";

        let coord = make_peer(model_id, true, 0.01, 32768, 1000);
        let good = make_peer(model_id, true, 0.02, 16384, 500);
        let bad = {
            let mut p = PeerInfo::new(PeerId::random(), vec![]);
            p.capability = Some(PeerCapability {
                has_gpu: false, memory_mb: 8192, compute_score: 1.0,
                supported_models: vec![], layer_time: HashMap::new(),
            });
            p
        };

        let input = Scheduler_Input {
            strategy: Scheduler_Strategy::Weighted,
            model_info: make_model_info(32),
            model_id: model_id.to_string(),
            available_peers: vec![good.clone(), bad],
            coordinator_info: coord,
            local_peer_id: local,
            inference_id: 1,
        };
        let plan = svc.Plan_Pipeline(input).await.unwrap();
        assert_eq!(plan.workers.len(), 1);
        assert_eq!(plan.workers[0].peer_id, good.peer_id);
    }

    #[tokio::test]
    async fn test_weighted_fast_gets_more() {
        let svc = Scheduler_Weighted::New();
        let local = PeerId::random();
        let model_id = "test_model";

        let coord = make_peer(model_id, true, 0.01, 32768, 1000);
        let fast = make_peer(model_id, true, 0.01, 32768, 1000);
        let slow = make_peer(model_id, true, 0.05, 32768, 1000);

        let input = Scheduler_Input {
            strategy: Scheduler_Strategy::Weighted,
            model_info: make_model_info(32),
            model_id: model_id.to_string(),
            available_peers: vec![fast.clone(), slow.clone()],
            coordinator_info: coord,
            local_peer_id: local,
            inference_id: 2,
        };
        let plan = svc.Plan_Pipeline(input).await.unwrap();
        let fast_layers = plan.workers.iter()
            .find(|w| w.peer_id == fast.peer_id)
            .map(|w| w.layer_end - w.layer_start + 1).unwrap();
        let slow_layers = plan.workers.iter()
            .find(|w| w.peer_id == slow.peer_id)
            .map(|w| w.layer_end - w.layer_start + 1).unwrap();
        assert!(fast_layers > slow_layers);
    }

    #[tokio::test]
    async fn test_weighted_low_bandwidth_penalized() {
        let svc = Scheduler_Weighted::New();
        let local = PeerId::random();
        let model_id = "test_model";

        let mut model = make_model_info(16);
        model.embedding_length = 4096;

        let coord = make_peer(model_id, true, 0.01, 32768, 1000);
        let high_bw = make_peer(model_id, true, 0.02, 32768, 1000);
        let low_bw = make_peer(model_id, true, 0.02, 32768, 10);

        let input = Scheduler_Input {
            strategy: Scheduler_Strategy::Weighted,
            model_info: model,
            model_id: model_id.to_string(),
            available_peers: vec![high_bw.clone(), low_bw.clone()],
            coordinator_info: coord,
            local_peer_id: local,
            inference_id: 3,
        };
        let plan = svc.Plan_Pipeline(input).await.unwrap();
        let hi_layers = plan.workers.iter()
            .find(|w| w.peer_id == high_bw.peer_id)
            .map(|w| w.layer_end - w.layer_start + 1).unwrap();
        let lo_layers = plan.workers.iter()
            .find(|w| w.peer_id == low_bw.peer_id)
            .map(|w| w.layer_end - w.layer_start + 1).unwrap();
        assert!(hi_layers > lo_layers);
    }
}
