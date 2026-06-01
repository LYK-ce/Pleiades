#!/usr/bin/env python3
# validate_pgguf.py — 校验 --wrap-native 模式生成的 PGGUF 文件
#
# Presented by KeJi
# Date: 2026-06-01
#
# Usage:
#   python Tool/validate_pgguf.py <model.pgguf>
#
# 检查项:
#   1. GGUF header 合法性 (magic, version)
#   2. Metadata: architecture + weight_format + num_shards
#   3. deepseek_v4.* 架构参数完整性 (与 Rust Config::from_gguf_metadata 对齐)
#   4. safetensors/shard-* tensor 数量 = num_shards
#   5. 每个 shard 是合法的 safetensors 格式
#   6. 分片间 tensor 名不重叠
#   7. 文件大小合理性

import json
import struct
import sys
from collections import Counter
from pathlib import Path
from typing import Any

# ── GGUF 常量 ──
GGUF_MAGIC = b"GGUF"
GGUF_VERSION = 3

# GGUF Value 类型标记
GGUF_TYPE_UINT8 = 0
GGUF_TYPE_INT8 = 1
GGUF_TYPE_UINT16 = 2
GGUF_TYPE_INT16 = 3
GGUF_TYPE_UINT32 = 4
GGUF_TYPE_INT32 = 5
GGUF_TYPE_FLOAT32 = 6
GGUF_TYPE_BOOL = 7
GGUF_TYPE_STRING = 8
GGUF_TYPE_ARRAY = 9
GGUF_TYPE_UINT64 = 10
GGUF_TYPE_INT64 = 11
GGUF_TYPE_FLOAT64 = 12

# ── 必须存在的 metadata key ──
REQUIRED_METADATA_KEYS = [
    "general.architecture",
    "pleiades.weight_format",
    "pleiades.num_shards",
    "pleiades.model_id",
    "pleiades.layer_bitmap",
    "deepseek_v4.block_count",
    "deepseek_v4.embedding_length",
    "deepseek_v4.vocab_size",
    "deepseek_v4.attention.head_count",
    "deepseek_v4.attention.key_length",
    "deepseek_v4.q_lora_rank",
    "deepseek_v4.rope_head_dim",
    "deepseek_v4.hc_mult",
    "deepseek_v4.score_func",
    "deepseek_v4.o_groups",
    "deepseek_v4.o_lora_rank",
    "deepseek_v4.context_length",
    "deepseek_v4.attention.layer_norm_rms_epsilon",
    "deepseek_v4.rope.freq_base",
    "deepseek_v4.compress_ratios",
]

# ── Tokenizer 相关 key（至少需要以下核心字段）──
TOKENIZER_KEYS = {
    "tokenizer.ggml.model":       ("string", "tokenizer model type (e.g. 'gpt2', 'llama')"),
    "tokenizer.ggml.tokens":      ("array",  "vocab token list"),
    "tokenizer.chat_template":    ("string", "Jinja2 chat template"),
}

# ── 值范围约束 ──
RANGES = {
    "deepseek_v4.block_count": (1, 200),
    "deepseek_v4.embedding_length": (256, 32768),
    "deepseek_v4.vocab_size": (1000, 200000),
    "deepseek_v4.attention.head_count": (1, 256),
    "deepseek_v4.attention.key_length": (32, 1024),
    "deepseek_v4.q_lora_rank": (32, 4096),
    "deepseek_v4.rope_head_dim": (16, 1024),
    "deepseek_v4.hc_mult": (1, 16),
    "deepseek_v4.o_groups": (1, 32),
    "deepseek_v4.o_lora_rank": (0, 4096),
    "deepseek_v4.context_length": (256, 131072),
}


