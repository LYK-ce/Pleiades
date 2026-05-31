// Presented by KeJi
// Date: 2026-05-31

//! Task 13 — HF safetensors → PGGUF 转换验证
//!
//! 验证 Python 转换器生成的 PGGUF 文件能被 Rust 侧完整加载和推理。
//!
//! ## 前置条件
//!
//! 在项目根目录下准备好 PGGUF 文件：
//! ```bash
//! cp test_models/Qwen3-0.6B.pgguf .
//! ```
//!
//! ## 运行
//!
//! ```bash
//! cargo test t13_hf2gguf_validation -- --nocapture
//! ```

#![allow(non_snake_case)]

use std::path::PathBuf;
use pleiades::MlSession;

/// 查找测试模型路径
fn test_model_path() -> PathBuf {
    let candidates = [
        "test_models/Qwen3-0.6B.pgguf",
        "/workspace/test_models/Qwen3-0.6B.pgguf",
        "Qwen3-0.6B.pgguf",
    ];
    for p in &candidates {
        let path = std::path::Path::new(p);
        if path.exists() {
            return path.to_path_buf();
        }
    }
    panic!(
        "Test model not found. Run: cp test_models/Qwen3-0.6B.pgguf ."
    );
}

/// L1: 格式可读 — GGUF_Analyze 解析 metadata
#[test]
fn t13_l1_gguf_analyze() {
    let path = test_model_path();
    println!("=== L1: GGUF_Analyze ===");
    println!("File: {}", path.display());

    let arch = pleiades::GGUF_Analyze(&path).expect("GGUF_Analyze failed");

    println!("  architecture:    {}", arch.architecture);
    println!("  num_layers:      {}", arch.num_layers);
    println!("  embedding_length: {}", arch.embedding_length);
    println!("  head_count:      {} / {}", arch.head_count, arch.head_count_kv);
    println!("  head_dim:        {}", arch.head_dim);
    println!("  context_length:  {}", arch.context_length);
    println!("  vocab_size:      {}", arch.vocab_size);
    println!("  eos_token_id:    {}", arch.eos_token_id);
    println!("  rms_norm_eps:    {}", arch.rms_norm_eps);
    println!("  rope_freq_base:  {}", arch.rope_freq_base);
    println!("  model_id:        {:?}", arch.model_id);
    println!("  layer_bitmap:    {:?}", arch.layer_bitmap.as_ref().map(|b| hex::encode(b)));
    println!("  is_split:        {}", arch.is_split);

    assert_eq!(arch.architecture.to_lowercase(), "qwen3");
    assert_eq!(arch.num_layers, 28);
    // model_id/layer_bitmap only set by GGUF_Analyze_And_Convert, not GGUF_Analyze
    println!("  model_id:        {:?} (None is ok for GGUF_Analyze)", arch.model_id);
    println!("  layer_bitmap:    {:?} (None is ok for GGUF_Analyze)", arch.layer_bitmap.as_ref().map(|b| hex::encode(b)));

    println!("✅ L1 PASSED");
}

/// L2: 权重可加载 — GGUF_Load_Model 完整加载
#[test]
fn t13_l2_load_model() {
    let path = test_model_path();
    let device = candle_core::Device::Cpu;

    println!("=== L2: GGUF_Load_Model ===");
    println!("File: {}", path.display());

    let model = pleiades::GGUF_Load_Model(0, 999999, &path, &device)
        .expect("GGUF_Load_Model failed");

    println!("  has_input_head:  {}", model.has_input_head);
    println!("  has_output_head: {}", model.has_output_head);
    println!("  arch_info.architecture: {}", model.arch_info.architecture);

    assert!(model.has_input_head, "Model should have embedding layer");
    assert!(model.has_output_head, "Model should have output layer");

    // 释放模型
    pleiades::GGUF_Unload_Model(model);
    println!("✅ L2 PASSED");
}

/// L3: 端到端推理 — 输入"你好"，打印完整输出
#[test]
fn t13_l3_inference() {
    let path = test_model_path();

    println!("=== L3: End-to-End Inference ===");
    println!("Model: {}", path.display());
    println!();

    // 1. 创建 session
    let mut sess = MlSession::new("cpu").expect("MlSession::new failed");

    // 2. 加载模型
    println!("[1/4] Loading model…");
    sess.load_model(&path, 0, 999999)
        .expect("load_model failed");
    println!("  ✅ model loaded");

    // 3. 加载 tokenizer
    println!("[2/4] Loading tokenizer…");
    sess.load_tokenizer(&path)
        .expect("load_tokenizer failed");
    println!("  ✅ tokenizer loaded");

    // 4. 编码输入 — 使用 chat_template (enable_thinking=false 禁用思考)
    let input_text = "你好";
    let messages = vec![pleiades::Message {
        role: "user".to_string(),
        content: input_text.to_string(),
    }];
    let token_ids = sess.encode_messages(&messages).expect("encode_messages failed");
    println!("[3/4] Encoding: \"{}\" → {} tokens (chat_template)", input_text, token_ids.len());

    let input_tensor = sess.tensorize(&token_ids).expect("tensorize failed");

    // 4. Prefill
    println!("[3/4] Prefill…");
    let mut logits = sess.forward(&input_tensor, Some(0)).expect("forward failed");

    // 5. 自回归生成
    println!("[4/4] Generating…");
    let eos = sess.get_eos();
    let max_tokens = 128;
    let mut output_tokens: Vec<u32> = Vec::new();
    let mut output_text = String::new();

    print!("  Output: ");

    for i in 0..max_tokens {
        let token_id = sess.sample(&logits, 0.8).expect("sample failed");

        if token_id == eos {
            println!(" <EOS>");
            break;
        }

        output_tokens.push(token_id);
        let decoded = sess.decode(token_id).unwrap_or_else(|_| "�".to_string());
        // Qwen3 thinking mode: collect raw output, strip <think>... blocks at end
        output_text.push_str(&decoded);
        // Print only the non-thinking part
        let clean = strip_thinking(&output_text);
        if clean.len() > printed_len {
            print!("{}", &clean[printed_len..]);
            printed_len = clean.len();
        }

        let next_input = sess.tensorize(&[token_id]).expect("tensorize next failed");
        logits = sess.forward(&next_input, None).expect("forward next failed");

        if i == max_tokens - 1 {
            println!(" <MAX_TOKENS>");
        }
    }

    println!();
    println!("────────────────────────────────────────");
    println!("Total tokens generated: {}", output_tokens.len());
    println!("Token IDs: {:?}", output_tokens);
    println!();
    println!("✅ L3 PASSED");
}
