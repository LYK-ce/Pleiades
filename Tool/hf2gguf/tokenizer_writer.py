# tokenizer_writer — Read tokenizer.json and write to GGUF metadata
#
# Presented by KeJi
# Date: 2026-05-31
#
# Writes tokenizer data into GGUF using the standard llama.cpp GGUF tokenizer schema
# (tokenizer.ggml.* namespace), compatible with shimmytok::Tokenizer::from_gguf_file().

import json
from pathlib import Path
from typing import Any

from gguf import GGUFWriter


def write_tokenizer(writer: GGUFWriter, model_dir: Path) -> None:
    """Read tokenizer.json and write tokenizer data to GGUFWriter.

    The GGUF tokenizer schema uses:
      - tokenizer.ggml.model       (e.g. "gpt2" for BPE)
      - tokenizer.ggml.tokens      (vocab list)
      - tokenizer.ggml.scores      (optional token scores)
      - tokenizer.ggml.merges      (BPE merge rules)
      - tokenizer.ggml.token_type  (token type per entry)
      - tokenizer.ggml.eos_token_id
      - tokenizer.ggml.bos_token_id
      - tokenizer.chat_template    (Jinja template string)
    """
    tok_path = model_dir / "tokenizer.json"
    if not tok_path.exists():
        # Also try tokenizer_config.json for special tokens
        tok_config_path = model_dir / "tokenizer_config.json"
        if tok_config_path.exists():
            _write_from_tokenizer_config(writer, tok_config_path)
            return
        print("  [warn] tokenizer.json not found, skipping tokenizer")
        return

    with open(tok_path, "r", encoding="utf-8") as f:
        tok = json.load(f)

    # ---- Model type ----
    model_type = tok.get("model", {}).get("type", "bpe").lower()
    # Map HF tokenizer model types to GGUF tokenizer.ggml.model values
    model_map = {
        "bpe": "gpt2",
        "BBPE": "gpt2",
        "wordpiece": "bert",
        "unigram": "llama",
        "sentencepiece": "llama",
    }
    writer.add_tokenizer_model(model_map.get(model_type, model_type))

    # ---- Vocab (token list) ----
    vocab: dict = tok.get("model", {}).get("vocab", {})
    # Build complete vocab: base vocab + added tokens
    # vocab is {token_str: token_id}, values are 0-indexed
    vocab_items = [(tid, tstr) for tstr, tid in vocab.items()]
    vocab_items.sort(key=lambda x: x[0])

    # Add added_tokens (special tokens like <|im_start|>, <|im_end|> etc.)
    added_tokens = tok.get("added_tokens", [])
    added_ids: set[int] = set()
    for entry in added_tokens:
        tid = entry.get("id")
        if tid is not None and tid not in {t[0] for t in vocab_items}:
            vocab_items.append((tid, entry.get("content", "")))
            if entry.get("special", False):
                added_ids.add(tid)
    vocab_items.sort(key=lambda x: x[0])

    # Pad gaps with empty strings
    if vocab_items:
        max_id = vocab_items[-1][0]
        existing = {t[0] for t in vocab_items}
        for tid in range(max_id + 1):
            if tid not in existing:
                vocab_items.append((tid, ""))
        vocab_items.sort(key=lambda x: x[0])

    tokens = [t for _, t in vocab_items]
    writer.add_token_list(tokens)

    # ---- Merges (BPE) ----
    merges = tok.get("model", {}).get("merges", [])
    if merges:
        if merges and isinstance(merges[0], list):
            merges = [" ".join(m) for m in merges]
        writer.add_token_merges(merges)

    # ---- Scores ----
    scores = tok.get("model", {}).get("scores", None)
    if scores:
        writer.add_token_scores(scores)

    # ---- Token types ----
    token_types = tok.get("model", {}).get("token_types", None)
    if token_types:
        writer.add_token_types(token_types)

    # ---- Special tokens (from added_tokens) ----
    special_map: dict[str, int] = {}
    for entry in added_tokens:
        if entry.get("special", False) and "content" in entry:
            special_map[entry["content"]] = entry["id"]

    eos_id = special_map.get("</s>") or special_map.get("<|endoftext|>") or special_map.get("<|im_end|>")
    bos_id = special_map.get("<s>") or special_map.get("<|beginoftext|>") or special_map.get("<|im_start|>")

    if eos_id is not None:
        writer.add_eos_token_id(eos_id)
    if bos_id is not None:
        writer.add_bos_token_id(bos_id)

    # Vocab size already set from config.json (e.g. 151936), don't override

    # ---- Chat template (from tokenizer_config.json, not tokenizer.json) ----
    chat_template = tok.get("chat_template", None)
    if not chat_template:
        tok_config_path = model_dir / "tokenizer_config.json"
        if tok_config_path.exists():
            with open(tok_config_path, "r", encoding="utf-8") as f:
                tok_cfg = json.load(f)
                chat_template = tok_cfg.get("chat_template", None)
    if chat_template:
        writer.add_chat_template(chat_template)

    print(f"  [tokenizer] model={model_type}, vocab={len(vocab_items)}, "
          f"eos={eos_id}, bos={bos_id}, merges={len(merges)}, "
          f"chat_template={'yes' if chat_template else 'no'}")


def _write_from_tokenizer_config(writer: GGUFWriter, config_path: Path) -> None:
    """Fallback: extract tokenizer info from tokenizer_config.json only."""
    print("  [warn] tokenizer.json not found, trying tokenizer_config.json…")
    with open(config_path, "r", encoding="utf-8") as f:
        cfg = json.load(f)

    writer.add_tokenizer_model("gpt2")

    eos_id = cfg.get("eos_token_id")
    bos_id = cfg.get("bos_token_id")
    if eos_id is not None and isinstance(eos_id, int):
        writer.add_eos_token_id(eos_id)
    if bos_id is not None and isinstance(bos_id, int):
        writer.add_bos_token_id(bos_id)

    # Vocab size already from config.json

    chat_template = cfg.get("chat_template")
    if chat_template:
        writer.add_chat_template(chat_template)