def main() -> None:
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <model.pgguf>", file=sys.stderr)
        sys.exit(1)

    pgguf_path = Path(sys.argv[1]).resolve()
    if not pgguf_path.exists():
        print(f"ERROR: file not found: {pgguf_path}", file=sys.stderr)
        sys.exit(1)

    errors: list[str] = []
    warnings: list[str] = []

    with open(pgguf_path, "rb") as f:
        data = f.read()

    file_size = len(data)
    print(f"File: {pgguf_path.name}  ({file_size / (1024**3):.2f} GB)")
    print(f"{'='*60}")

    # ── 1. GGUF Header ──
    print("\n[1] GGUF Header")
    ok, errs = check_header(data)
    errors.extend(errs)
    if ok:
        print("  ✅ Magic + version OK")

    # ── 2. Metadata ──
    print("\n[2] Metadata")
    metadata, tensor_infos, errs = parse_gguf(data)
    errors.extend(errs)
    if metadata is None:
        print("  ❌ Cannot parse metadata — aborting further checks")
        report(errors, warnings)
        sys.exit(1)

    print(f"  Metadata keys: {len(metadata)}")
    print(f"  Tensor count:  {len(tensor_infos)}")

    # 2a. 必须字段
    print("\n  [2a] Required keys")
    for key in REQUIRED_METADATA_KEYS:
        if key in metadata:
            print(f"    ✅ {key}")
        else:
            err = f"missing required metadata key: {key}"
            errors.append(err)
            print(f"    ❌ {err}")

    # 2b. architecture + weight_format
    arch = metadata.get("general.architecture", "")
    wf = metadata.get("pleiades.weight_format", "")
    print(f"\n  [2b] Architecture check")
    if arch == "deepseek_v4":
        print(f"    ✅ general.architecture = deepseek_v4")
    else:
        errors.append(f"expected general.architecture='deepseek_v4', got '{arch}'")
        print(f"    ❌ general.architecture = '{arch}' (expected 'deepseek_v4')")

    if wf == "safetensors":
        print(f"    ✅ pleiades.weight_format = safetensors")
    else:
        errors.append(f"expected pleiades.weight_format='safetensors', got '{wf}'")
        print(f"    ❌ pleiades.weight_format = '{wf}'")

    # 2c. 值范围
    print("\n  [2c] Value ranges")
    for key, (lo, hi) in RANGES.items():
        val = metadata.get(key)
        if val is None:
            continue  # already caught in 2a
        if not isinstance(val, int):
            errors.append(f"{key}: expected int, got {type(val).__name__}")
            print(f"    ❌ {key} = {val} (not int)")
        elif not (lo <= val <= hi):
            err = f"{key} = {val} out of range [{lo}, {hi}]"
            errors.append(err)
            print(f"    ❌ {err}")
        else:
            print(f"    ✅ {key} = {val}")

    # 2d. compress_ratios
    cr = metadata.get("deepseek_v4.compress_ratios", "")
    n_layers = metadata.get("deepseek_v4.block_count", 0)
    if cr:
        ratios = [int(x) for x in cr.split(",")]
        if len(ratios) == n_layers:
            print(f"    ✅ compress_ratios: {len(ratios)} entries, matches block_count={n_layers}")
        else:
            err = f"compress_ratios length {len(ratios)} != block_count {n_layers}"
            errors.append(err)
            print(f"    ❌ {err}")
    elif n_layers > 0:
        warnings.append("compress_ratios is empty (will default to all zeros)")
        print(f"    ⚠️  compress_ratios empty")

    # 2e. score_func
    score_func = metadata.get("deepseek_v4.score_func", "")
    valid_sf = {"sqrtsoftplus", "softmax", "sigmoid"}
    if score_func in valid_sf:
        print(f"    ✅ score_func = {score_func}")
    else:
        errors.append(f"unknown score_func: '{score_func}' (expected one of {valid_sf})")
        print(f"    ❌ score_func = '{score_func}'")

    # ── 3. Tensors ──
    print("\n[3] Tensor shards")
    num_shards = metadata.get("pleiades.num_shards", 0)
    if num_shards <= 0:
        errors.append("pleiades.num_shards must be > 0")
        print(f"    ❌ num_shards = {num_shards}")
        report(errors, warnings)
        sys.exit(1)

    shard_names = [f"safetensors/shard-{i}" for i in range(num_shards)]
    missing_shards = [n for n in shard_names if n not in tensor_infos]
    extra_tensors = [n for n in tensor_infos if not n.startswith("safetensors/shard-")]

    if missing_shards:
        for s in missing_shards:
            errors.append(f"missing tensor: {s}")
            print(f"    ❌ missing: {s}")
    else:
        print(f"    ✅ All {num_shards} shard tensors present")

    if extra_tensors:
        warnings.append(f"{len(extra_tensors)} non-shard tensors found: {extra_tensors}")
        print(f"    ⚠️  extra tensors: {extra_tensors}")

    # ── 4. Safetensors 格式验证 ──
    print(f"\n[4] Safetensors format validation")

    all_tensor_names: list[str] = []
    total_safetensors_size = 0

    for i, shard_name in enumerate(shard_names):
        info = tensor_infos.get(shard_name)
        if info is None:
            continue
        offset = info["offset"]
        length = info["size"]
        shard_bytes = data[offset:offset + length]
        total_safetensors_size += length

        ok, names, errs = validate_safetensors(shard_bytes, shard_name)
        if ok:
            print(f"    ✅ {shard_name}: {len(names)} tensors, {length / (1024**2):.1f} MB")
            all_tensor_names.extend(names)
        else:
            errors.extend(errs)
            for e in errs:
                print(f"    ❌ {e}")

    # ── 5. 跨分片 tensor 名去重 ──
    print(f"\n[5] Cross-shard tensor name dedup")
    dupes = [name for name, cnt in Counter(all_tensor_names).items() if cnt > 1]
    if dupes:
        for d in dupes:
            errors.append(f"duplicate tensor across shards: {d}")
        print(f"    ❌ {len(dupes)} duplicate tensor names found")
    else:
        print(f"    ✅ No duplicate tensor names ({len(all_tensor_names)} total)")

    # ── 6. 文件大小合理性 ──
    print(f"\n[6] File size sanity")
    overhead = file_size - total_safetensors_size
    overhead_pct = overhead / file_size * 100 if file_size else 0
    if overhead_pct < 10:
        print(f"    ✅ Overhead: {overhead / (1024**2):.1f} MB ({overhead_pct:.1f}%)")
    else:
        warnings.append(f"high overhead: {overhead_pct:.1f}% ({overhead / (1024**2):.1f} MB)")
        print(f"    ⚠️  High overhead: {overhead_pct:.1f}% ({overhead / (1024**2):.1f} MB)")

    # ── 7. model_id + layer_bitmap ──
    print(f"\n[7] PGGUF metadata")
    model_id = metadata.get("pleiades.model_id")
    layer_bitmap = metadata.get("pleiades.layer_bitmap", "")
    if model_id is not None:
        print(f"    ✅ model_id = {model_id:#010x}")
    if layer_bitmap:
        bits_set = sum(bin(int(layer_bitmap[i:i+2], 16)).count('1') for i in range(0, 64, 2))
        print(f"    ✅ layer_bitmap: {bits_set} layers covered")

    # ── 8. Tokenizer & Chat Template ──
    print(f"\n[8] Tokenizer & Chat Template")
    _check_tokenizer(metadata, errors, warnings)

    # ── 最终报告 ──
    report(errors, warnings)


