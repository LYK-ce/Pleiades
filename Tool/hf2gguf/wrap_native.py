# wrap_native — Safetensors 分片原样打包进 PGGUF（--wrap-native 模式）
#
# Presented by KeJi
# Date: 2026-06-01
#
# 职责：
#   1. 读取 config.json → GGUF metadata keys（架构参数、层信息等）
#   2. 读取 tokenizer.json + tokenizer_config.json → GGUF tokenizer metadata
#   3. 读取 model.safetensors.index.json → 获取分片列表
#   4. 将每个 safetensors 分片作为 GGUF tensor blob 写入
#      - tensor 名：safetensors/shard-{N}（N 从 0 开始）
#      - 内容：分片文件的原始字节（不做精度转换）
#   5. 写入 pleiades.weight_format = "safetensors" 标记
#   6. 写入 pleiades.model_id + pleiades.layer_bitmap

import json
from pathlib import Path
from typing import Any

import numpy as np
import xxhash
from gguf import GGUFWriter

from .config_reader import read_config, identify_architecture
from .tokenizer_writer import write_tokenizer
from .pgguf_writer import build_layer_bitmap, bitmap_to_hex
from .utils import has_sharded_tensors


def _read_index_json(model_dir: Path) -> dict[str, Any]:
    """读取 model.safetensors.index.json，返回原始 JSON 字典。"""
    index_path = model_dir / "model.safetensors.index.json"
    if not index_path.exists():
        raise FileNotFoundError(f"model.safetensors.index.json not found in {model_dir}")
    with open(index_path, "r", encoding="utf-8") as f:
        return json.load(f)


def _get_shard_files(index: dict[str, Any]) -> list[str]:
    """从 index.json 中提取去重排序后的分片文件名列表。"""
    weight_map: dict[str, str] = index.get("weight_map", {})
    shard_set: set[str] = set(weight_map.values())
    return sorted(shard_set)


