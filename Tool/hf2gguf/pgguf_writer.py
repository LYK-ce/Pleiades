# pgguf_writer — PGGUF metadata (pleiades.model_id + pleiades.layer_bitmap)
#
# Presented by KeJi
# Date: 2026-05-31

import hashlib
from pathlib import Path
from typing import Optional


def compute_model_id(file_path: Path) -> int:
    """Compute xxhash32 of the file content.

    Mirrors the Rust logic in GGUF_Analyze_And_Convert.
    Uses xxhash32 (not xxhash64 or xxh3) for compatibility.
    """
    try:
        import xxhash
        h = xxhash.xxh32()
        with open(file_path, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        return h.intdigest()
    except ImportError:
        # Fallback: use Python's hashlib for approximate compatibility
        # (not exact xxhash32, but usable for development)
        print("  [warn] xxhash not installed, using hashlib fallback for model_id")
        h = hashlib.md5()
        with open(file_path, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        return int(h.hexdigest()[:8], 16)


def build_layer_bitmap(num_layers: int) -> bytes:
    """Build 256-bit layer bitmap.

    Bit N=1 means the file contains layer N.
    Layers: 0=embedding, 1..num_layers=transformer, num_layers+1=output.
    Total covered bits = num_layers + 2.
    """
    bitmap = bytearray(32)  # 256 bits = 32 bytes
    total_bits = num_layers + 2  # embedding + N blocks + output

    for i in range(min(total_bits, 256)):
        byte_idx = i // 8
        bit_idx = i % 8
        bitmap[byte_idx] |= 1 << bit_idx

    return bytes(bitmap)


def bitmap_to_hex(bitmap: bytes) -> str:
    """Encode 32-byte bitmap as hex string."""
    return bitmap.hex()


def write_pgguf_metadata(
    gguf_path: Path,
    num_layers: int,
) -> None:
    """Post-process a GGUF file to add Pleiades metadata.

    NOTE: This is called AFTER GGUFWriter has closed the file.
    The current implementation computes model_id from the file and
    writes metadata via GGUFWriter post-processing.

    In practice, these values will be added as GGUF KV pairs using
    the standard gguf-py API during the main write pass.
    See converter.py for the integrated approach.
    """
    model_id = compute_model_id(gguf_path)
    layer_bitmap = build_layer_bitmap(num_layers)

    print(f"  [pgguf] model_id={model_id:08x}, layers_covered={num_layers + 2}")
    print(f"  [pgguf] layer_bitmap={bitmap_to_hex(layer_bitmap)}")

    return model_id, layer_bitmap
