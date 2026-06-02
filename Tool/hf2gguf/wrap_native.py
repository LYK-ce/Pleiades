# wrap_native — Safetensors 分片原样打包进 PGGUF（--wrap-native 模式）
#
# Presented by KeJi
# Date: 2026-06-02
#
# 完全手写 GGUF 二进制格式，零依赖 gguf-py。
# GGUF header + metadata KV pairs + tensor info table 保留标准结构，
# 但 tensor 数据区直接存放原始 safetensors 字节（不做 GGML 量化）。
# Rust 侧通过 weight_format="safetensors" 标记走 seek+read 路径。

import json
import struct
from pathlib import Path
from typing import Any

import xxhash

from .config_reader import read_config
from .pgguf_writer import build_layer_bitmap, bitmap_to_hex
from .utils import has_sharded_tensors

# ── GGUF 常量 ──
GGUF_MAGIC = b"GGUF"
GGUF_VERSION = 3
GGUF_ALIGNMENT = 32

VT_UINT8 = 0
VT_INT8 = 1
VT_UINT32 = 4
VT_FLOAT32 = 6
VT_BOOL = 7
VT_STRING = 8
VT_ARRAY = 9
VT_UINT64 = 10

GGML_F32 = 0


# ═══════════════════════════════════════════════════════════
# GGUF 二进制写入辅助
# ═══════════════════════════════════════════════════════════

def _u8(v: int) -> bytes:
    return struct.pack("<B", v)

def _u32(v: int) -> bytes:
    return struct.pack("<I", v)

def _u64(v: int) -> bytes:
    return struct.pack("<Q", v)

def _f32(v: float) -> bytes:
    return struct.pack("<f", v)

def _str(s: str) -> bytes:
    b = s.encode("utf-8")
    return _u64(len(b)) + b

def _kv(key: str, vtype: int, vdata: bytes) -> bytes:
    return _str(key) + _u32(vtype) + vdata

def _kv_string(key: str, value: str) -> bytes:
    return _kv(key, VT_STRING, _str(value))

def _kv_uint32(key: str, value: int) -> bytes:
    return _kv(key, VT_UINT32, _u32(value))

def _kv_float32(key: str, value: float) -> bytes:
    return _kv(key, VT_FLOAT32, _f32(value))

def _kv_array(key: str, elem_type: int, items: list[bytes]) -> bytes:
    body = _u32(elem_type) + _u64(len(items))
    for item in items:
        body += item
    return _kv(key, VT_ARRAY, body)

def _arr_str(s: str) -> bytes:
    return _str(s)


# ═══════════════════════════════════════════════════════════
# Tokenizer
# ═══════════════════════════════════════════════════════════

