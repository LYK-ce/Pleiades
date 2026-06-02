# config_reader — Parse HuggingFace config.json → GGUF metadata dict
#
# Presented by KeJi
# Date: 2026-05-31

import json
from pathlib import Path
from typing import Any


def read_config(model_dir: Path) -> dict[str, Any]:
    """Read config.json from a HuggingFace model directory.

    Returns the raw config dict. The caller is responsible for interpreting
    architecture-specific keys.
    """
    config_path = model_dir / "config.json"
    if not config_path.exists():
        raise FileNotFoundError(f"config.json not found in {model_dir}")

    with open(config_path, "r", encoding="utf-8") as f:
        return json.load(f)


def identify_architecture(config: dict[str, Any]) -> str:
    """Map HF config to Pleiades architecture name.

    Returns one of: "qwen3", "qwen3moe", "deepseek_v3", "deepseek2".
    """
    arch_list: list[str] = config.get("architectures", [])

    if not arch_list:
        # Fallback: try model_type
        model_type = config.get("model_type", "")
        if "qwen3_moe" in model_type or "qwen3moe" in model_type:
            return "qwen3moe"
        if "qwen3" in model_type:
            return "qwen3"
        if "deepseek" in model_type:
            return "deepseek_v3"
        raise ValueError(f"Cannot identify architecture from config: {model_type}")

    hf_arch = arch_list[0]

    # Check Qwen3 MoE first (more specific)
    if "Qwen3MoE" in hf_arch or "Qwen3Moe" in hf_arch:
        return "qwen3moe"
    if "Qwen3" in hf_arch:
        return "qwen3"
    if "DeepseekV4" in hf_arch or "DeepSeekV4" in hf_arch:
        return "deepseek_v4"
    if "DeepSeekV3" in hf_arch or "DeepSeek" in hf_arch:
        return "deepseek_v3"
    if "deepseek2" in hf_arch.lower():
        return "deepseek2"

    raise ValueError(f"Unsupported architecture: {hf_arch}")
