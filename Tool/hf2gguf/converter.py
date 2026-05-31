# converter — Main hf2gguf pipeline orchestration
#
# Presented by KeJi
# Date: 2026-05-31

import hashlib
from pathlib import Path
from typing import Any

import numpy as np
import torch
import xxhash
from gguf import GGUFWriter, GGMLQuantizationType

from .config_reader import read_config, identify_architecture
from .tokenizer_writer import write_tokenizer
from .pgguf_writer import build_layer_bitmap, bitmap_to_hex
from .utils import resolve_tensor_source
from .mappings.qwen3 import (
    QWEN3_TENSOR_MAP,
    QWEN3_CONFIG_TO_GGUF,
    QWEN3_GGUF_KEY_HEAD_DIM,
)


def _compute_model_id_for_tensors(hf_tensors: dict[str, Any]) -> int:
    """Compute xxhash32 model_id from sorted tensor names and shapes.

    Mirrors the Rust logic: deterministic hash from tensor metadata.
    """
    h = xxhash.xxh32()
    for name in sorted(hf_tensors.keys()):
        t = hf_tensors[name]
        h.update(name.encode())
        h.update(str(t.shape).encode())
    return h.intdigest()


def convert_hf_to_pgguf(
    model_dir: Path,
    output_path: Path | None = None,
    *,
    arch_override: str | None = None,
) -> Path:
    """Convert a HuggingFace safetensors model directory to PGGUF.

    Args:
        model_dir: Path to HF model directory (with config.json, *.safetensors, tokenizer.json)
        output_path: Output .pgguf path. Defaults to {model_dir.name}.pgguf
        arch_override: Force architecture name (skip auto-detection)

    Returns:
        Path to the generated .pgguf file.
    """
    model_dir = model_dir.resolve()

    # 1. Read config
    print(f"[1/6] Reading config.json…")
    config = read_config(model_dir)
    arch = arch_override or identify_architecture(config)
    print(f"  architecture: {arch}")

    if output_path is None:
        output_path = model_dir.parent / f"{model_dir.name}.pgguf"
    output_path = output_path.resolve()
    print(f"  output: {output_path}")

    # 2. Set up GGUFWriter
    print(f"[2/6] Initializing GGUF writer…")
    gguf_arch = _hf_arch_to_gguf_arch(arch)
    writer = GGUFWriter(str(output_path), gguf_arch)

    # 3. Write GGUF metadata
    print(f"[3/6] Writing GGUF metadata…")
    num_layers = _write_gguf_metadata(writer, config, arch)

    # 4. Write tokenizer (FLOAT64 already removed for shimmytok compatibility)
    print(f"[4/6] Writing tokenizer…")
    write_tokenizer(writer, model_dir)

    # 5. Load safetensors (do this before PGGUF metadata so we can hash)
    print(f"[5/6] Loading safetensors…")
    hf_tensors = resolve_tensor_source(model_dir)
    print(f"  loaded {len(hf_tensors)} tensors")

    # 6. Add PGGUF metadata (before tensors so model_id is deterministic)
    model_id = _compute_model_id_for_tensors(hf_tensors)
    layer_bitmap = build_layer_bitmap(num_layers)
    writer.add_uint32("pleiades.model_id", model_id)
    writer.add_string("pleiades.layer_bitmap", bitmap_to_hex(layer_bitmap))
    print(f"  [pgguf] model_id={model_id:08x}, layers={num_layers + 2}")

    # 7. Add tensors to writer (must be in NO_FILE state before write_* calls)
    print(f"[6/6] Adding & writing tensors…")
    mapped_count = _add_tensors(writer, hf_tensors, arch, num_layers)
    print(f"  added {mapped_count} tensors")

    # Write GGUF file: open → header → KV → (TI + tensors via write_tensors_to_file) → close
    writer.open_output_file()
    writer.write_header_to_file()
    writer.write_kv_data_to_file()
    writer.write_tensors_to_file()  # internally calls write_ti_data_to_file
    writer.close()
    del writer

    print(f"\nDone: {output_path}")
    return output_path


def _hf_arch_to_gguf_arch(arch: str) -> str:
    """Map internal arch name to GGUF general.architecture value."""
    mapping = {
        "qwen3": "qwen3",
        "qwen3moe": "qwen3moe",
        "deepseek_v3": "deepseek2",
        "deepseek2": "deepseek2",
    }
    return mapping.get(arch, arch)