def _write_tokenizer_kv(model_dir: Path) -> list[tuple[str, bytes]]:
    kvs: list[tuple[str, bytes]] = []

    tok_path = model_dir / "tokenizer.json"
    if not tok_path.exists():
        cfg_path = model_dir / "tokenizer_config.json"
        if cfg_path.exists():
            with open(cfg_path, "r", encoding="utf-8") as f:
                cfg = json.load(f)
            kvs.append(("tokenizer.ggml.model", _kv_string("tokenizer.ggml.model", "gpt2")))
            eos = cfg.get("eos_token_id")
            bos = cfg.get("bos_token_id")
            if eos is not None and isinstance(eos, int):
                kvs.append(("tokenizer.ggml.eos_token_id", _kv_uint32("tokenizer.ggml.eos_token_id", eos)))
            if bos is not None and isinstance(bos, int):
                kvs.append(("tokenizer.ggml.bos_token_id", _kv_uint32("tokenizer.ggml.bos_token_id", bos)))
            ct = cfg.get("chat_template")
            if ct:
                kvs.append(("tokenizer.chat_template", _kv_string("tokenizer.chat_template", ct)))
            print(f"  [tokenizer] from config: eos={eos}, bos={bos}, chat={'yes' if ct else 'no'}")
            return kvs
        print("  [warn] tokenizer.json not found, skipping")
        return kvs

    with open(tok_path, "r", encoding="utf-8") as f:
        tok = json.load(f)

    model_type = tok.get("model", {}).get("type", "bpe").lower()
    m = {"bpe": "gpt2", "BBPE": "gpt2", "wordpiece": "bert", "unigram": "llama", "sentencepiece": "llama"}
    kvs.append(("tokenizer.ggml.model", _kv_string("tokenizer.ggml.model", m.get(model_type, model_type))))

    vocab: dict = tok.get("model", {}).get("vocab", {})
    items = [(tid, tstr) for tstr, tid in vocab.items()]
    items.sort(key=lambda x: x[0])
    added = tok.get("added_tokens", [])
    for e in added:
        tid = e.get("id")
        if tid is not None and tid not in {t[0] for t in items}:
            items.append((tid, e.get("content", "")))
    items.sort(key=lambda x: x[0])
    if items:
        mx = items[-1][0]
        ex = {t[0] for t in items}
        for tid in range(mx + 1):
            if tid not in ex:
                items.append((tid, ""))
        items.sort(key=lambda x: x[0])
    tokens = [t for _, t in items]
    kvs.append(("tokenizer.ggml.tokens", _kv_array("tokenizer.ggml.tokens", VT_STRING, [_arr_str(t) for t in tokens])))

    merges = tok.get("model", {}).get("merges", [])
    if merges:
        if merges and isinstance(merges[0], list):
            merges = [" ".join(m) for m in merges]
        kvs.append(("tokenizer.ggml.merges", _kv_array("tokenizer.ggml.merges", VT_STRING, [_arr_str(m) for m in merges])))

    sm: dict[str, int] = {}
    for e in added:
        if e.get("special", False) and "content" in e:
            sm[e["content"]] = e["id"]
    eos = sm.get("</s>") or sm.get("<|endoftext|>") or sm.get("<|im_end|>")
    bos = sm.get("<s>") or sm.get("<|beginoftext|>") or sm.get("<|im_start|>")
    if eos is not None:
        kvs.append(("tokenizer.ggml.eos_token_id", _kv_uint32("tokenizer.ggml.eos_token_id", eos)))
    if bos is not None:
        kvs.append(("tokenizer.ggml.bos_token_id", _kv_uint32("tokenizer.ggml.bos_token_id", bos)))

    ct = tok.get("chat_template", None)
    if not ct:
        tcp = model_dir / "tokenizer_config.json"
        if tcp.exists():
            with open(tcp, "r", encoding="utf-8") as f:
                ct = json.load(f).get("chat_template", None)
    if ct:
        kvs.append(("tokenizer.chat_template", _kv_string("tokenizer.chat_template", ct)))

    print(f"  [tokenizer] model={model_type}, vocab={len(items)}, eos={eos}, bos={bos}, merges={len(merges)}, chat={'yes' if ct else 'no'}")
    return kvs


# ═══════════════════════════════════════════════════════════
# 主流程
# ═══════════════════════════════════════════════════════════

def _read_index_json(model_dir: Path) -> dict[str, Any]:
    p = model_dir / "model.safetensors.index.json"
    if not p.exists():
        raise FileNotFoundError(str(p))
    with open(p, "r", encoding="utf-8") as f:
        return json.load(f)


def _get_shard_files(index: dict[str, Any]) -> list[str]:
    return sorted(set(index.get("weight_map", {}).values()))