# ═══════════════════════════════════════════════════════════════
# Helpers
# ═══════════════════════════════════════════════════════════════

def check_header(data: bytes) -> tuple[bool, list[str]]:
    errs: list[str] = []
    if len(data) < 16:
        errs.append("file too small (< 16 bytes)")
        return False, errs

    magic = data[:4]
    version = struct.unpack_from("<I", data, 4)[0]

    if magic != GGUF_MAGIC:
        errs.append(f"bad magic: {magic!r} (expected {GGUF_MAGIC!r})")
    if version != GGUF_VERSION:
        errs.append(f"unsupported GGUF version: {version} (expected {GGUF_VERSION})")

    return len(errs) == 0, errs


def parse_gguf(data: bytes) -> tuple[dict[str, Any] | None, dict[str, dict], list[str]]:
    """解析 GGUF metadata + tensor 索引。返回 (metadata, tensor_infos, errors)。"""
    errs: list[str] = []
    if len(data) < 16:
        return None, {}, ["file too small"]

    version = struct.unpack_from("<I", data, 4)[0]
    n_tensors = struct.unpack_from("<Q", data, 8)[0]
    n_kv = struct.unpack_from("<Q", data, 12)[0]

    offset = 16  # 跳过 header

    metadata: dict[str, Any] = {}
    for _ in range(n_kv):
        if offset + 8 > len(data):
            errs.append("metadata: unexpected EOF")
            return metadata, {}, errs

        key_len = struct.unpack_from("<Q", data, offset)[0]
        offset += 8
        if offset + key_len > len(data):
            errs.append(f"metadata key: unexpected EOF at offset {offset}")
            return metadata, {}, errs

        key = data[offset:offset + key_len].decode("utf-8", errors="replace")
        offset += key_len

        if offset + 4 > len(data):
            errs.append(f"metadata value type for '{key}': unexpected EOF")
            return metadata, {}, errs

        val_type = struct.unpack_from("<I", data, offset)[0]
        offset += 4

        val, offset = read_gguf_value(data, offset, val_type, version)
        metadata[key] = val

    # Tensor infos
    tensor_infos: dict[str, dict] = {}
    for _ in range(n_tensors):
        if offset + 8 > len(data):
            errs.append("tensor info: unexpected EOF")
            return metadata, tensor_infos, errs

        name_len = struct.unpack_from("<Q", data, offset)[0]
        offset += 8
        if offset + name_len > len(data):
            errs.append("tensor name: unexpected EOF")
            return metadata, tensor_infos, errs

        name = data[offset:offset + name_len].decode("utf-8", errors="replace")
        offset += name_len

        if offset + 4 > len(data):
            errs.append(f"tensor '{name}': EOF reading n_dims")
            return metadata, tensor_infos, errs

        n_dims = struct.unpack_from("<I", data, offset)[0]
        offset += 4

        dims = []
        for _ in range(n_dims):
            if offset + 8 > len(data):
                errs.append(f"tensor '{name}': EOF reading dims")
                return metadata, tensor_infos, errs
            dims.append(struct.unpack_from("<Q", data, offset)[0])
            offset += 8

        if offset + 4 > len(data):
            errs.append(f"tensor '{name}': EOF reading dtype")
            return metadata, tensor_infos, errs
        dtype = struct.unpack_from("<I", data, offset)[0]
        offset += 4

        if offset + 8 > len(data):
            errs.append(f"tensor '{name}': EOF reading tensor offset")
            return metadata, tensor_infos, errs
        tensor_offset = struct.unpack_from("<Q", data, offset)[0]
        offset += 8

        # Compute tensor size: product of dims * type_size
        elem_count = 1
        for d in dims:
            elem_count *= d
        type_size_map = {
            0: 1, 1: 1, 2: 2, 3: 2, 4: 4, 5: 4, 6: 4, 7: 1, 8: 1,
            10: 8, 11: 8, 12: 8, 13: 2, 14: 2, 15: 2, 16: 2, 17: 4,
            18: 2, 19: 2, 20: 4, 21: 6, 22: 8, 23: 2, 24: 4, 25: 8,
        }
        ts = type_size_map.get(dtype, 1)
        tensor_size = elem_count * ts // max(block_size(dtype), 1)

        tensor_infos[name] = {
            "dims": dims,
            "dtype": dtype,
            "offset": tensor_offset,
            "size": tensor_size,
        }

    return metadata, tensor_infos, errs