def _write_dsv4_metadata(
    writer: GGUFWriter,
    config: dict[str, Any],
    num_shards: int,
) -> int:
    """写入 DeepSeek V4 架构参数到 GGUF metadata。

    V4 不使用 standard GGUF block_count/embedding_length 等 key，
    而是将完整的 config.json 序列化存储，并写入一个简化的架构标记。
    返回 num_layers（用于 layer_bitmap）。
    """
    # 基础标记
    writer.add_string("general.architecture", "deepseek_v4")
    writer.add_string("pleiades.weight_format", "safetensors")
    writer.add_uint32("pleiades.num_shards", num_shards)

    # 核心架构参数（从 config.json 提取）
    num_layers = config.get("num_hidden_layers", config.get("n_layers", 0))
    hidden_size = config.get("hidden_size", config.get("dim", 0))
    vocab_size = config.get("vocab_size", 0)
    num_heads = config.get("num_attention_heads", config.get("n_heads", 0))
    head_dim = config.get("head_dim", 0)
    moe_inter_dim = config.get("moe_inter_dim", config.get("intermediate_size", 0))
    n_routed_experts = config.get("n_routed_experts", 0)
    n_shared_experts = config.get("n_shared_experts", 0)
    n_activated_experts = config.get("n_activated_experts", 0)
    q_lora_rank = config.get("q_lora_rank", 0)
    rope_head_dim = config.get("rope_head_dim", config.get("kv_lora_rank", 0))
    hc_mult = config.get("hc_mult", 0)
    window_size = config.get("window_size", 0)
    max_seq_len = config.get("max_seq_len", config.get("max_position_embeddings", 4096))
    rms_norm_eps = config.get("rms_norm_eps", config.get("norm_eps", 1e-6))
    rope_theta = config.get("rope_theta", 10000.0)
    rope_factor = config.get("rope_factor", 1.0)
    beta_fast = config.get("beta_fast", 32.0)
    beta_slow = config.get("beta_slow", 1.0)
    compress_rope_theta = config.get("compress_rope_theta", rope_theta)
    original_seq_len = config.get("original_seq_len", 0)

    # Indexer 参数（CSA layers）
    index_n_heads = config.get("index_n_heads", 0)
    index_head_dim = config.get("index_head_dim", 0)
    index_topk = config.get("index_topk", 0)

    # mHC 参数
    hc_sinkhorn_iters = config.get("hc_sinkhorn_iters", 0)
    hc_eps = config.get("hc_eps", 1e-6)

    # 量化参数（标记用）
    dtype = config.get("dtype", "")
    scale_fmt = config.get("scale_fmt", "")
    expert_dtype = config.get("expert_dtype", "")

    # Score function
    score_func = config.get("score_func", "sqrtsoftplus")
    swiglu_limit = config.get("swiglu_limit", 0.0)
    route_scale = config.get("route_scale", 1.0)

    # Compress ratios（per-layer）
    compress_ratios = config.get("compress_ratios", [])

    # n_hash_layers（hash routing 层数）
    n_hash_layers = config.get("n_hash_layers", 0)

    writer.add_uint32("deepseek_v4.block_count", num_layers)
    writer.add_uint32("deepseek_v4.embedding_length", hidden_size)
    writer.add_uint32("deepseek_v4.vocab_size", vocab_size)
    writer.add_uint32("deepseek_v4.attention.head_count", num_heads)
    writer.add_uint32("deepseek_v4.attention.key_length", head_dim)
    writer.add_uint32("deepseek_v4.feed_forward_length", moe_inter_dim)
    writer.add_uint32("deepseek_v4.context_length", max_seq_len)
    writer.add_float32("deepseek_v4.attention.layer_norm_rms_epsilon", rms_norm_eps)
    writer.add_float32("deepseek_v4.rope.freq_base", rope_theta)

    # V4 特有参数
    writer.add_uint32("deepseek_v4.n_routed_experts", n_routed_experts)
    writer.add_uint32("deepseek_v4.n_shared_experts", n_shared_experts)
    writer.add_uint32("deepseek_v4.n_activated_experts", n_activated_experts)
    writer.add_uint32("deepseek_v4.q_lora_rank", q_lora_rank)
    writer.add_uint32("deepseek_v4.rope_head_dim", rope_head_dim)
    writer.add_uint32("deepseek_v4.hc_mult", hc_mult)
    writer.add_uint32("deepseek_v4.window_size", window_size)
    writer.add_float32("deepseek_v4.rope_factor", rope_factor)
    writer.add_float32("deepseek_v4.beta_fast", beta_fast)
    writer.add_float32("deepseek_v4.beta_slow", beta_slow)
    writer.add_float32("deepseek_v4.compress_rope_theta", compress_rope_theta)
    writer.add_uint32("deepseek_v4.original_seq_len", original_seq_len)
    writer.add_uint32("deepseek_v4.index_n_heads", index_n_heads)
    writer.add_uint32("deepseek_v4.index_head_dim", index_head_dim)
    writer.add_uint32("deepseek_v4.index_topk", index_topk)
    writer.add_uint32("deepseek_v4.hc_sinkhorn_iters", hc_sinkhorn_iters)
    writer.add_float32("deepseek_v4.hc_eps", hc_eps)
    writer.add_uint32("deepseek_v4.o_groups", config.get("o_groups", 1))
    writer.add_uint32("deepseek_v4.o_lora_rank", config.get("o_lora_rank", 0))
    writer.add_string("deepseek_v4.score_func", score_func)
    writer.add_float32("deepseek_v4.swiglu_limit", swiglu_limit)
    writer.add_float32("deepseek_v4.route_scale", route_scale)
    writer.add_uint32("deepseek_v4.n_hash_layers", n_hash_layers)

    if dtype:
        writer.add_string("deepseek_v4.dtype", dtype)
    if scale_fmt:
        writer.add_string("deepseek_v4.scale_fmt", scale_fmt)
    if expert_dtype:
        writer.add_string("deepseek_v4.expert_dtype", expert_dtype)

    if compress_ratios:
        # 序列化为逗号分隔的字符串（GGUF 不支持 int array）
        cr_str = ",".join(str(r) for r in compress_ratios)
        writer.add_string("deepseek_v4.compress_ratios", cr_str)

    print(f"  blocks={num_layers}, hidden={hidden_size}, heads={num_heads}")
    print(f"  head_dim={head_dim}, moe={n_routed_experts}/{n_shared_experts}/{n_activated_experts}")
    print(f"  hc_mult={hc_mult}, q_lora_rank={q_lora_rank}, rope_head_dim={rope_head_dim}")
    print(f"  window={window_size}, ctx={max_seq_len}, eps={rms_norm_eps}")
    print(f"  score_func={score_func}, dtype={dtype}, expert_dtype={expert_dtype}")

    return num_layers


