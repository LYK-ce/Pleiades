# Presented by KeJi
# Date: 2026-03-23

"""
Token 提取脚本 - 用于获取 Qwen3 模型的 token IDs

使用方法:
    python tests/common/token_extract.py

功能:
    1. 加载 Qwen3-0.6B 的 tokenizer
    2. 编码指定文本获取 token IDs
    3. 输出详细的 token 信息
"""

import sys
from pathlib import Path

# 尝试导入 tokenizers
try:
    from tokenizers import Tokenizer
except ImportError:
    print("[错误] 请先安装 tokenizers: pip install tokenizers")
    sys.exit(1)

# 模型路径
MODEL_DIR = Path(__file__).parent.parent / "test_models" / "qwen3-0.6b"
TOKENIZER_PATH = MODEL_DIR / "tokenizer.json"


def Load_Tokenizer():
    """加载 tokenizer"""
    if not TOKENIZER_PATH.exists():
        print(f"[错误] Tokenizer 文件不存在: {TOKENIZER_PATH}")
        print("[提示] 请确保已下载 Qwen3-0.6B 模型到 tests/test_models/qwen3-0.6b/")
        sys.exit(1)
    
    tokenizer = Tokenizer.from_file(str(TOKENIZER_PATH))
    print(f"[成功] 加载 tokenizer: {TOKENIZER_PATH}")
    return tokenizer


def Encode_Text(tokenizer, text: str, add_special_tokens: bool = True):
    """编码文本并输出详细信息"""
    print(f"\n{'='*50}")
    print(f"输入文本: '{text}'")
    print(f"{'='*50}")
    
    # 编码
    encoding = tokenizer.encode(text, add_special_tokens=add_special_tokens)
    
    # 获取 token IDs
    token_ids = encoding.ids
    
    # 获取 tokens
    tokens = encoding.tokens
    
    print(f"\n编码结果:")
    print(f"  Token IDs: {token_ids}")
    print(f"  Tokens:    {tokens}")
    
    # 详细信息表格
    print(f"\n详细分解:")
    print(f"{'Index':<8}{'Token ID':<12}{'Token':<20}")
    print("-" * 40)
    for i, (tid, tok) in enumerate(zip(token_ids, tokens)):
        print(f"{i:<8}{tid:<12}{repr(tok):<20}")
    
    # 解码验证
    decoded = tokenizer.decode(token_ids)
    print(f"\n解码验证: '{decoded}'")
    
    return token_ids


def Generate_Rust_Code(token_ids: list):
    """生成 Rust 代码片段"""
    print(f"\n{'='*50}")
    print("Rust 代码片段 (可直接复制到测试中使用):")
    print(f"{'='*50}")
    
    # i64 格式
    ids_str = ", ".join([str(tid) for tid in token_ids])
    print(f"\nlet input_ids: Vec<i64> = vec![{ids_str}];")
    
    # usize 格式 (用于形状)
    seq_len = len(token_ids)
    print(f"let seq_len: usize = {seq_len};")


def main():
    print("="*50)
    print("Qwen3 Token 提取工具")
    print("="*50)
    
    # 加载 tokenizer
    tokenizer = Load_Tokenizer()
    
    # 测试文本
    test_text = "你好！"
    
    # 编码并输出
    token_ids = Encode_Text(tokenizer, test_text, add_special_tokens=True)
    
    # 生成 Rust 代码
    Generate_Rust_Code(token_ids)
    
    # 额外测试: 不带特殊 token
    print(f"\n{'='*50}")
    print("对比: 不带特殊 token")
    print(f"{'='*50}")
    token_ids_no_special = Encode_Text(tokenizer, test_text, add_special_tokens=False)
    
    print(f"\n[完成] Token 提取完成!")


if __name__ == "__main__":
    main()
