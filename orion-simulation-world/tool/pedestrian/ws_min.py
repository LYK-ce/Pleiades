#!/usr/bin/env python3
# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# 标准库 RFC6455 最小 WebSocket 客户端（task_35 §二 Step 4 / COMPONENTS.md §8.1）。
#
# 为什么要有它：`websockets` 是**可选**依赖。没装时不允许静默失败，也不允许直接报
#   "ImportError" 了事 —— 这里用 socket + hashlib + struct 手写一个够用的客户端，
#   只覆盖世界侧真正用到的子集：
#     · 客户端 -> 服务端：文本帧、**必须掩码**（不掩码服务端按 1002 断开）
#     · 服务端 -> 客户端：文本帧（Ping/Pong/Close/分片在这里就地消化）
#     · 二进制帧：本 step 不消费，直接丢弃（世界只对带 manifest 的连接下发二进制）
#
# 明确**不支持**（够用即可，不写没用的代码）：
#   · wss://（TLS）—— 世界是本地 ws://，需要 TLS 请 `pip install websockets`
#   · 扩展协商（permessage-deflate 一律不请求）、子协议、HTTP 代理
#
# 不读地图、不连 P2P、不做任何业务判断：只回答「怎么收发一帧文本」。

import base64
import hashlib
import os
import select
import socket
import struct
import time
from urllib.parse import urlsplit

# ---------------------------------------------------------------- 常量
## RFC6455 §1.3 的握手 GUID（拼在 Sec-WebSocket-Key 后面做 SHA-1）。
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

OP_CONTINUATION = 0x0
OP_TEXT = 0x1
OP_BINARY = 0x2
OP_CLOSE = 0x8
OP_PING = 0x9
OP_PONG = 0xA

## 单帧上限（字节）：与世界侧 ComponentLink.MAX_FRAME_BUFFER_BYTES 同量级，
## 只防「对端发疯把我们撑爆」，正常文本帧都是几百字节。
MAX_FRAME_BYTES = 4 * 1024 * 1024
## 握手响应头上限（字节）：正常 < 300 B，超过这个数说明对端不是 WS。
MAX_HANDSHAKE_BYTES = 64 * 1024


class WsClosed(RuntimeError):
    """对端关闭 / 连接断开 / 协议错误。`code` 是 WS 关闭码（1000 正常）。"""

    def __init__(self, code: int, reason: str = ""):
        super().__init__("ws closed code=%d reason=%r" % (code, reason))
        self.code = int(code)
        self.reason = str(reason)


def parse_ws_url(url: str) -> tuple:
    """`ws://host:port/path` -> (host, port, path)。wss:// / 其它协议一律 ValueError。"""
    parts = urlsplit(url)
    scheme = (parts.scheme or "").lower()
    if scheme == "wss":
        raise ValueError(
            "本模块不支持 wss://（TLS）：请 `pip install websockets` 后用 --engine websockets")
    if scheme != "ws":
        raise ValueError("只支持 ws:// 开头的 URL，收到 %r" % url)
    host = parts.hostname or "127.0.0.1"
    port = int(parts.port or 80)
    path = parts.path or "/"
    if parts.query:
        path += "?" + parts.query
    return host, port, path


def build_frame(opcode: int, payload: bytes, fin: bool = True) -> bytes:
    """拼一帧**客户端帧**（FIN + 必掩码）。长度分支 7 位 / 126+u16 / 127+u64。"""
    header = bytearray()
    header.append((0x80 if fin else 0x00) | (opcode & 0x0F))
    size = len(payload)
    if size < 126:
        header.append(0x80 | size)
    elif size < 65536:
        header.append(0x80 | 126)
        header += struct.pack("!H", size)
    else:
        header.append(0x80 | 127)
        header += struct.pack("!Q", size)
    mask = os.urandom(4)
    header += mask
    # 掩码：payload[i] ^ mask[i % 4]（RFC6455 §5.3）
    masked = bytes(payload[i] ^ mask[i & 3] for i in range(size))
    return bytes(header) + masked