def block_size(dtype: int) -> int:
    """GGML 量化类型的 block size (0 = 非量化)."""
    quant_block_sizes = {
        10: 256, 11: 256, 12: 256, 13: 32, 14: 32, 15: 32, 16: 32,
        17: 256, 18: 256, 19: 256, 20: 256, 21: 256, 22: 256, 23: 32,
        24: 32, 25: 256,
    }
    return quant_block_sizes.get(dtype, 0)


def read_gguf_value(data: bytes, offset: int, val_type: int, version: int) -> tuple[Any, int]:
    """读取一个 GGUF value，返回 (value, new_offset)。"""
    if val_type == GGUF_TYPE_UINT8:
        return data[offset], offset + 1
    elif val_type == GGUF_TYPE_INT8:
        return struct.unpack_from("<b", data, offset)[0], offset + 1
    elif val_type == GGUF_TYPE_UINT16:
        return struct.unpack_from("<H", data, offset)[0], offset + 2
    elif val_type == GGUF_TYPE_INT16:
        return struct.unpack_from("<h", data, offset)[0], offset + 2
    elif val_type == GGUF_TYPE_UINT32:
        return struct.unpack_from("<I", data, offset)[0], offset + 4
    elif val_type == GGUF_TYPE_INT32:
        return struct.unpack_from("<i", data, offset)[0], offset + 4
    elif val_type == GGUF_TYPE_FLOAT32:
        return struct.unpack_from("<f", data, offset)[0], offset + 4
    elif val_type == GGUF_TYPE_BOOL:
        return data[offset] != 0, offset + 1
    elif val_type == GGUF_TYPE_STRING:
        length = struct.unpack_from("<Q", data, offset)[0]
        offset += 8
        s = data[offset:offset + length].decode("utf-8", errors="replace")
        return s, offset + length
    elif val_type == GGUF_TYPE_UINT64:
        return struct.unpack_from("<Q", data, offset)[0], offset + 8
    elif val_type == GGUF_TYPE_INT64:
        return struct.unpack_from("<q", data, offset)[0], offset + 8
    elif val_type == GGUF_TYPE_FLOAT64:
        return struct.unpack_from("<d", data, offset)[0], offset + 8
    elif val_type == GGUF_TYPE_ARRAY:
        # array: [type, len, values...]
        elem_type = struct.unpack_from("<I", data, offset)[0]
        offset += 4
        arr_len = struct.unpack_from("<Q", data, offset)[0]
        offset += 8
        # 对于 array of string，我们返回字符串列表
        # 对于我们的用途（compress_ratios），只需跳过
        # 简化处理：跳过整个数组
        for _ in range(arr_len):
            _, offset = read_gguf_value(data, offset, elem_type, version)
        return f"<array[{arr_len}]>", offset
    else:
        return f"<unknown type {val_type}>", offset