def _write_shard_tensors(
    writer: GGUFWriter,
    model_dir: Path,
    shard_files: list[str],
) -> int:
    """将每个 safetensors 分片作为 GGUF tensor blob 写入。

    每个分片的 tensor 名称为 safetensors/shard-{N}（N 从 0 开始），
    内容为该分片文件的原始字节（不做精度转换）。
    """
    count = 0
    for i, shard_file in enumerate(shard_files):
        shard_path = model_dir / shard_file
        if not shard_path.exists():
            raise FileNotFoundError(f"Shard file not found: {shard_path}")

        with open(shard_path, "rb") as f:
            raw_bytes = f.read()

        tensor_name = f"safetensors/shard-{i}"
        # 将原始字节包装为 uint8 numpy array，形状为 [len(raw_bytes)]
        arr = np.frombuffer(raw_bytes, dtype=np.uint8)
        writer.add_tensor(tensor_name, arr)
        count += 1

        size_mb = len(raw_bytes) / (1024 * 1024)
        print(f"    {tensor_name} ← {shard_file}  [{size_mb:.1f} MB]")

    return count


def wrap_native_to_pgguf(
    model_dir: Path,
    output_path: Path,
) -> Path:
    """将 DeepSeek V4 Flash 模型的 safetensors 分片原样打包到 PGGUF。

    Args:
        model_dir: HF 模型目录（含 config.json, *.safetensors, tokenizer.json 等）
        output_path: 输出的 .pgguf 文件路径

    Returns:
        Path to the generated .pgguf file.
    """
    model_dir = model_dir.resolve()
    output_path = output_path.resolve()
    print(f"\n=== wrap-native mode ===\n  model_dir: {model_dir}\n  output: {output_path}")

    # 1. 读取 config.json
    print(f"\n[1/5] Reading config.json…")
    config = read_config(model_dir)
    arch = identify_architecture(config)
    print(f"  detected arch: {arch}")

    # 2. 读取 index.json，获取分片列表
    print(f"\n[2/5] Reading model.safetensors.index.json…")
    if not has_sharded_tensors(model_dir):
        raise ValueError(
            "wrap-native mode requires a sharded model (model.safetensors.index.json). "
            "The model must have multiple safetensors shard files."
        )
    index = _read_index_json(model_dir)
    shard_files = _get_shard_files(index)
    print(f"  found {len(shard_files)} shard files")

    # 3. 初始化 GGUFWriter
    print(f"\n[3/5] Initializing GGUF writer…")
    writer = GGUFWriter(str(output_path), "deepseek_v4")

    # 4. 写入 GGUF metadata + PGGUF metadata
    print(f"\n[4/5] Writing metadata…")
    num_layers = _write_dsv4_metadata(writer, config, len(shard_files))

    # Tokenizer metadata
    print(f"  writing tokenizer metadata…")
    write_tokenizer(writer, model_dir)

    # PGGUF metadata
    model_id = _compute_model_id_for_shards(model_dir, shard_files)
    layer_bitmap = build_layer_bitmap(num_layers)
    writer.add_uint32("pleiades.model_id", model_id)
    writer.add_string("pleiades.layer_bitmap", bitmap_to_hex(layer_bitmap))
    print(f"  [pgguf] model_id={model_id:08x}, layers={num_layers + 2}")

    # 5. 添加 tensor blobs（分片文件原样）
    print(f"\n[5/5] Adding shard tensors…")
    shard_count = _write_shard_tensors(writer, model_dir, shard_files)
    print(f"  added {shard_count} shard tensors")

    # 6. 写入 GGUF 文件
    print(f"\nWriting GGUF file…")
    writer.open_output_file()
    writer.write_header_to_file()
    writer.write_kv_data_to_file()
    writer.write_tensors_to_file()
    writer.close()
    del writer

    # 验证输出
    output_size = output_path.stat().st_size
    print(f"\nDone: {output_path} ({output_size / (1024**3):.1f} GB)")

    return output_path


def _compute_model_id_for_shards(model_dir: Path, shard_files: list[str]) -> int:
    """根据分片文件名和大小计算 xxhash32 model_id。"""
    h = xxhash.xxh32()
    for shard_file in shard_files:
        h.update(shard_file.encode())
        shard_path = model_dir / shard_file
        file_size = shard_path.stat().st_size
        h.update(str(file_size).encode())
    return h.intdigest()