def _make_metadata_kvs(config: dict[str, Any], num_shards: int, model_dir: Path) -> list[tuple[str, bytes]]:
    kvs: list[tuple[str, bytes]] = []

    def s(k, v): kvs.append((k, _kv_string(k, v)))
    def u(k, v): kvs.append((k, _kv_uint32(k, v)))
    def f(k, v): kvs.append((k, _kv_float32(k, v)))

    s("general.architecture", "deepseek_v4")
    s("pleiades.weight_format", "safetensors")
    u("pleiades.num_shards", num_shards)

    nl = config.get("num_hidden_layers", config.get("n_layers", 0))
    hs = config.get("hidden_size", config.get("dim", 0))

    u("deepseek_v4.block_count", nl)
    u("deepseek_v4.embedding_length", hs)
    u("deepseek_v4.vocab_size", config.get("vocab_size", 0))
    u("deepseek_v4.attention.head_count", config.get("num_attention_heads", config.get("n_heads", 0)))
    u("deepseek_v4.attention.key_length", config.get("head_dim", 0))
    u("deepseek_v4.feed_forward_length", config.get("moe_inter_dim", config.get("intermediate_size", 0)))
    u("deepseek_v4.context_length", config.get("max_seq_len", config.get("max_position_embeddings", 4096)))
    f("deepseek_v4.attention.layer_norm_rms_epsilon", config.get("rms_norm_eps", config.get("norm_eps", 1e-6)))
    f("deepseek_v4.rope.freq_base", config.get("rope_theta", 10000.0))

    u("deepseek_v4.n_routed_experts", config.get("n_routed_experts", 0))
    u("deepseek_v4.n_shared_experts", config.get("n_shared_experts", 0))
    u("deepseek_v4.n_activated_experts", config.get("n_activated_experts", 0))
    u("deepseek_v4.q_lora_rank", config.get("q_lora_rank", 0))
    u("deepseek_v4.rope_head_dim", config.get("rope_head_dim", 0))
    u("deepseek_v4.hc_mult", config.get("hc_mult", 0))
    u("deepseek_v4.window_size", config.get("window_size", 0))
    f("deepseek_v4.rope_factor", config.get("rope_factor", 1.0))
    f("deepseek_v4.beta_fast", config.get("beta_fast", 32.0))
    f("deepseek_v4.beta_slow", config.get("beta_slow", 1.0))
    f("deepseek_v4.compress_rope_theta", config.get("compress_rope_theta", config.get("rope_theta", 10000.0)))
    u("deepseek_v4.original_seq_len", config.get("original_seq_len", 0))
    u("deepseek_v4.index_n_heads", config.get("index_n_heads", 0))
    u("deepseek_v4.index_head_dim", config.get("index_head_dim", 0))
    u("deepseek_v4.index_topk", config.get("index_topk", 0))
    u("deepseek_v4.hc_sinkhorn_iters", config.get("hc_sinkhorn_iters", 0))
    f("deepseek_v4.hc_eps", config.get("hc_eps", 1e-6))
    u("deepseek_v4.o_groups", config.get("o_groups", 1))
    u("deepseek_v4.o_lora_rank", config.get("o_lora_rank", 0))
    s("deepseek_v4.score_func", config.get("score_func", "sqrtsoftplus"))
    f("deepseek_v4.swiglu_limit", config.get("swiglu_limit", 0.0))
    f("deepseek_v4.route_scale", config.get("route_scale", 1.0))
    u("deepseek_v4.n_hash_layers", config.get("n_hash_layers", 0))

    dt = config.get("dtype", "")
    if dt: s("deepseek_v4.dtype", dt)
    sf = config.get("scale_fmt", "")
    if sf: s("deepseek_v4.scale_fmt", sf)
    ed = config.get("expert_dtype", "")
    if ed: s("deepseek_v4.expert_dtype", ed)

    cr = config.get("compress_ratios", [])
    s("deepseek_v4.compress_ratios", ",".join(str(r) for r in cr) if cr else "")

    print(f"  blocks={nl}, hidden={hs}, heads={config.get('n_heads', 0)}")
    print(f"  moe={config.get('n_routed_experts', 0)}/{config.get('n_shared_experts', 0)}/{config.get('n_activated_experts', 0)}")

    # Tokenizer
    print("  writing tokenizer metadata…")
    tok_kvs = _write_tokenizer_kv(model_dir)
    kvs.extend(tok_kvs)

    # PGGUF
    model_id = _model_id(model_dir, [])
    lb = build_layer_bitmap(nl)
    u("pleiades.model_id", model_id)
    s("pleiades.layer_bitmap", bitmap_to_hex(lb))
    print(f"  [pgguf] model_id={model_id:08x}, layers={nl + 2}")

    kvs.sort(key=lambda x: x[0])
    return kvs