def validate_safetensors(data: bytes, label: str) -> tuple[bool, list[str], list[str]]:
    """验证单个 safetensors 分片格式。返回 (ok, tensor_names, errors)。"""
    errs: list[str] = []
    names: list[str] = []

    if len(data) < 8:
        errs.append(f"{label}: too small for safetensors header (< 8 bytes)")
        return False, names, errs

    header_len = struct.unpack_from("<Q", data, 0)[0]
    if header_len > len(data) - 8:
        errs.append(f"{label}: header_len={header_len} exceeds file size {len(data)}")
        return False, names, errs
    if header_len > 100 * 1024 * 1024:
        errs.append(f"{label}: header_len={header_len} implausibly large")
        return False, names, errs

    try:
        header = json.loads(data[8:8 + header_len].decode("utf-8"))
    except (json.JSONDecodeError, UnicodeDecodeError) as e:
        errs.append(f"{label}: invalid JSON header: {e}")
        return False, names, errs

    if not isinstance(header, dict):
        errs.append(f"{label}: header is not a JSON object")
        return False, names, errs

    tensor_count = 0
    for name, info in header.items():
        if name == "__metadata__":
            continue
        tensor_count += 1
        names.append(name)

        # 验证 shape / dtype / data_offsets
        if not isinstance(info, dict):
            errs.append(f"{label}/{name}: not a dict")
            continue
        if "dtype" not in info:
            errs.append(f"{label}/{name}: missing dtype")
        if "shape" not in info:
            errs.append(f"{label}/{name}: missing shape")
        if "data_offsets" not in info:
            errs.append(f"{label}/{name}: missing data_offsets")
        else:
            offs = info["data_offsets"]
            if not isinstance(offs, list) or len(offs) != 2:
                errs.append(f"{label}/{name}: data_offsets must be [begin, end]")
            else:
                begin, end = offs[0], offs[1]
                if begin > end or end > len(data) - 8 - header_len:
                    errs.append(f"{label}/{name}: data_offsets [{begin}, {end}] out of range")

    return len(errs) == 0, names, errs


def report(errors: list[str], warnings: list[str]) -> None:
    print(f"\n{'='*60}")
    if errors:
        print(f"❌ FAILED — {len(errors)} error(s):")
        for e in errors:
            print(f"   • {e}")
    else:
        print("✅ ALL CHECKS PASSED")

    if warnings:
        print(f"\n⚠️  {len(warnings)} warning(s):")
        for w in warnings:
            print(f"   • {w}")

    if errors:
        sys.exit(1)
    else:
        sys.exit(0)


