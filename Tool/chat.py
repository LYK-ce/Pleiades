#!/usr/bin/env python3
"""Pleiades Chat Client — 多轮对话工具

使用 Pleiades OpenAI 兼容 API（`api <session_id>` 命令启动）进行多轮对话。
默认流式输出，对话历史由本脚本维护。

用法:
  python chat.py [选项]

选项:
  --url URL           API 地址 (默认: http://127.0.0.1:8080)
  --model NAME        模型名 (默认从 /v1/models 自动获取)
  -s, --system TEXT   System prompt (/clear 不会清除)
  --no-stream         禁用流式输出
  --max-tokens N      最大生成 token 数 (默认: 256)

示例:
  python chat.py
  python chat.py --url http://127.0.0.1:8080
  python chat.py -s "你是一个乐于助人的助手"
  python chat.py -s "用简洁的语言回答" --max-tokens 512
  python chat.py --no-stream

对话命令:
  /exit     退出
  /clear    清空历史（保留 system prompt）
  /system   显示当前 system prompt

依赖: Python 3.10+ (仅标准库)
"""

import argparse
import json
import os
import sys
import urllib.request
import urllib.error


# ── SSE 流式读取 ────────────────────────────────────────────

def stream_chat(url: str, payload: bytes) -> None:
    """发送流式请求，实时逐 token 打印。"""
    req = urllib.request.Request(
        url,
        data=payload,
        headers={"Content-Type": "application/json", "Accept": "text/event-stream"},
        method="POST",
    )

    with urllib.request.urlopen(req) as resp:
        if resp.status != 200:
            print(f"[错误] HTTP {resp.status}", file=sys.stderr)
            return

        content = ""
        for line_bytes in resp:
            line = line_bytes.decode("utf-8").strip()
            if not line or line.startswith(":"):
                continue
            if line == "data: [DONE]":
                break
            if line.startswith("data: "):
                data = line[6:]
                try:
                    chunk = json.loads(data)
                    delta = chunk.get("choices", [{}])[0].get("delta", {})
                    token = delta.get("content")
                    if token:
                        print(token, end="", flush=True)
                        content += token
                except json.JSONDecodeError:
                    pass
        print()  # 换行
        return content


# ── 非流式请求 ──────────────────────────────────────────────

def nonstream_chat(url: str, payload: bytes) -> str | None:
    """发送非流式请求，返回完整回复。"""
    req = urllib.request.Request(
        url,
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )

    try:
        with urllib.request.urlopen(req) as resp:
            body = json.loads(resp.read().decode("utf-8"))
            content = body.get("choices", [{}])[0].get("message", {}).get("content", "")
            print(content)
            return content
    except urllib.error.HTTPError as e:
        print(f"[错误] HTTP {e.code}: {e.reason}", file=sys.stderr)
        return None


# ── 获取模型名 ──────────────────────────────────────────────

def get_model(base_url: str) -> str | None:
    """从 /v1/models 获取模型名。"""
    try:
        with urllib.request.urlopen(f"{base_url}/v1/models") as resp:
            body = json.loads(resp.read().decode("utf-8"))
            models = body.get("data", [])
            return models[0]["id"] if models else None
    except Exception as e:
        print(f"[警告] 无法获取模型列表: {e}", file=sys.stderr)
        return None


# ── 主程序 ──────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Pleiades Chat Client — 多轮对话",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  python chat.py
  python chat.py --url http://127.0.0.1:8080
  python chat.py -s "你是一个简洁的助手" --no-stream

特殊命令:
  /exit     退出对话
  /clear    清空对话历史（保留 system prompt）
  /system   显示当前 system prompt
        """,
    )
    parser.add_argument(
        "--url", default="http://127.0.0.1:8080",
        help="Pleiades API 地址 (默认: http://127.0.0.1:8080)",
    )
    parser.add_argument(
        "--model", default=None,
        help="模型名 (默认从 /v1/models 自动获取)",
    )
    parser.add_argument(
        "-s", "--system", default=None,
        help="System prompt (放置在对话最前，/clear 不会清除)",
    )
    parser.add_argument(
        "--no-stream", action="store_true",
        help="禁用流式输出 (默认启用 SSE 流式)",
    )
    parser.add_argument(
        "--max-tokens", type=int, default=256,
        help="最大生成 token 数 (默认: 256)",
    )
    args = parser.parse_args()

    base_url = args.url.rstrip("/")
    chat_url = f"{base_url}/v1/chat/completions"

    # 获取模型名
    model = args.model or get_model(base_url)
    if not model:
        print("[错误] 无法确定模型名，请用 --model 指定", file=sys.stderr)
        sys.exit(1)
    print(f"模型: {model}  地址: {base_url}  流式: {'✓' if not args.no_stream else '✗'}")

    # 初始化消息历史
    messages: list[dict[str, str]] = []
    if args.system:
        messages.append({"role": "system", "content": args.system})
        print(f"System: {args.system}")

    stream = not args.no_stream

    print('输入消息开始对话，/exit 退出，/clear 清空历史\n')

    while True:
        try:
            user_input = input("> ").strip()
        except (EOFError, KeyboardInterrupt):
            print()
            break

        if not user_input:
            continue

        # 特殊命令
        if user_input == "/exit":
            break
        elif user_input == "/clear":
            # 保留 system prompt
            messages = [m for m in messages if m["role"] == "system"]
            print("[历史已清空]")
            continue
        elif user_input == "/system":
            sys_msg = next((m["content"] for m in messages if m["role"] == "system"), None)
            print(f"System: {sys_msg or '(无)'}")
            continue

        # 添加用户消息
        messages.append({"role": "user", "content": user_input})

        # 构造请求
        payload = json.dumps({
            "model": model,
            "messages": messages,
            "stream": stream,
            "max_tokens": args.max_tokens,
        }).encode("utf-8")

        # 发送请求
        try:
            if stream:
                content = stream_chat(chat_url, payload)
            else:
                content = nonstream_chat(chat_url, payload)

            if content:
                messages.append({"role": "assistant", "content": content})
        except urllib.error.URLError as e:
            print(f"\n[错误] 连接失败: {e.reason}", file=sys.stderr)
            messages.pop()  # 移除失败的用户消息
        except Exception as e:
            print(f"\n[错误] {e}", file=sys.stderr)
            messages.pop()

    print("再见。")


if __name__ == "__main__":
    main()