def _ti(name: str, n_elem: int, offset: int) -> bytes:
    """写 tensor info: name, shape=[1, n_elem], dtype=F32, offset"""
    out = _str(name)
    out += _u32(2)         # n_dims
    out += _u64(1)
    out += _u64(n_elem)
    out += _u32(GGML_F32)  # dtype
    out += _u64(offset)
    return out


def _model_id(model_dir: Path, _shard_files: list[str]) -> int:
    # 简单 hash：用 config.json + tokenizer.json 内容
    h = xxhash.xxh32()
    for name in ["config.json", "tokenizer.json"]:
        p = model_dir / name
        if p.exists():
            h.update(p.read_bytes())
    return h.intdigest()


def wrap_native_to_pgguf(model_dir: Path, output_path: Path) -> Path:
    model_dir = model_dir.resolve()
    output_path = output_path.resolve()
    print(f"\n=== wrap-native mode ===\n  model_dir: {model_dir}\n  output: {output_path}")

    # 1. config
    print("\n[1/5] Reading config.json…")
    config = read_config(model_dir)
    print("  ok")

    # 2. 分片列表
    print("\n[2/5] Reading model.safetensors.index.json…")
    if not has_sharded_tensors(model_dir):
        raise ValueError("wrap-native mode requires a sharded model")
    index = _read_index_json(model_dir)
    shard_files = _get_shard_files(index)
    print(f"  found {len(shard_files)} shard files")

    # 3. 读全量 shard 数据
    print("\n[3/5] Reading shard data…")
    shard_data: list[bytes] = []
    for sf in shard_files:
        with open(model_dir / sf, "rb") as f:
            shard_data.append(f.read())
        print(f"  {sf}: {len(shard_data[-1]) / (1024**2):.1f} MB")
    total_data = sum(len(d) for d in shard_data)
    print(f"  total: {total_data / (1024**3):.2f} GB")

    # 4. metadata
    print("\n[4/5] Building metadata…")
    kvs = _make_metadata_kvs(config, len(shard_files), model_dir)
    n_kv = len(kvs)
    n_tensors = len(shard_files)

    kv_section = b"".join(raw for _, raw in kvs)

    ti_section = b""
    offs: list[int] = []
    sizes: list[int] = []
    running = 0
    for i, sd in enumerate(shard_data):
        plen = (len(sd) + 3) // 4 * 4
        n_f32 = plen // 4
        offs.append(running)
        sizes.append(plen)
        ti_section += _ti(f"safetensors/shard-{i}", n_f32, running)
        running += plen

    # 5. 写文件
    print(f"\n[5/5] Writing PGGUF ({running / (1024**3):.2f} GB tensor data)…")

    with open(output_path, "wb") as f:
        f.write(GGUF_MAGIC)
        f.write(_u32(GGUF_VERSION))
        f.write(_u64(n_tensors))
        f.write(_u64(n_kv))
        f.write(kv_section)
        f.write(ti_section)
        pos = f.tell()
        pad = (GGUF_ALIGNMENT - (pos % GGUF_ALIGNMENT)) % GGUF_ALIGNMENT
        f.write(b"\x00" * pad)
        for i, sd in enumerate(shard_data):
            f.write(sd)
            plen = sizes[i] - len(sd)
            if plen > 0:
                f.write(b"\x00" * plen)

    sz = output_path.stat().st_size
    print(f"\nDone: {output_path}")
    print(f"  safetensors: {total_data / (1024**3):.2f} GB, overhead: {(sz - total_data) / (1024**2):.1f} MB ({(sz - total_data) / sz * 100:.2f}%)")
    return output_path
