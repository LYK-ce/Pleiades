# hf2gguf/mappings — Tensor name mapping tables per architecture
#
# Presented by KeJi
# Date: 2026-05-31

from .qwen3 import QWEN3_TENSOR_MAP
from .qwen3_moe import QWEN3_MOE_TENSOR_MAP
from .deepseek import DEEPSEEK_TENSOR_MAP

ARCHITECTURE_MAP: dict[str, dict[str, str]] = {
    "qwen3": QWEN3_TENSOR_MAP,
    "qwen3moe": QWEN3_MOE_TENSOR_MAP,
    "deepseek_v3": DEEPSEEK_TENSOR_MAP,
    "deepseek2": DEEPSEEK_TENSOR_MAP,
}

__all__ = [
    "QWEN3_TENSOR_MAP",
    "QWEN3_MOE_TENSOR_MAP",
    "DEEPSEEK_TENSOR_MAP",
    "ARCHITECTURE_MAP",
]
