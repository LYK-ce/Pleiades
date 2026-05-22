//Presented by KeJi
//Date ： 2026-05-22

//! Batch 协议 — Session Manager ↔ ML Thread 接口类型
//!
//! 定义 batch 请求/响应结构、slot 映射和 sample 逻辑。
//! ML Thread 侧暂不实现，Session Manager 侧负责拼 batch + sample + 分发。

use std::collections::HashMap;

// ─── Batch 类型 ─────────────────────────────────────────────

/// Session Manager → ML Thread：batch 推理请求
#[derive(Debug, Clone)]
pub struct BatchRequest {
    /// 每个 slot 的 token 序列（已 padding 到同一长度）
    pub token_batches: Vec<Vec<u32>>,
    /// batch index → (session_id, slot_id) 映射
    pub slot_order: Vec<(String, usize)>,
}

/// ML Thread → Session Manager：batch 推理结果
#[derive(Debug, Clone)]
pub struct BatchResult {
    /// 每个 slot 的 logits [vocab_size]，vocab_size 可能不同
    pub logits_batches: Vec<Vec<f32>>,
    /// batch index → (session_id, slot_id) 映射（原样返回）
    pub slot_order: Vec<(String, usize)>,
}

// ─── Batch 组装 ─────────────────────────────────────────────

/// 从多个 slot 的 token 历史收集 dirty slot，拼成 BatchRequest
///
/// 返回 None 表示没有 dirty slot。
pub fn assemble_batch(
    dirty_slots: &[(String, usize, Vec<u32>)],  // (session_id, slot_id, token_buf)
) -> Option<BatchRequest> {
    if dirty_slots.is_empty() {
        return None;
    }

    // 找到 max_seqlen
    let max_len = dirty_slots.iter().map(|(_, _, buf)| buf.len()).max().unwrap_or(0);
    if max_len == 0 {
        return None;
    }

    let mut token_batches = Vec::with_capacity(dirty_slots.len());
    let mut slot_order = Vec::with_capacity(dirty_slots.len());

    for (sess_id, slot_id, buf) in dirty_slots {
        let mut padded = buf.clone();
        padded.resize(max_len, 0); // 用 0 做 padding（通常 0 是 pad token）
        token_batches.push(padded);
        slot_order.push((sess_id.clone(), *slot_id));
    }

    Some(BatchRequest { token_batches, slot_order })
}

// ─── Sample 逻辑 ────────────────────────────────────────────

/// Temperature sampling: 从 logits 向量采样一个 token
///
/// 算法: logits / temperature → softmax → categorical sample
pub fn sample_logits(logits: &[f32], temperature: f64) -> u32 {
    if logits.is_empty() {
        return 0;
    }

    let t = temperature.max(1e-6) as f32;

    // 1. temperature scaling
    let scaled: Vec<f32> = logits.iter().map(|&x| x / t).collect();

    // 2. softmax: exp(x - max) / sum(exp(x - max))
    let max_val = scaled.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum: Vec<f32> = scaled.iter().map(|&x| (x - max_val).exp()).collect();
    let sum: f32 = exp_sum.iter().sum();

    if sum == 0.0 {
        return 0;
    }

    let probs: Vec<f32> = exp_sum.iter().map(|&x| x / sum).collect();

    // 3. categorical sample
    let r: f32 = fast_random();
    let mut cumulative = 0.0f32;
    for (i, &p) in probs.iter().enumerate() {
        cumulative += p;
        if r < cumulative {
            return i as u32;
        }
    }

    // 浮点精度兜底：返回最后一个
    (probs.len() - 1) as u32
}

/// 简单的伪随机数 [0, 1)
fn fast_random() -> f32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos as f32) / 1_000_000_000.0
}

// ─── 分发结果 ───────────────────────────────────────────────

/// 对 BatchResult 逐 slot sample + 返回 (slot_id → token_id) 映射
///
/// 返回每个 slot 采样的 token，按 slot_order 顺序排列。
pub fn sample_batch(
    result: &BatchResult,
    temperatures: &HashMap<(String, usize), f64>,
) -> Vec<((String, usize), u32)> {
    result
        .logits_batches
        .iter()
        .zip(result.slot_order.iter())
        .map(|(logits, key)| {
            let temp = temperatures.get(key).copied().unwrap_or(0.8);
            let token = sample_logits(logits, temp);
            (key.clone(), token)
        })
        .collect()
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_batch_single_slot() {
        let dirty = vec![("sess-1".into(), 0usize, vec![101, 204, 307])];
        let req = assemble_batch(&dirty).expect("should produce batch");
        assert_eq!(req.slot_order.len(), 1);
        assert_eq!(req.token_batches[0], vec![101, 204, 307]);
        assert_eq!(req.slot_order[0], ("sess-1".into(), 0));
    }

    #[test]
    fn test_assemble_batch_multi_slot_with_padding() {
        let dirty = vec![
            ("sess-1".into(), 0, vec![1, 2]),           // len 2
            ("sess-1".into(), 1, vec![10, 20, 30, 40]), // len 4
            ("sess-2".into(), 0, vec![100]),             // len 1
        ];
        let req = assemble_batch(&dirty).expect("should produce batch");
        assert_eq!(req.slot_order.len(), 3);
        // 全部 padding 到 max_len = 4
        assert_eq!(req.token_batches[0], vec![1, 2, 0, 0]);
        assert_eq!(req.token_batches[1], vec![10, 20, 30, 40]);
        assert_eq!(req.token_batches[2], vec![100, 0, 0, 0]);
    }

    #[test]
    fn test_assemble_batch_empty_returns_none() {
        let dirty: Vec<(String, usize, Vec<u32>)> = vec![];
        assert!(assemble_batch(&dirty).is_none());
    }

    #[test]
    fn test_assemble_batch_all_empty_buffers() {
        let dirty = vec![("sess-1".into(), 0, vec![])];
        assert!(assemble_batch(&dirty).is_none());
    }

    #[test]
    fn test_sample_logits_deterministic_high_temp() {
        // 高温度 → 接近均匀分布；只验证不 panic 且 token 在范围内
        let logits = vec![0.1f32, 0.2, 0.7, 0.0];
        for _ in 0..20 {
            let token = sample_logits(&logits, 100.0);
            assert!(token < 4, "token out of range: {}", token);
        }
    }

    #[test]
    fn test_sample_logits_low_temp_modes() {
        // 很低温度 → 几乎总是选最大 logit
        let logits = vec![0.1f32, 0.2, 5.0, 0.0];
        let mut count = 0;
        for _ in 0..50 {
            if sample_logits(&logits, 0.01) == 2 {
                count += 1;
            }
        }
        // 极低温度下几乎总是选 index 2
        assert!(count > 40, "low temp should strongly prefer max logit");
    }

    #[test]
    fn test_sample_logits_empty_returns_zero() {
        assert_eq!(sample_logits(&[], 1.0), 0);
    }

    #[test]
    fn test_sample_batch() {
        let result = BatchResult {
            logits_batches: vec![
                vec![0.1, 0.9, 0.0],  // slot 0
                vec![0.5, 0.1, 0.4],  // slot 1
            ],
            slot_order: vec![("sess-1".into(), 0), ("sess-1".into(), 1)],
        };

        let mut temps = HashMap::new();
        temps.insert(("sess-1".into(), 0), 0.01); // 几乎确定选 index 1
        temps.insert(("sess-1".into(), 1), 0.01); // 几乎确定选 index 0

        let tokens = sample_batch(&result, &temps);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].0, ("sess-1".into(), 0));
        assert_eq!(tokens[1].0, ("sess-1".into(), 1));
    }
}
