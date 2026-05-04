//Presented by KeJi
//Date ： 2026-05-04

//! Scheduler Capability — 分布式流水线拓扑规划接口
//!
//! 定义 `Scheduler_Capability` trait 以及相关数据结构。
//! Orchestrator 通过此 trait 调用调度策略，实现解耦。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use async_trait::async_trait;
use libp2p::PeerId;
use crate::ml_engine::ml_thread_engine_instruction::Model_Info;
use crate::peer_management::PeerInfo;

// ─── 数据结构 ────────────────────────────────────────────────

/// Scheduler 输入（由 PlanPipeline handler 组装）
pub struct Scheduler_Input {
    /// 模型分析结果（层数、架构等）— 来自 AnalyzeModel 指令
    pub model_info: Model_Info,
    /// 可用节点列表（空闲 + 已连接的）— 来自 PeerManager.Get_Idle_Peers()
    pub available_peers: Vec<PeerInfo>,
    /// Coordinator 自己的 PeerId — 来自 Network.get_local_peer_id()
    pub local_peer_id: PeerId,
    /// 全局唯一推理 ID — 由 Core 在 route_user 中预生成
    pub inference_id: u64,
}

/// Pipeline 拓扑规划（完整 plan）
#[derive(Debug, Clone)]
pub struct Pipeline_Plan {
    /// 全局唯一推理 ID
    pub inference_id: u64,
    /// Coordinator 负责的层范围 — GGUF 层编号（起始，含；0 = embedding）
    pub coord_layer_start: usize,
    /// Coordinator 负责的层范围 — GGUF 层编号（结束，含）
    pub coord_layer_end: usize,
    /// Coordinator 的出站目标（第一个 Worker；无 Worker 时为 None）
    pub coord_outbound_target: Option<PeerId>,
    /// Worker 分配列表（按流水线顺序排列）
    pub workers: Vec<Worker_Assignment>,
}

/// 单个 Worker 的分配信息
#[derive(Debug, Clone)]
pub struct Worker_Assignment {
    /// Worker 节点 ID
    pub peer_id: PeerId,
    /// 负责的层范围 — GGUF 层编号（起始，含；block_index + 1）
    pub layer_start: usize,
    /// 负责的层范围 — GGUF 层编号（结束，含；最后 Worker 为 num_layers+1 以包含 output head）
    pub layer_end: usize,
    /// 出站目标节点（该 Worker 需要向其打开张量流）
    pub outbound_target: PeerId,
    /// 模型分片 file_id（如果已分发；None 表示需要先分发）
    pub model_shard_id: Option<String>,
    /// 设备偏好
    pub device: String,
}

/// Scheduler 错误
#[derive(Debug, thiserror::Error)]
pub enum Scheduler_Error {
    #[error("模型层数为 0")]
    Zero_Layers,
}

// ─── Trait 定义 ──────────────────────────────────────────────

/// Scheduler 能力接口
///
/// 提供分布式流水线拓扑规划功能。
/// 不同实现可采用不同的调度策略（均分、加权、学习型等）。
#[async_trait]
pub trait Scheduler_Capability: Send + Sync {
    /// 规划 Pipeline 拓扑
    ///
    /// 给定模型信息 + 可用节点列表 → 输出分布式拓扑 plan。
    /// 当前实现为纯计算（无实际 async I/O），但保留 async 签名以兼容未来可能的
    /// 异步查询需求（如从远端获取节点负载信息等）。
    async fn Plan_Pipeline(&self, input: Scheduler_Input) -> Result<Pipeline_Plan, Scheduler_Error>;
}

// ─── 单元测试 ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_error_display() {
        let err = Scheduler_Error::Zero_Layers;
        assert_eq!(format!("{}", err), "模型层数为 0");
    }

    #[test]
    fn test_pipeline_plan_debug() {
        let plan = Pipeline_Plan {
            inference_id: 42,
            coord_layer_start: 0,
            coord_layer_end: 16,
            coord_outbound_target: None,
            workers: vec![],
        };
        let debug_str = format!("{:?}", plan);
        assert!(debug_str.contains("42"));
        assert!(debug_str.contains("16"));
    }

    #[test]
    fn test_worker_assignment_clone() {
        let peer_id = PeerId::random();
        let target = PeerId::random();
        let assignment = Worker_Assignment {
            peer_id,
            layer_start: 10,
            layer_end: 20,
            outbound_target: target,
            model_shard_id: Some("shard_10_20".to_string()),
            device: "cuda".to_string(),
        };
        let cloned = assignment.clone();
        assert_eq!(cloned.peer_id, peer_id);
        assert_eq!(cloned.layer_start, 10);
        assert_eq!(cloned.device, "cuda");
    }
}