def _check_tokenizer(metadata: dict, errors: list[str], warnings: list[str]) -> None:
    """检查 tokenizer 和 chat template 是否完整嵌入。"""

    # 8a. 核心 tokenizer 字段
    for key, (expected_type, desc) in TOKENIZER_KEYS.items():
        if key in metadata:
            val = metadata[key]
            if key == "tokenizer.ggml.model":
                model = str(val)
                known = {"gpt2", "llama", "bert", "t5", "falcon"}
                if model.lower() in known:
                    print(f"    ✅ {key} = '{model}'")
                else:
                    warnings.append(f"{key} = '{model}' (non-standard; shimmytok may not support)")
                    print(f"    ⚠️  {key} = '{model}' (non-standard)")
            elif key == "tokenizer.ggml.tokens":
                # 值是 string "<array[N]>" — 我们无法知道实际长度
                # 但可以通过 vocab_size 交叉验证
                vocab_size = metadata.get("deepseek_v4.vocab_size", 0)
                if vocab_size > 0:
                    print(f"    ✅ {key} present (expected ~{vocab_size} tokens)")
                else:
                    print(f"    ✅ {key} present")
            elif key == "tokenizer.chat_template":
                tmpl = str(val)
                # 检查 chat template 是否非空且包含 Jinja2 特征
                if len(tmpl) < 10:
                    errors.append(f"{key} is too short ({len(tmpl)} chars) — likely empty or broken")
                    print(f"    ❌ {key}: {len(tmpl)} chars (too short)")
                elif "{{" not in tmpl and "{%" not in tmpl:
                    warnings.append(f"{key} has no Jinja2 syntax ({{{{...}}}} or {{%...%}}) — may be plain text")
                    print(f"    ⚠️  {key}: {len(tmpl)} chars, no Jinja2 syntax detected")
                    print(f"       Preview: {tmpl[:120]}...")
                else:
                    print(f"    ✅ {key}: {len(tmpl)} chars (Jinja2 detected)")
                    # 显示前 120 字符预览
                    preview = tmpl[:120].replace("\n", "\\n")
                    print(f"       Preview: {preview}...")
        else:
            if key == "tokenizer.chat_template":
                # chat_template 缺失是 warning（模型仍可加载，但对话格式有问题）
                warnings.append(f"missing {key} ({desc})")
                print(f"    ⚠️  missing {key} — chat format will be broken")
            else:
                errors.append(f"missing {key} ({desc})")
                print(f"    ❌ missing {key}")

    # 8b. EOS token
    eos = metadata.get("tokenizer.ggml.eos_token_id")
    if eos is not None:
        print(f"    ✅ tokenizer.ggml.eos_token_id = {eos}")
    else:
        warnings.append("missing tokenizer.ggml.eos_token_id")
        print(f"    ⚠️  missing tokenizer.ggml.eos_token_id")

    # 8c. BOS token
    bos = metadata.get("tokenizer.ggml.bos_token_id")
    if bos is not None:
        print(f"    ✅ tokenizer.ggml.bos_token_id = {bos}")
    else:
        warnings.append("missing tokenizer.ggml.bos_token_id")
        print(f"    ⚠️  missing tokenizer.ggml.bos_token_id (optional)")

    # 8d. Merges (BPE models only — check if model is 'gpt2')
    merges = metadata.get("tokenizer.ggml.merges")
    model_type = str(metadata.get("tokenizer.ggml.model", ""))
    if model_type.lower() == "gpt2":
        if merges is not None:
            print(f"    ✅ tokenizer.ggml.merges present (BPE merge rules)")
        else:
            warnings.append("BPE model ('gpt2') missing tokenizer.ggml.merges")
            print(f"    ⚠️  missing tokenizer.ggml.merges (BPE models need merge rules)")

    # 8e. 交叉验证: vocab_size vs token count
    vocab_size = metadata.get("deepseek_v4.vocab_size", 0)
    if vocab_size > 0:
        tokens_val = metadata.get("tokenizer.ggml.tokens", "")
        if isinstance(tokens_val, str) and tokens_val.startswith("<array["):
            # 尝试提取 array 长度
            try:
                n = int(tokens_val.split("[")[1].split("]")[0])
                if n >= vocab_size:
                    print(f"    ✅ tokenizer tokens count ({n}) >= vocab_size ({vocab_size})")
                else:
                    errors.append(f"tokenizer tokens ({n}) < vocab_size ({vocab_size})")
                    print(f"    ❌ tokenizer tokens ({n}) < vocab_size ({vocab_size})")
            except (ValueError, IndexError):
                pass  # can't parse array size from our simplified reader


if __name__ == "__main__":
    main()