class MiniWebSocket:
    """够用的 RFC6455 客户端：connect / send_text / recv_text / close。

    用法（调用方按 30 Hz 轮询，recv 超时返回 None 而不是抛异常）：

        ws = MiniWebSocket("ws://127.0.0.1:9999", open_timeout=8.0)
        ws.connect()
        ws.send_text('{"msg":"motion_cmd","seq":1,"v":1.0}')
        line = ws.recv_text(timeout=0.0)     # None = 这段时间没有文本帧
        ws.close()
    """

    def __init__(self, url: str, open_timeout: float = 8.0):
        self.url = url
        self.open_timeout = float(open_timeout)
        self._sock = None                 # type: socket.socket | None
        self._buf = bytearray()           # 已收未解析的字节
        self._frag = bytearray()          # 分片重组缓冲
        self._frag_opcode = 0             # 0 = 当前无待续分片
        self._close_sent = False
        self._closed = False
        self._bytes_in = 0
        self._bytes_out = 0

    # ------------------------------------------------------------ 连接
    def connect(self) -> None:
        """TCP 连接 + HTTP/1.1 Upgrade 握手（校验 101 与 Sec-WebSocket-Accept）。"""
        host, port, path = parse_ws_url(self.url)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        # ConnectionRefusedError / socket.timeout 原样抛给调用方（CLI 映射退出码）
        sock = socket.create_connection((host, port), timeout=self.open_timeout)
        try:
            sock.settimeout(self.open_timeout)
            request = (
                "GET %s HTTP/1.1\r\n"
                "Host: %s:%d\r\n"
                "Upgrade: websocket\r\n"
                "Connection: Upgrade\r\n"
                "Sec-WebSocket-Key: %s\r\n"
                "Sec-WebSocket-Version: 13\r\n"
                "\r\n" % (path, host, port, key)
            )
            sock.sendall(request.encode("ascii"))
            head, extra = self._read_handshake(sock)
            self._check_handshake(head, key)
            self._buf = bytearray(extra)
            sock.setblocking(False)       # 之后一律 select + 非阻塞 recv
            self._sock = sock
        except BaseException:
            try:
                sock.close()
            except OSError:
                pass
            raise

    @staticmethod
    def _read_handshake(sock) -> tuple:
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = sock.recv(4096)
            if not chunk:
                raise WsClosed(1006, "握手中对端关闭（世界在跑吗？端口对吗？）")
            data += chunk
            if len(data) > MAX_HANDSHAKE_BYTES:
                raise WsClosed(1002, "握手响应头超过 %d 字节" % MAX_HANDSHAKE_BYTES)
        head, _, extra = data.partition(b"\r\n\r\n")
        return head.decode("iso-8859-1", errors="replace"), extra

    @staticmethod
    def _check_handshake(head: str, key: str) -> None:
        lines = head.split("\r\n")
        status = lines[0] if lines else ""
        parts = status.split()
        if len(parts) < 2 or parts[0] != "HTTP/1.1" or parts[1] != "101":
            raise WsClosed(1002, "握手失败：%r" % status)
        headers = {}
        for line in lines[1:]:
            name, sep, value = line.partition(":")
            if sep:
                headers[name.strip().lower()] = value.strip()
        expect = base64.b64encode(
            hashlib.sha1((key + WS_GUID).encode("ascii")).digest()).decode("ascii")
        got = headers.get("sec-websocket-accept", "")
        if got != expect:
            raise WsClosed(1002, "Sec-WebSocket-Accept 不匹配（got=%r expect=%r）" % (got, expect))

    # ------------------------------------------------------------ 发送
    def send_text(self, text: str) -> None:
        self._send_frame(OP_TEXT, text.encode("utf-8"))

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        if self._closed or self._sock is None:
            raise WsClosed(1006, "send on closed socket")
        frame = build_frame(opcode, payload)
        try:
            self._sock.setblocking(True)
            self._sock.settimeout(self.open_timeout)
            self._sock.sendall(frame)        # 长帧一次 sendall，别自己切
        except OSError as exc:
            self._closed = True
            raise WsClosed(1006, "发送失败：%s" % exc) from exc
        finally:
            if self._sock is not None:
                try:
                    self._sock.setblocking(False)
                except OSError:
                    pass
        self._bytes_out += len(frame)

    # ------------------------------------------------------------ 接收
    def recv_text(self, timeout: float = 0.0):
        """返回一帧**完整**文本；`timeout` 内没有（timeout=0 = 只看已有缓冲）返回 None。

        控制帧与分片在本函数内消化：Ping 自动回 Pong、Close 回 Close 后抛 WsClosed、
        二进制帧丢弃后继续等下一帧。
        """
        deadline = time.monotonic() + max(float(timeout), 0.0)
        first = True
        while True:
            while True:
                frame = self._extract_frame()
                if frame is None:
                    break
                text = self._handle_frame(*frame)
                if text is not None:
                    return text
            # 缓冲里没有完整帧了：等新字节（首轮即使 timeout=0 也要试一次非阻塞读）
            remain = deadline - time.monotonic()
            if not first and remain <= 0.0:
                return None
            first = False
            if not self._fill(remain if remain > 0.0 else 0.0):
                return None

    def _fill(self, wait_s: float) -> bool:
        """等最多 wait_s 秒并读一段字节；读到字节返回 True，超时/EOF 返回 False。"""
        if self._closed or self._sock is None:
            raise WsClosed(1006, "recv on closed socket")
        try:
            readable, _, _ = select.select([self._sock], [], [], wait_s)
        except (OSError, ValueError) as exc:
            self._closed = True
            raise WsClosed(1006, "select 失败：%s" % exc) from exc
        if not readable:
            return False
        try:
            chunk = self._sock.recv(65536)
        except (BlockingIOError, InterruptedError):
            return False
        except OSError as exc:
            self._closed = True
            raise WsClosed(1006, "recv 失败：%s" % exc) from exc
        if not chunk:
            self._closed = True
            raise WsClosed(1006, "对端关闭 TCP（世界退了？）")
        self._buf += chunk
        self._bytes_in += len(chunk)
        return True

    def _extract_frame(self):
        """从缓冲切出**一帧** -> (fin, opcode, payload)；不足一帧返回 None（不消费）。"""
        buf = self._buf
        if len(buf) < 2:
            return None
        b1, b2 = buf[0], buf[1]
        if b1 & 0x70:
            raise WsClosed(1002, "RSV 位非零（本模块不协商扩展）")
        fin = bool(b1 & 0x80)
        opcode = b1 & 0x0F
        masked = bool(b2 & 0x80)
        length = b2 & 0x7F
        offset = 2
        if length == 126:
            if len(buf) < offset + 2:
                return None
            length = struct.unpack_from("!H", buf, offset)[0]
            offset += 2
        elif length == 127:
            if len(buf) < offset + 8:
                return None
            length = struct.unpack_from("!Q", buf, offset)[0]
            offset += 8
        if length > MAX_FRAME_BYTES:
            raise WsClosed(1009, "帧长度 %d 超过上限 %d" % (length, MAX_FRAME_BYTES))
        mask = b""
        if masked:
            if len(buf) < offset + 4:
                return None
            mask = bytes(buf[offset:offset + 4])
            offset += 4
        if len(buf) < offset + length:
            return None
        payload = bytearray(buf[offset:offset + length])
        if masked:
            # 服务端帧不应掩码；真带了也照解（宽容，不因此断开）
            for i in range(length):
                payload[i] ^= mask[i & 3]
        del buf[:offset + length]
        return fin, opcode, bytes(payload)

    def _handle_frame(self, fin: bool, opcode: int, payload: bytes):
        """消化一帧：返回文本 str，或 None（控制帧 / 未完成分片 / 二进制）。"""
        if opcode == OP_PING:
            if len(payload) > 125:
                raise WsClosed(1002, "Ping payload > 125 字节")
            self._send_frame(OP_PONG, payload)     # 原样回 payload（世界 10 s 一次心跳）
            return None
        if opcode == OP_PONG:
            return None
        if opcode == OP_CLOSE:
            code, reason = 1005, ""
            if len(payload) >= 2:
                code = struct.unpack_from("!H", payload, 0)[0]
                reason = payload[2:].decode("utf-8", errors="replace")
            if not self._close_sent:
                self._close_sent = True
                try:
                    self._send_frame(OP_CLOSE, payload[:125])
                except WsClosed:
                    pass
            self._closed = True
            raise WsClosed(code, reason)
        if opcode == OP_TEXT or opcode == OP_BINARY:
            if not fin:
                self._frag_opcode = opcode
                self._frag = bytearray(payload)
                return None
            if opcode == OP_BINARY:
                return None                        # 本 step 不消费二进制：丢弃
            return payload.decode("utf-8", errors="replace")
        if opcode == OP_CONTINUATION:
            if self._frag_opcode == 0:
                raise WsClosed(1002, "收到没有起始帧的 continuation")
            self._frag += payload
            if len(self._frag) > MAX_FRAME_BYTES:
                raise WsClosed(1009, "分片重组超过上限")
            if not fin:
                return None
            opcode = self._frag_opcode
            data = bytes(self._frag)
            self._frag_opcode = 0
            self._frag = bytearray()
            if opcode == OP_BINARY:
                return None
            return data.decode("utf-8", errors="replace")
        raise WsClosed(1002, "未知 opcode 0x%X" % opcode)

    # ------------------------------------------------------------ 关闭
    def close(self, code: int = 1000, reason: str = "") -> None:
        """发 Close（幂等）再关 socket；任何异常都吞掉（收尾路径不许再抛）。"""
        if self._close_sent and self._closed:
            return
        if self._sock is not None and not self._close_sent:
            self._close_sent = True
            try:
                payload = struct.pack("!H", int(code) & 0xFFFF) + reason.encode("utf-8")[:123]
                self._send_frame(OP_CLOSE, payload)
            except (WsClosed, OSError):
                pass
        self._closed = True
        if self._sock is not None:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None

    def __enter__(self):
        self.connect()
        return self

    def __exit__(self, exc_type, exc, tb):
        self.close()
        return False
