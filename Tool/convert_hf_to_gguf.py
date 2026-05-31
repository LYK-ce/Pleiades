#!/usr/bin/env python3
# convert_hf_to_gguf — CLI entry point for HuggingFace safetensors → PGGUF
#
# Presented by KeJi
# Date: 2026-05-31
#
# Usage:
#   python Tool/convert_hf_to_gguf.py <model_dir> [-o output.pgguf] [--arch qwen3]
#
# Requirements:
#   pip install safetensors gguf numpy

import argparse
import sys
from pathlib import Path

# Ensure Tool/ is on path
_SCRIPT_DIR = Path(__file__).resolve().parent
if str(_SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPT_DIR))

from hf2gguf import convert_hf_to_pgguf
from hf2gguf.utils import discover_model_dir


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Convert HuggingFace safetensors model to Pleiades PGGUF format",
    )
    parser.add_argument(
        "model_dir",
        type=Path,
        help="Path to HuggingFace model directory (must contain config.json)",
    )
    parser.add_argument(
        "-o", "--output",
        type=Path,
        default=None,
        help="Output .pgguf path (default: {model_dir}.pgguf)",
    )
    parser.add_argument(
        "--arch",
        type=str,
        default=None,
        choices=["qwen3", "qwen3moe", "deepseek_v3", "deepseek2"],
        help="Force architecture (skip auto-detection)",
    )
    args = parser.parse_args()

    try:
        model_dir = discover_model_dir(args.model_dir)
        convert_hf_to_pgguf(
            model_dir,
            output_path=args.output,
            arch_override=args.arch,
        )
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
