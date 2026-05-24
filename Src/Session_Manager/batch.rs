//Presented by KeJi
//Date ： 2026-05-22

//! Batch 协议 — Session Manager ↔ ML Thread 接口类型

use std::collections::HashMap;

// ─── Batch 类型 ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BatchRequest {
    pub token_batches: Vec<Vec<u32>>,
    pub slot_order: Vec<(u64, usize)>,
}

#[derive(Debug, Clone)]
pub struct BatchResult {
    pub logits_batches: Vec<Vec<f32>>,
    pub slot_order: Vec<(u64, usize)>,
}

// ─── Batch 组装 ─────────────────────────────────────────────

pub fn assemble_batch(
    dirty_slots: &[(u64, usize, Vec<u32>)],
) -> Option<BatchRequest> {
    if dirty_slots.is_empty() {
        return None;
    }

    let max_len = dirty_slots.iter().map(|(_, _, buf)| buf.len()).max().unwrap_or(0);
    if max_len == 0 {
        return None;
    }

    let mut token_batches = Vec::with_capacity(dirty_slots.len());
    let mut slot_order = Vec::with_capacity(dirty_slots.len());

    for (sess_id, slot_id, buf) in dirty_slots {
        let mut padded = buf.clone();
        padded.resize(max_len, 0);
        token_batches.push(padded);
        slot_order.push((*sess_id, *slot_id));
    }

    Some(BatchRequest { token_batches, slot_order })
}

// ─── Sample 逻辑 ────────────────────────────────────────────

pub fn sample_logits(logits: &[f32], temperature: f64) -> u32 {
    if logits.is_empty() {
        return 0;
    }

    let t = temperature.max(1e-6) as f32;

    let scaled: Vec<f32> = logits.iter().map(|&x| x / t).collect();

    let max_val = scaled.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum: Vec<f32> = scaled.iter().map(|&x| (x - max_val).exp()).collect();
    let sum: f32 = exp_sum.iter().sum();

    if sum == 0.0 {
        return 0;
    }

    let probs: Vec<f32> = exp_sum.iter().map(|&x| x / sum).collect();

    let r: f32 = fast_random();
    let mut cumulative = 0.0f32;
    for (i, &p) in probs.iter().enumerate() {
        cumulative += p;
        if r < cumulative {
            return i as u32;
        }
    }

    (probs.len() - 1) as u32
}

fn fast_random() -> f32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos as f32) / 1_000_000_000.0
}

// ─── 分发结果 ───────────────────────────────────────────────

pub fn sample_batch(
    result: &BatchResult,
    temperatures: &HashMap<(u64, usize), f64>,
) -> Vec<((u64, usize), u32)> {
    result
        .logits_batches
        .iter()
        .zip(result.slot_order.iter())
        .map(|(logits, key)| {
            let temp = temperatures.get(key).copied().unwrap_or(0.8);
            let token = sample_logits(logits, temp);
            (*key, token)
        })
        .collect()
}

// ─── 内联测试 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_batch_single_slot() {
        let dirty = vec![(1u64, 0usize, vec![101, 204, 307])];
        let req = assemble_batch(&dirty).expect("should produce batch");
        assert_eq!(req.slot_order.len(), 1);
        assert_eq!(req.token_batches[0], vec![101, 204, 307]);
        assert_eq!(req.slot_order[0], (1, 0));
    }

    #[test]
    fn test_assemble_batch_multi_slot_with_padding() {
        let dirty = vec![
            (1, 0, vec![1, 2]),
            (1, 1, vec![10, 20, 30, 40]),
            (2, 0, vec![100]),
        ];
        let req = assemble_batch(&dirty).expect("should produce batch");
        assert_eq!(req.slot_order.len(), 3);
        assert_eq!(req.token_batches[0], vec![1, 2, 0, 0]);
        assert_eq!(req.token_batches[1], vec![10, 20, 30, 40]);
        assert_eq!(req.token_batches[2], vec![100, 0, 0, 0]);
    }

    #[test]
    fn test_assemble_batch_empty_returns_none() {
        let dirty: Vec<(u64, usize, Vec<u32>)> = vec![];
        assert!(assemble_batch(&dirty).is_none());
    }

    #[test]
    fn test_assemble_batch_all_empty_buffers() {
        let dirty = vec![(1, 0, vec![])];
        assert!(assemble_batch(&dirty).is_none());
    }

    #[test]
    fn test_sample_logits_deterministic_high_temp() {
        let logits = vec![0.1f32, 0.2, 0.7, 0.0];
        for _ in 0..20 {
            let token = sample_logits(&logits, 100.0);
            assert!(token < 4, "token out of range: {}", token);
        }
    }

    #[test]
    fn test_sample_logits_low_temp_modes() {
        let logits = vec![0.1f32, 0.2, 5.0, 0.0];
        let mut count = 0;
        for _ in 0..50 {
            if sample_logits(&logits, 0.01) == 2 {
                count += 1;
            }
        }
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
                vec![0.1, 0.9, 0.0],
                vec![0.5, 0.1, 0.4],
            ],
            slot_order: vec![(1, 0), (1, 1)],
        };

        let mut temps = HashMap::new();
        temps.insert((1, 0), 0.01);
        temps.insert((1, 1), 0.01);

        let tokens = sample_batch(&result, &temps);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].0, (1, 0));
        assert_eq!(tokens[1].0, (1, 1));
    }
}