def _write_gguf_metadata(
    writer: GGUFWriter,
    config: dict[str, Any],
    arch: str,
) -> int:
    """Write architecture metadata to GGUF and return num_layers."""
    prefix = _hf_arch_to_gguf_arch(arch)

    num_layers = config.get("num_hidden_layers", 0)
    hidden_size = config.get("hidden_size", 0)
    num_heads = config.get("num_attention_heads", 0)
    num_kv_heads = config.get("num_key_value_heads", num_heads)
    head_dim_val = config.get("head_dim", hidden_size // num_heads if num_heads else 0)
    intermediate_size = config.get("intermediate_size", 0)
    max_pos = config.get("max_position_embeddings", 2048)
    rms_eps = config.get("rms_norm_eps", 1e-6)
    rope_theta = config.get("rope_theta", 10000.0)
    vocab_size = config.get("vocab_size", 0)

    writer.add_uint32(f"{prefix}.block_count", num_layers)
    writer.add_uint32(f"{prefix}.embedding_length", hidden_size)
    writer.add_uint32(f"{prefix}.attention.head_count", num_heads)
    writer.add_uint32(f"{prefix}.attention.head_count_kv", num_kv_heads)
    writer.add_uint32(f"{prefix}.attention.key_length", head_dim_val)
    writer.add_uint32(f"{prefix}.feed_forward_length", intermediate_size)
    writer.add_uint32(f"{prefix}.context_length", max_pos)
    writer.add_float32(f"{prefix}.attention.layer_norm_rms_epsilon", rms_eps)
    writer.add_float32(f"{prefix}.rope.freq_base", rope_theta)
    writer.add_uint32(f"{prefix}.vocab_size", vocab_size)

    print(f"  blocks={num_layers}, hidden={hidden_size}, heads={num_heads}/{num_kv_heads}")
    print(f"  head_dim={head_dim_val}, ffn={intermediate_size}, ctx={max_pos}")
    print(f"  eps={rms_eps}, rope={rope_theta}, vocab={vocab_size}")

    return num_layers


def _add_tensors(
    writer: GGUFWriter,
    hf_tensors: dict[str, Any],
    arch: str,
    num_layers: int,
) -> int:
    """Map HF tensor names to GGUF names and add to GGUFWriter."""
    tensor_map = _get_tensor_map(arch)
    count = 0

    for hf_name, tensor in hf_tensors.items():
        raw_dtype = None
        if hasattr(tensor, "numpy"):
            dtype_str = str(tensor.dtype) if hasattr(tensor, "dtype") else ""
            if "bfloat16" in dtype_str:
                # Preserve BF16: view as uint16 bytes + pass raw_dtype=BF16
                arr = tensor.view(torch.int16).numpy().view(np.uint16)
                raw_dtype = GGMLQuantizationType.BF16
            else:
                arr = tensor.numpy()
        elif hasattr(tensor, "detach"):
            arr = tensor.detach().cpu().numpy()
        else:
            arr = np.asarray(tensor)

        gguf_name = tensor_map.get(hf_name)
        if gguf_name is None:
            gguf_name = _match_layered_tensor(hf_name, tensor_map, num_layers)
        if gguf_name is None:
            continue

        writer.add_tensor(gguf_name, arr, raw_dtype=raw_dtype)
        count += 1

        if count <= 3 or count % 10 == 0:
            print(f"    {hf_name} → {gguf_name}  [{arr.dtype}]")

    return count


def _match_layered_tensor(
    hf_name: str,
    tensor_map: dict[str, str],
    num_layers: int,
) -> str | None:
    """Match a layered tensor name by substituting {n}."""
    import re

    for pattern, gguf_pattern in tensor_map.items():
        if "{n}" not in pattern:
            continue
        regex = "^" + re.escape(pattern).replace(r"\{n\}", r"(\d+)") + "$"
        m = re.match(regex, hf_name)
        if m:
            layer_idx = int(m.group(1))
            return gguf_pattern.replace("{n}", str(layer_idx))

    return None


def _get_tensor_map(arch: str) -> dict[str, str]:
    """Get the tensor name mapping dict for a given architecture."""
    if arch in ("qwen3",):
        return QWEN3_TENSOR_MAP
    return {}
