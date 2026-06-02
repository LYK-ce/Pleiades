# utils — Shared utilities for hf2gguf
#
# Presented by KeJi
# Date: 2026-05-31

import json
from pathlib import Path
from typing import Iterator

import numpy as np


def discover_model_dir(path: Path) -> Path:
    """Validate that the given path is a HuggingFace model directory.

    Must contain at minimum a config.json.
    Returns the resolved absolute path.
    """
    path = path.resolve()
    if not path.is_dir():
        raise NotADirectoryError(f"Not a directory: {path}")
    if not (path / "config.json").exists():
        raise FileNotFoundError(f"config.json not found in {path}")
    return path


def has_sharded_tensors(model_dir: Path) -> bool:
    """Check if model uses sharded safetensors (has index.json)."""
    return (model_dir / "model.safetensors.index.json").exists()


def load_single_tensor_file(model_dir: Path) -> dict[str, np.ndarray]:
    """Load tensors from a single model.safetensors file."""
    import safetensors  # lazy import

    st_path = model_dir / "model.safetensors"
    if not st_path.exists():
        raise FileNotFoundError(f"model.safetensors not found in {model_dir}")
    result: dict[str, np.ndarray] = {}
    with safetensors.safe_open(str(st_path), framework="pt") as sf:
        for key in sf.keys():
            result[key] = sf.get_tensor(key)
    return result


def load_sharded_tensors(model_dir: Path) -> dict[str, np.ndarray]:
    """Load tensors from sharded safetensors using index.json."""
    import safetensors  # lazy import

    index_path = model_dir / "model.safetensors.index.json"
    with open(index_path, "r", encoding="utf-8") as f:
        index = json.load(f)

    weight_map: dict[str, str] = index.get("weight_map", {})
    if not weight_map:
        raise ValueError("index.json has no weight_map")

    # Group tensors by shard file
    shard_files: dict[str, list[str]] = {}
    for tensor_name, shard_file in weight_map.items():
        shard_files.setdefault(shard_file, []).append(tensor_name)

    all_tensors: dict[str, np.ndarray] = {}
    for shard_file in sorted(shard_files.keys()):
        shard_path = model_dir / shard_file
        if not shard_path.exists():
            raise FileNotFoundError(f"Shard file not found: {shard_path}")
        with safetensors.safe_open(str(shard_path), framework="pt") as sf:
            for tensor_name in shard_files[shard_file]:
                all_tensors[tensor_name] = sf.get_tensor(tensor_name)

    return all_tensors


def resolve_tensor_source(model_dir: Path) -> dict[str, np.ndarray]:
    """Load all tensors — handles single-file and sharded automatically."""
    if has_sharded_tensors(model_dir):
        print(f"  [sharded] detected, reading index.json…")
        return load_sharded_tensors(model_dir)
    else:
        print(f"  [single] reading model.safetensors…")
        return load_single_tensor_file(model_dir)
