#!/usr/bin/env python3
"""Live-smoke the Codex Chrome extension through the native host bridge.

This live smoke starts a temporary browser profile, loads the official Codex
Chrome extension, waits for the native host socket, then proves the native
messaging host used by the running browser can bridge:

- client -> native host -> extension -> client with `getInfo`
- client -> native host -> extension -> client with `getTabs`
- sky-cua MCP -> daemon -> native host -> extension -> MCP with `browser_list_tabs`
- extension -> native host -> client with the heartbeat `ping`
- session-log completion -> native host -> extension with `turnEnded`
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.server
import json
import math
import re
import secrets
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager, suppress
from datetime import UTC, datetime
from pathlib import Path
from typing import cast
from urllib.parse import urlparse

from _chrome_bridge import (
    DEFAULT_EXTENSION_ID,
    DEFAULT_HOST_PATH,
    ManifestRestore,
    browser_command,
    default_extension_dir,
    host_pid_from_socket,
    install_temp_manifest,
    launch_browser,
    restore_manifest,
    terminate_browser,
    wait_for_devtools_port,
    wait_for_extension_target,
    wait_for_host_process,
    wait_for_socket,
)
from _mcp_stdio import McpClient, stop_service_processes_for_socket

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MCP_CLIENT_PATH = REPO_ROOT / "target/debug/sky-cua-client"
SMOKE_SESSION_ID = "smoke-session"
SMOKE_TURN_ID = "smoke-turn"
MCP_BROWSER_SESSION_ID = "sky-cua-mcp"
MCP_BROWSER_TURN_ID = "browser-list-tabs"
MCP_READ_TIMEOUT_SECONDS = 15.0
TURN_ENDED_ID = f"native-turn-ended:{SMOKE_SESSION_ID}:{SMOKE_TURN_ID}"
CDP_COUNTER = 0
NATIVE_COUNTER = 0
CURSOR_TARGET_CSS_X = 240
CURSOR_TARGET_CSS_Y = 160
CURSOR_DIFF_RADIUS_CSS = 72
CURSOR_OUTSIDE_COMPONENT_MAX_CSS = 96
CURSOR_FIXTURE_HTML = b"""<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <title>sky-cua cursor proof</title>
    <style>
      html, body {
        background: white;
        height: 100%;
        margin: 0;
        overflow: hidden;
      }
    </style>
  </head>
  <body></body>
</html>
"""


def connected_components(points: set[tuple[int, int]]) -> list[dict[str, int]]:
    components: list[dict[str, int]] = []
    remaining = set(points)
    while remaining:
        start = remaining.pop()
        stack = [start]
        pixels = 0
        min_x = start[0]
        min_y = start[1]
        max_x = start[0]
        max_y = start[1]
        while stack:
            x, y = stack.pop()
            pixels += 1
            min_x = min(min_x, x)
            min_y = min(min_y, y)
            max_x = max(max_x, x)
            max_y = max(max_y, y)
            for next_y in range(y - 1, y + 2):
                for next_x in range(x - 1, x + 2):
                    neighbor = (next_x, next_y)
                    if neighbor in remaining:
                        remaining.remove(neighbor)
                        stack.append(neighbor)
        components.append(
            {
                "pixels": pixels,
                "x": min_x,
                "y": min_y,
                "width": max_x - min_x + 1,
                "height": max_y - min_y + 1,
            }
        )
    components.sort(key=lambda component: component["pixels"], reverse=True)
    return components


class DevToolsWebSocket:
    def __init__(self, url: str) -> None:
        parsed = urlparse(url)
        if parsed.scheme != "ws" or parsed.hostname is None or parsed.port is None:
            raise ValueError(f"unsupported websocket URL: {url}")
        self._path = parsed.path
        if parsed.query:
            self._path += f"?{parsed.query}"
        self._sock = socket.create_connection((parsed.hostname, parsed.port), timeout=5)
        self._handshake(parsed.hostname, parsed.port)

    def __enter__(self) -> DevToolsWebSocket:
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def _handshake(self, host: str, port: int) -> None:
        key = base64.b64encode(secrets.token_bytes(16)).decode("ascii")
        request = (
            f"GET {self._path} HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "Origin: http://127.0.0.1\r\n"
            "\r\n"
        )
        self._sock.sendall(request.encode("ascii"))
        response = self._read_until(b"\r\n\r\n")
        if not response.startswith(b"HTTP/1.1 101"):
            raise RuntimeError(f"DevTools websocket handshake failed: {response!r}")
        accept = base64.b64encode(
            hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
        ).decode("ascii")
        if accept.encode("ascii") not in response:
            raise RuntimeError("DevTools websocket handshake did not echo the expected accept key")

    def _read_until(self, marker: bytes) -> bytes:
        data = b""
        while marker not in data:
            chunk = self._sock.recv(4096)
            if not chunk:
                raise RuntimeError("unexpected EOF during websocket handshake")
            data += chunk
        return data

    def send_json(self, message: dict[str, object]) -> None:
        payload = json.dumps(message, separators=(",", ":")).encode("utf-8")
        header = bytearray([0x81])
        if len(payload) < 126:
            header.append(0x80 | len(payload))
        elif len(payload) <= 0xFFFF:
            header.extend([0x80 | 126, *struct.pack("!H", len(payload))])
        else:
            header.extend([0x80 | 127, *struct.pack("!Q", len(payload))])
        mask = secrets.token_bytes(4)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self._sock.sendall(bytes(header) + mask + masked)

    def recv_json(self) -> dict[str, object]:
        while True:
            first, second = self._recv_exact(2)
            opcode = first & 0x0F
            masked = bool(second & 0x80)
            length = second & 0x7F
            if length == 126:
                length = struct.unpack("!H", self._recv_exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._recv_exact(8))[0]
            mask = self._recv_exact(4) if masked else b""
            payload = self._recv_exact(length)
            if masked:
                payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
            if opcode == 0x8:
                raise RuntimeError("DevTools websocket closed")
            if opcode == 0x9:
                self._send_pong(payload)
                continue
            if opcode == 0x1:
                value = json.loads(payload.decode("utf-8"))
                if isinstance(value, dict):
                    return value
                raise RuntimeError(f"unexpected DevTools payload: {value!r}")

    def _send_pong(self, payload: bytes) -> None:
        header = bytearray([0x8A])
        header.append(0x80 | len(payload))
        mask = secrets.token_bytes(4)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self._sock.sendall(bytes(header) + mask + masked)

    def _recv_exact(self, length: int) -> bytes:
        data = b""
        while len(data) < length:
            chunk = self._sock.recv(length - len(data))
            if not chunk:
                raise RuntimeError("unexpected EOF from DevTools websocket")
            data += chunk
        return data

    def close(self) -> None:
        with suppress(OSError):
            self._sock.close()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", choices=["auto", "brave", "chromium"], default="auto")
    parser.add_argument("--extension-id", default=DEFAULT_EXTENSION_ID)
    parser.add_argument("--extension-dir", type=Path, default=default_extension_dir())
    parser.add_argument(
        "--host-path",
        type=Path,
        default=DEFAULT_HOST_PATH,
        help="sky-cua-chrome-host binary to install temporarily for this smoke.",
    )
    parser.add_argument(
        "--mcp-client-path",
        type=Path,
        default=DEFAULT_MCP_CLIENT_PATH,
        help="sky-cua-client binary to use for the optional MCP browser_list_tabs proof.",
    )
    parser.add_argument(
        "--install-temp-native-manifest",
        action="store_true",
        help="Temporarily point the selected browser native manifest at --host-path, then restore it.",
    )
    parser.add_argument(
        "--mcp-list-tabs-proof",
        action="store_true",
        help="Also prove sky-cua MCP browser_list_tabs against the launched browser socket.",
    )
    parser.add_argument(
        "--skip-turn-ended-proof",
        action="store_true",
        help="Skip the session-log completion proof for turnEnded.",
    )
    parser.add_argument(
        "--skip-cursor-proof",
        action="store_true",
        help="Skip the browser agent cursor overlay proof.",
    )
    parser.add_argument(
        "--artifacts-root",
        type=Path,
        default=Path("artifacts/chrome-host-smoke"),
    )
    parser.add_argument(
        "--hacker-news-proof",
        action="store_true",
        help=(
            "Visit Hacker News in the launched browser, open the top 3 external stories, "
            "save website screenshots, and write a markdown proof with the first 3 comments."
        ),
    )
    parser.add_argument("--keep-browser-open", action="store_true")
    return parser.parse_args()


def read_native_frame(sock: socket.socket, timeout: float = 10) -> dict[str, object]:
    sock.settimeout(timeout)
    header = recv_exact(sock, 4)
    length = struct.unpack("@I", header)[0]
    payload = recv_exact(sock, length)
    value = json.loads(payload.decode("utf-8"))
    if not isinstance(value, dict):
        raise RuntimeError(f"unexpected native frame payload: {value!r}")
    return value


def recv_exact(sock: socket.socket, length: int) -> bytes:
    data = b""
    while len(data) < length:
        chunk = sock.recv(length - len(data))
        if not chunk:
            raise RuntimeError("unexpected EOF from native socket")
        data += chunk
    return data


def write_native_frame(sock: socket.socket, message: dict[str, object]) -> None:
    payload = json.dumps(message, separators=(",", ":")).encode("utf-8")
    sock.sendall(struct.pack("@I", len(payload)) + payload)


def turn_ended_response_from_stderr(stderr: str) -> dict[str, object] | None:
    marker = f"received unmatched Chrome response id={TURN_ENDED_ID} payload="
    for line in stderr.splitlines():
        if marker not in line:
            continue
        payload_text = line.split(marker, 1)[1].strip()
        try:
            payload = json.loads(payload_text)
        except json.JSONDecodeError:
            return None
        if isinstance(payload, dict):
            return payload
        return None
    return None


def turn_ended_response_was_successful(response: dict[str, object] | None) -> bool:
    return (
        isinstance(response, dict)
        and response.get("id") == TURN_ENDED_ID
        and "error" not in response
    )


def native_request(
    sock: socket.socket,
    method: str,
    params: dict[str, object],
    *,
    timeout: float = 15,
    request_id: str | None = None,
) -> dict[str, object]:
    global NATIVE_COUNTER
    NATIVE_COUNTER += 1
    message_id = request_id or f"client-{method}-{NATIVE_COUNTER}"
    write_native_frame(
        sock,
        {
            "jsonrpc": "2.0",
            "id": message_id,
            "method": method,
            "params": params,
        },
    )
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(
                f"native request {method} id={message_id} timed out after {timeout:g}s"
            )
        try:
            response = read_native_frame(sock, timeout=remaining)
        except TimeoutError:
            raise TimeoutError(
                f"native request {method} id={message_id} timed out after {timeout:g}s"
            ) from None
        if response.get("id") == message_id:
            if "error" in response:
                raise RuntimeError(f"native request {method} failed: {response['error']!r}")
            return response
        if response.get("method") == "ping":
            write_native_frame(
                sock,
                {
                    "jsonrpc": "2.0",
                    "id": response.get("id"),
                    "result": "pong",
                },
            )
            continue
        if isinstance(response.get("method"), str):
            continue
        raise RuntimeError(f"unexpected native frame while waiting for {message_id}: {response!r}")


def native_result(response: dict[str, object]) -> object:
    if "result" not in response:
        raise RuntimeError(f"native response had no result: {response!r}")
    return response["result"]


def tab_id_from_payload(tab: object) -> str | None:
    if not isinstance(tab, dict):
        return None
    for key in ("tab_id", "tabId", "id"):
        value = tab.get(key)
        if isinstance(value, (str, int)):
            return str(value)
    return None


def expected_tab_present(tabs: list[object], expected_tab_id: int | None) -> bool | None:
    if expected_tab_id is None:
        return None
    expected = str(expected_tab_id)
    return any(tab_id_from_payload(tab) == expected for tab in tabs)


def tab_list_result(response: dict[str, object]) -> list[object]:
    result = response.get("result")
    if isinstance(result, list):
        return result
    if isinstance(result, dict):
        tabs = result.get("tabs")
        if isinstance(tabs, list):
            return tabs
    return []


def redacted_tab_list_proof(
    response: dict[str, object],
    *,
    expected_tab_id: int | None,
) -> dict[str, object]:
    tabs = tab_list_result(response)
    return {
        "id": response.get("id"),
        "has_result": "result" in response,
        "expected_tab_id": expected_tab_id,
        "expected_tab_present": expected_tab_present(tabs, expected_tab_id),
        "tabs_count": len(tabs),
    }


def run_mcp_list_tabs_proof(
    *,
    client_path: Path,
    socket_dir: Path,
    service_socket_path: Path,
    expected_tab_id: int | None,
) -> dict[str, object]:
    client_path = client_path.expanduser().resolve()
    if not client_path.exists():
        raise FileNotFoundError(f"MCP client binary not found: {client_path}")

    client = McpClient(
        [str(client_path), "mcp"],
        extra_env={
            "CODEX_BROWSER_USE_SOCKET_DIR": str(socket_dir),
            "SKY_CUA_BROWSER_USE_SOCKET_DIR": str(socket_dir),
            "SKY_CUA_REPO_ROOT": str(REPO_ROOT),
            "SKY_CUA_SERVICE_SOCKET_PATH": str(service_socket_path),
        },
        read_timeout=MCP_READ_TIMEOUT_SECONDS,
        client_name="live-chrome-host-smoke",
    )
    try:
        client.initialize()
        tools = client.tools_list()
        tool_names: list[str] = []
        for tool in tools:
            name = tool.get("name")
            if isinstance(name, str):
                tool_names.append(name)
        tool_names.sort()
        result = client.tools_call(
            3,
            "list_resources",
            {"surface": "browser", "resource": "tabs", "target": "user_chrome"},
        )
    finally:
        try:
            client.close()
        finally:
            stop_service_processes_for_socket(service_socket_path)

    if "list_resources" not in tool_names:
        raise RuntimeError(f"list_resources was not advertised by tools/list: {tool_names!r}")
    structured = result.get("structuredContent")
    if not isinstance(structured, dict):
        raise RuntimeError(f"list_resources returned no structuredContent: {result!r}")
    grouped_result = structured.get("result")
    if not isinstance(grouped_result, dict):
        raise RuntimeError(f"list_resources returned invalid grouped payload: {structured!r}")
    structured = grouped_result
    tabs = structured.get("tabs")
    if not isinstance(tabs, list):
        raise RuntimeError(f"list_resources returned invalid tabs payload: {structured!r}")
    diagnostics = structured.get("diagnostics", [])
    if not isinstance(diagnostics, list):
        raise RuntimeError(f"list_resources returned invalid diagnostics payload: {structured!r}")
    if diagnostics:
        raise RuntimeError(f"browser_list_tabs reported diagnostics: {diagnostics!r}")
    if not tabs:
        raise RuntimeError("browser_list_tabs returned no tabs from the live browser socket")
    found_expected_tab = expected_tab_present(tabs, expected_tab_id)
    if expected_tab_id is not None and not found_expected_tab:
        raise RuntimeError(
            "browser_list_tabs did not return the expected live tab id "
            f"{expected_tab_id}; tabs_count={len(tabs)}"
        )

    return {
        "client_path": str(client_path),
        "tool_listed": True,
        "expected_tab_id": expected_tab_id,
        "expected_tab_present": found_expected_tab,
        "tabs_count": len(tabs),
        "diagnostics": diagnostics,
    }


class CursorFixtureHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(CURSOR_FIXTURE_HTML)))
        self.end_headers()
        self.wfile.write(CURSOR_FIXTURE_HTML)

    def log_message(self, format: str, *_args: object) -> None:
        del format, _args
        return


@contextmanager
def cursor_fixture_url() -> Iterator[str]:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), CursorFixtureHandler)
    thread = threading.Thread(target=server.serve_forever, name="cursor-fixture-http", daemon=True)
    thread.start()
    try:
        host, port = cast(tuple[str, int], server.server_address)
        yield f"http://{host}:{port}/cursor-proof.html"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def cursor_diff_stats(
    before_path: Path,
    after_path: Path,
    *,
    target_x_css: float,
    target_y_css: float,
    device_pixel_ratio: float,
    radius_css: float = CURSOR_DIFF_RADIUS_CSS,
    threshold: int = 24,
) -> dict[str, object]:
    if device_pixel_ratio <= 0:
        raise ValueError(f"invalid device pixel ratio: {device_pixel_ratio}")
    from PIL import Image

    with Image.open(before_path) as before_image, Image.open(after_path) as after_image:
        before = before_image.convert("RGB")
        after = after_image.convert("RGB")

    if before.size != after.size:
        raise AssertionError(
            f"cursor proof screenshots have different sizes: {before.size} != {after.size}"
        )

    width, height = before.size
    target_x = round(target_x_css * device_pixel_ratio)
    target_y = round(target_y_css * device_pixel_ratio)
    radius = max(1, math.ceil(radius_css * device_pixel_ratio))
    radius_squared = radius * radius
    changed = 0
    near_changed = 0
    outside_changed = 0
    outside_points: set[tuple[int, int]] = set()
    min_x = width
    min_y = height
    max_x = -1
    max_y = -1

    before_pixels = before.load()
    after_pixels = after.load()
    if before_pixels is None or after_pixels is None:
        raise AssertionError("cursor proof screenshots could not be loaded as RGB pixels")
    for y in range(height):
        for x in range(width):
            before_pixel = cast(tuple[int, int, int], before_pixels[x, y])
            after_pixel = cast(tuple[int, int, int], after_pixels[x, y])
            delta = sum(
                abs(int(after_pixel[channel]) - int(before_pixel[channel])) for channel in range(3)
            )
            if delta <= threshold:
                continue
            changed += 1
            min_x = min(min_x, x)
            min_y = min(min_y, y)
            max_x = max(max_x, x)
            max_y = max(max_y, y)
            if (x - target_x) ** 2 + (y - target_y) ** 2 <= radius_squared:
                near_changed += 1
            else:
                outside_changed += 1
                outside_points.add((x, y))

    bounds = None
    if changed:
        bounds = {
            "x": min_x,
            "y": min_y,
            "width": max_x - min_x + 1,
            "height": max_y - min_y + 1,
        }

    return {
        "changed_pixels": changed,
        "near_changed_pixels": near_changed,
        "outside_changed_pixels": outside_changed,
        "outside_components": connected_components(outside_points),
        "bounds": bounds,
        "device_pixel_ratio": device_pixel_ratio,
        "target_css": {"x": target_x_css, "y": target_y_css},
        "target_pixel": {"x": target_x, "y": target_y},
        "radius_pixels": radius,
        "screenshot_size": {"width": width, "height": height},
    }


def assert_localized_cursor_diff(
    before_path: Path,
    after_path: Path,
    *,
    target_x_css: float,
    target_y_css: float,
    device_pixel_ratio: float,
    min_near_changed_pixels: int = 25,
    max_outside_changed_pixels: int = 500,
    max_outside_changed_ratio: float = 0.30,
    max_outside_components: int = 1,
    max_outside_component_pixels: int = 900,
    max_outside_component_size_css: float = CURSOR_OUTSIDE_COMPONENT_MAX_CSS,
) -> dict[str, object]:
    stats = cursor_diff_stats(
        before_path,
        after_path,
        target_x_css=target_x_css,
        target_y_css=target_y_css,
        device_pixel_ratio=device_pixel_ratio,
    )
    changed = cast(int, stats["changed_pixels"])
    near_changed = cast(int, stats["near_changed_pixels"])
    outside_changed = cast(int, stats["outside_changed_pixels"])
    if near_changed < min_near_changed_pixels:
        raise AssertionError(
            "cursor proof did not find enough changed pixels near the requested point: "
            f"{near_changed} < {min_near_changed_pixels}; stats={stats}"
        )
    outside_limit = min(
        max_outside_changed_pixels,
        max(1, math.ceil(changed * max_outside_changed_ratio)),
    )
    if outside_changed > outside_limit:
        outside_components = cast(list[dict[str, int]], stats["outside_components"])
        outside_size_limit = max(1, math.ceil(max_outside_component_size_css * device_pixel_ratio))
        outside_component_pixel_limit = max(
            max_outside_component_pixels,
            outside_size_limit * outside_size_limit,
        )
        outside_components_fit_cursor_move = len(
            outside_components
        ) <= max_outside_components and all(
            component["pixels"] <= outside_component_pixel_limit
            and component["width"] <= outside_size_limit
            and component["height"] <= outside_size_limit
            for component in outside_components
        )
        if outside_components_fit_cursor_move:
            return {"ok": True, **stats}
        raise AssertionError(
            "cursor proof changed too many pixels outside the cursor region: "
            f"{outside_changed} > {outside_limit}; stats={stats}"
        )
    return {"ok": True, **stats}


def read_heartbeat(sock: socket.socket, timeout: float = 10) -> tuple[dict[str, object], bool]:
    while True:
        message = read_native_frame(sock, timeout=timeout)
        if message.get("method") != "ping":
            if isinstance(message.get("method"), str):
                continue
            raise RuntimeError(f"unexpected native frame while waiting for heartbeat: {message!r}")
        write_native_frame(
            sock,
            {
                "jsonrpc": "2.0",
                "id": message.get("id"),
                "result": "pong",
            },
        )
        return message, True


def cdp_call(
    websocket_url: str, method: str, params: dict[str, object] | None = None
) -> dict[str, object]:
    global CDP_COUNTER
    CDP_COUNTER += 1
    message: dict[str, object] = {"id": CDP_COUNTER, "method": method}
    if params is not None:
        message["params"] = params
    with DevToolsWebSocket(websocket_url) as ws:
        ws.send_json(message)
        while True:
            response = ws.recv_json()
            if response.get("id") == message["id"]:
                if "error" in response:
                    raise RuntimeError(f"DevTools call failed: {response['error']!r}")
                return response


def evaluate(websocket_url: str, expression: str, timeout_ms: int = 5000) -> object:
    response = cdp_call(
        websocket_url,
        "Runtime.evaluate",
        {
            "expression": expression,
            "awaitPromise": True,
            "returnByValue": True,
            "timeout": timeout_ms,
        },
    )
    result = response.get("result")
    if not isinstance(result, dict):
        raise RuntimeError(f"unexpected Runtime.evaluate response: {response!r}")
    inner = result.get("result")
    if not isinstance(inner, dict):
        raise RuntimeError(f"unexpected Runtime.evaluate result: {response!r}")
    if "exceptionDetails" in result:
        raise RuntimeError(f"extension evaluation failed: {result['exceptionDetails']!r}")
    return inner.get("value")


def markdown_escape(value: object) -> str:
    return str(value).replace("\\", "\\\\").replace("[", "\\[").replace("]", "\\]")


def comment_excerpt(value: object, limit: int = 900) -> str:
    text = re.sub(r"\s+", " ", str(value)).strip()
    if len(text) <= limit:
        return text
    return text[: limit - 1].rstrip() + "…"


def story_filename(index: int, title: object) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", str(title).lower()).strip("-")
    if not slug:
        slug = "story"
    return f"story-{index:02d}-{slug[:48]}.png"


def require_story_list(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list):
        raise RuntimeError(f"unexpected Hacker News story list: {value!r}")
    stories = [story for story in value if isinstance(story, dict)]
    if len(stories) < 3:
        raise RuntimeError(f"Hacker News yielded fewer than 3 external stories: {value!r}")
    return stories[:3]


def require_comment_list(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list):
        raise RuntimeError(f"unexpected Hacker News comments list: {value!r}")
    return [comment for comment in value if isinstance(comment, dict)][:3]


def require_tab_id(value: object) -> int:
    if not isinstance(value, dict) or not isinstance(value.get("id"), int):
        raise RuntimeError(f"extension createTab returned no tab id: {value!r}")
    return value["id"]


def browser_session_params() -> dict[str, object]:
    return {
        "session_id": SMOKE_SESSION_ID,
        "turn_id": SMOKE_TURN_ID,
    }


def mcp_browser_session_params() -> dict[str, object]:
    return {
        "session_id": MCP_BROWSER_SESSION_ID,
        "turn_id": MCP_BROWSER_TURN_ID,
    }


def create_mcp_visible_tab(client: socket.socket) -> tuple[dict[str, object], int]:
    tab = native_result(
        native_request(
            client,
            "createTab",
            mcp_browser_session_params(),
            request_id="client-create-tab-mcp-proof",
        )
    )
    tab_id = require_tab_id(tab)
    return cast(dict[str, object], tab), tab_id


def create_attached_tab(client: socket.socket) -> tuple[dict[str, object], int]:
    tab = native_result(native_request(client, "createTab", browser_session_params()))
    tab_id = require_tab_id(tab)
    native_request(
        client,
        "attach",
        {
            **browser_session_params(),
            "tabId": tab_id,
        },
    )
    extension_execute_cdp(client, tab_id=tab_id, method="Page.enable")
    return cast(dict[str, object], tab), tab_id


def extension_execute_cdp(
    client: socket.socket,
    *,
    tab_id: int,
    method: str,
    command_params: dict[str, object] | None = None,
    timeout_ms: int = 10000,
) -> object:
    response = native_request(
        client,
        "executeCdp",
        {
            **browser_session_params(),
            "target": {"tabId": tab_id},
            "method": method,
            "commandParams": command_params or {},
            "timeoutMs": timeout_ms,
        },
        timeout=max(15, (timeout_ms / 1000) + 5),
    )
    return native_result(response)


def same_requested_origin(requested_url: str, settled_url: object) -> bool:
    if not isinstance(settled_url, str):
        return False
    requested = urlparse(requested_url)
    settled = urlparse(settled_url)
    if requested.hostname is None or settled.hostname is None:
        return requested_url == settled_url
    requested_host = requested.hostname.removeprefix("www.")
    settled_host = settled.hostname.removeprefix("www.")
    web_schemes = {"http", "https"}
    compatible_scheme = (
        requested.scheme == settled.scheme
        or {
            requested.scheme,
            settled.scheme,
        }
        <= web_schemes
    )
    return compatible_scheme and requested_host == settled_host


def extension_evaluate(
    client: socket.socket,
    *,
    tab_id: int,
    expression: str,
    timeout_ms: int = 5000,
) -> object:
    value = extension_execute_cdp(
        client,
        tab_id=tab_id,
        method="Runtime.evaluate",
        command_params={
            "expression": expression,
            "awaitPromise": True,
            "returnByValue": True,
            "timeout": timeout_ms,
        },
        timeout_ms=timeout_ms + 2000,
    )
    if not isinstance(value, dict):
        raise RuntimeError(f"unexpected extension Runtime.evaluate response: {value!r}")
    inner = value.get("result")
    if not isinstance(inner, dict):
        raise RuntimeError(f"unexpected extension Runtime.evaluate result: {value!r}")
    if "exceptionDetails" in value:
        raise RuntimeError(f"extension page evaluation failed: {value['exceptionDetails']!r}")
    return inner.get("value")


def extension_navigate_and_wait(
    client: socket.socket,
    *,
    tab_id: int,
    url: str,
    timeout_s: float = 20,
) -> dict[str, object]:
    navigate_result = extension_execute_cdp(
        client,
        tab_id=tab_id,
        method="Page.navigate",
        command_params={"url": url},
        timeout_ms=10000,
    )
    if isinstance(navigate_result, dict):
        error_text = navigate_result.get("errorText")
        if isinstance(error_text, str) and error_text:
            raise RuntimeError(f"extension navigation to {url} failed: {error_text}")
    deadline = time.time() + timeout_s
    last_error: Exception | None = None
    while time.time() < deadline:
        try:
            state = extension_evaluate(
                client,
                tab_id=tab_id,
                expression=(
                    "({readyState: document.readyState, href: location.href, "
                    "title: document.title})"
                ),
                timeout_ms=1000,
            )
            if (
                isinstance(state, dict)
                and state.get("readyState") in {"interactive", "complete"}
                and same_requested_origin(url, state.get("href"))
            ):
                return state
        except RuntimeError as exc:
            last_error = exc
        time.sleep(0.25)
    if last_error is not None:
        raise TimeoutError(
            f"extension-driven page did not settle at {url}: {last_error}"
        ) from last_error
    raise TimeoutError(f"extension-driven page did not settle at {url}")


def extension_capture_screenshot(client: socket.socket, *, tab_id: int, output_path: Path) -> None:
    extension_execute_cdp(
        client,
        tab_id=tab_id,
        method="Page.bringToFront",
        timeout_ms=5000,
    )
    value = extension_execute_cdp(
        client,
        tab_id=tab_id,
        method="Page.captureScreenshot",
        command_params={
            "format": "png",
            "fromSurface": True,
            "captureBeyondViewport": True,
        },
        timeout_ms=10000,
    )
    if not isinstance(value, dict):
        raise RuntimeError(f"unexpected extension screenshot response: {value!r}")
    data = value.get("data")
    if not isinstance(data, str):
        raise RuntimeError(f"unexpected extension screenshot payload: {value!r}")
    output_path.write_bytes(base64.b64decode(data))


def page_device_pixel_ratio(client: socket.socket, tab_id: int) -> float:
    value = extension_evaluate(
        client,
        tab_id=tab_id,
        expression="window.devicePixelRatio || 1",
        timeout_ms=2000,
    )
    if not isinstance(value, int | float):
        raise RuntimeError(f"unexpected devicePixelRatio value: {value!r}")
    return float(value)


def run_cursor_proof(client: socket.socket, artifact_dir: Path) -> dict[str, object]:
    proof_dir = artifact_dir / "cursor"
    proof_dir.mkdir(parents=True, exist_ok=True)
    tab, tab_id = create_attached_tab(client)
    with cursor_fixture_url() as fixture_url:
        extension_navigate_and_wait(client, tab_id=tab_id, url=fixture_url)
        device_pixel_ratio = page_device_pixel_ratio(client, tab_id)
        before_path = proof_dir / "before.png"
        after_path = proof_dir / "after.png"
        extension_capture_screenshot(client, tab_id=tab_id, output_path=before_path)
        move_response = native_request(
            client,
            "moveMouse",
            {
                **browser_session_params(),
                "tabId": tab_id,
                "x": CURSOR_TARGET_CSS_X,
                "y": CURSOR_TARGET_CSS_Y,
                "waitForArrival": True,
            },
            timeout=20,
            request_id="client-move-mouse-cursor-proof",
        )
        time.sleep(0.2)
        extension_capture_screenshot(client, tab_id=tab_id, output_path=after_path)

    diff = assert_localized_cursor_diff(
        before_path,
        after_path,
        target_x_css=CURSOR_TARGET_CSS_X,
        target_y_css=CURSOR_TARGET_CSS_Y,
        device_pixel_ratio=device_pixel_ratio,
    )
    return {
        "ok": True,
        "tab": tab,
        "tab_id": tab_id,
        "fixture_url": fixture_url,
        "moveMouse_response": move_response,
        "before_screenshot": str(before_path),
        "after_screenshot": str(after_path),
        "diff": diff,
    }


def collect_hacker_news_stories(client: socket.socket, tab_id: int) -> list[dict[str, object]]:
    value = extension_evaluate(
        client,
        tab_id=tab_id,
        expression=r"""
(() => Array.from(document.querySelectorAll('tr.athing'))
  .map((row) => {
    const titleLink = row.querySelector('.titleline > a');
    const subtext = row.nextElementSibling?.querySelector('.subtext');
    const commentsLink = Array.from(subtext?.querySelectorAll('a') || [])
      .find((link) => link.href.includes('item?id=') || /comments?$/i.test(link.textContent || ''));
    let host = '';
    try { host = titleLink?.href ? new URL(titleLink.href).hostname : ''; } catch (_) {}
    return {
      id: row.id || '',
      rank: row.querySelector('.rank')?.textContent?.trim() || '',
      title: titleLink?.textContent?.trim() || '',
      url: titleLink?.href || '',
      host,
      hnUrl: commentsLink?.href || (row.id ? `https://news.ycombinator.com/item?id=${row.id}` : ''),
    };
  })
  .filter((story) => story.title && story.url && story.hnUrl && story.host !== 'news.ycombinator.com')
  .slice(0, 3))()
""",
        timeout_ms=5000,
    )
    return require_story_list(value)


def collect_hacker_news_comments(client: socket.socket, tab_id: int) -> list[dict[str, object]]:
    value = extension_evaluate(
        client,
        tab_id=tab_id,
        expression=r"""
(() => Array.from(document.querySelectorAll('tr.athing.comtr'))
  .slice(0, 3)
  .map((row) => ({
    id: row.id || '',
    user: row.querySelector('.hnuser')?.textContent?.trim() || '[deleted]',
    age: row.querySelector('.age a')?.textContent?.trim() || '',
    text: row.querySelector('.commtext')?.innerText?.trim() || '',
  })))()
""",
        timeout_ms=5000,
    )
    return require_comment_list(value)


def write_hacker_news_markdown(proof_dir: Path, stories: list[dict[str, object]]) -> Path:
    markdown_path = proof_dir / "hacker-news-top3-proof.md"
    lines = [
        "# Hacker News Chrome Host Smoke Proof",
        "",
        f"Generated: {datetime.now(UTC).isoformat()}",
        "",
    ]
    for index, story in enumerate(stories, start=1):
        title = markdown_escape(story.get("title", "Untitled"))
        lines.extend(
            [
                f"## {index}. {title}",
                "",
                f"- Rank: {story.get('rank', '')}",
                f"- Website: [{markdown_escape(story.get('host', ''))}]({story.get('url', '')})",
                f"- Hacker News: [comments]({story.get('hnUrl', '')})",
                f"- Screenshot: `{story.get('screenshot', '')}`",
                "",
                "### Top Comments",
                "",
            ]
        )
        comments = story.get("comments")
        if not isinstance(comments, list) or not comments:
            lines.extend(["No comments captured.", ""])
            continue
        for comment_index, comment in enumerate(comments[:3], start=1):
            if not isinstance(comment, dict):
                continue
            user = markdown_escape(comment.get("user", "[unknown]"))
            age = markdown_escape(comment.get("age", ""))
            text = comment_excerpt(comment.get("text", ""))
            lines.extend(
                [
                    f"{comment_index}. **{user}** {age}",
                    "",
                    f"   {text}",
                    "",
                ]
            )
    markdown_path.write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")
    return markdown_path


def run_hacker_news_proof(client: socket.socket, artifact_dir: Path) -> dict[str, object]:
    proof_dir = artifact_dir / "hacker-news"
    screenshots_dir = proof_dir / "screenshots"
    screenshots_dir.mkdir(parents=True, exist_ok=True)
    hn_tab, hn_tab_id = create_attached_tab(client)
    extension_navigate_and_wait(
        client,
        tab_id=hn_tab_id,
        url="https://news.ycombinator.com/",
    )
    stories = collect_hacker_news_stories(client, hn_tab_id)
    captured: list[dict[str, object]] = []
    for index, story in enumerate(stories, start=1):
        extension_navigate_and_wait(client, tab_id=hn_tab_id, url=str(story["hnUrl"]))
        comments = collect_hacker_news_comments(client, hn_tab_id)
        article_tab, article_tab_id = create_attached_tab(client)
        extension_navigate_and_wait(
            client,
            tab_id=article_tab_id,
            url=str(story["url"]),
            timeout_s=25,
        )
        screenshot_name = story_filename(index, story["title"])
        screenshot_path = screenshots_dir / screenshot_name
        extension_capture_screenshot(client, tab_id=article_tab_id, output_path=screenshot_path)
        captured_story = {
            **story,
            "article_tab": article_tab,
            "article_tab_id": article_tab_id,
            "comments": comments,
            "screenshot": str(screenshot_path.relative_to(proof_dir)),
            "screenshot_path": str(screenshot_path),
        }
        captured.append(captured_story)
    markdown_path = write_hacker_news_markdown(proof_dir, captured)
    return {
        "hn_tab": hn_tab,
        "hn_tab_id": hn_tab_id,
        "proof_dir": str(proof_dir),
        "markdown_path": str(markdown_path),
        "screenshots": [story["screenshot_path"] for story in captured],
        "stories": captured,
    }


def wait_for_extension_runtime(websocket_url: str) -> str:
    expression = (
        "typeof chrome !== 'undefined' && chrome.runtime && chrome.storage && chrome.alarms "
        "? chrome.runtime.id : null"
    )
    deadline = time.time() + 10
    while time.time() < deadline:
        value = evaluate(websocket_url, expression, timeout_ms=1000)
        if isinstance(value, str) and value:
            return value
        time.sleep(0.25)
    raise TimeoutError("extension runtime APIs did not become available")


def write_task_complete(sessions_dir: Path, session_id: str, turn_id: str) -> Path:
    now = datetime.now(UTC)
    rollout_dir = sessions_dir / now.strftime("%Y") / now.strftime("%m") / now.strftime("%d")
    rollout_dir.mkdir(parents=True, exist_ok=True)
    path = rollout_dir / f"rollout-live-smoke-{session_id}.jsonl"
    line = {
        "type": "event_msg",
        "payload": {
            "type": "task_complete",
            "turn_id": turn_id,
        },
    }
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(line, separators=(",", ":")) + "\n")
    return path


def create_artifact_dir(artifacts_root: Path) -> Path:
    stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    output_dir = artifacts_root / stamp
    output_dir.mkdir(parents=True, exist_ok=True)
    return output_dir


def write_artifact(output_dir: Path, result: dict[str, object]) -> Path:
    output_path = output_dir / "result.json"
    output_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return output_path


def run_smoke(args: argparse.Namespace, artifact_dir: Path) -> dict[str, object]:
    extension_dir = args.extension_dir.expanduser().resolve()
    if not extension_dir.exists():
        raise FileNotFoundError(f"extension directory not found: {extension_dir}")
    host_path = args.host_path.expanduser().resolve()
    if args.install_temp_native_manifest and not host_path.exists():
        raise FileNotFoundError(f"host binary not found: {host_path}")
    mcp_client_path = args.mcp_client_path.expanduser().resolve()
    if args.mcp_list_tabs_proof and not mcp_client_path.exists():
        raise FileNotFoundError(f"MCP client binary not found: {mcp_client_path}")
    if args.keep_browser_open and not args.skip_turn_ended_proof:
        raise ValueError(
            "--keep-browser-open cannot prove turnEnded because stderr is read on exit"
        )
    browser = browser_command(args.browser)
    manifest_restore: ManifestRestore | None = None
    manifest_restored = False
    with tempfile.TemporaryDirectory(prefix="sky-cua-chrome-host-smoke-") as temp:
        root = Path(temp)
        user_data_dir = root / "profile"
        loaded_extension_dir = root / "extension"
        socket_dir = root / "sockets"
        sessions_dir = root / "sessions"
        service_socket_path = root / "sky-cua-service.sock"
        shutil.copytree(extension_dir, loaded_extension_dir)
        socket_dir.mkdir()
        sessions_dir.mkdir()
        proc: subprocess.Popen[str] | None = None
        try:
            if args.install_temp_native_manifest:
                manifest_restore = install_temp_manifest(
                    browser.name, args.extension_id, host_path, user_data_dir=user_data_dir
                )
            proc = launch_browser(
                browser.command,
                user_data_dir=user_data_dir,
                extension_dir=loaded_extension_dir,
                socket_dir=socket_dir,
                sessions_dir=sessions_dir,
            )
            port = wait_for_devtools_port(user_data_dir, proc)
            target = wait_for_extension_target(port, args.extension_id)
            websocket_url = str(target["webSocketDebuggerUrl"])
            extension_id = wait_for_extension_runtime(websocket_url)
            status_before = evaluate(
                websocket_url, "chrome.storage.local.get('NATIVE_HOST_STATUS')"
            )
            socket_path = wait_for_socket(socket_dir)
            host_pid = host_pid_from_socket(socket_path)
            host_process = (
                wait_for_host_process(host_path, host_pid)
                if args.install_temp_native_manifest
                else None
            )
            if args.install_temp_native_manifest and host_process is None:
                raise RuntimeError(f"native host process did not match {host_path}")
            client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                client.connect(str(socket_path))
                get_info = native_request(
                    client,
                    "getInfo",
                    browser_session_params(),
                    request_id="client-get-info",
                )
                get_tabs = native_request(
                    client,
                    "getTabs",
                    browser_session_params(),
                    request_id="client-get-tabs",
                )
                cursor_proof = (
                    None if args.skip_cursor_proof else run_cursor_proof(client, artifact_dir)
                )
                hacker_news_proof = (
                    run_hacker_news_proof(client, artifact_dir) if args.hacker_news_proof else None
                )
                mcp_visible_tab = None
                mcp_visible_tab_id = None
                get_user_tabs_for_mcp_proof = None
                mcp_list_tabs_proof = None
                if args.mcp_list_tabs_proof:
                    mcp_visible_tab, mcp_visible_tab_id = create_mcp_visible_tab(client)
                    get_user_tabs_for_mcp_proof = native_request(
                        client,
                        "getUserTabs",
                        mcp_browser_session_params(),
                        request_id="client-get-user-tabs-mcp-proof",
                    )
                    get_user_tabs_for_mcp_proof = redacted_tab_list_proof(
                        get_user_tabs_for_mcp_proof,
                        expected_tab_id=mcp_visible_tab_id,
                    )
                    mcp_list_tabs_proof = run_mcp_list_tabs_proof(
                        client_path=mcp_client_path,
                        socket_dir=socket_dir,
                        service_socket_path=service_socket_path,
                        expected_tab_id=mcp_visible_tab_id,
                    )
                get_tabs_after_hacker_news = None
                if hacker_news_proof is not None:
                    get_tabs_after_hacker_news = native_request(
                        client,
                        "getTabs",
                        browser_session_params(),
                        request_id="client-get-tabs-after-hacker-news",
                    )
                session_file = None
                if not args.skip_turn_ended_proof:
                    session_file = write_task_complete(
                        sessions_dir, SMOKE_SESSION_ID, SMOKE_TURN_ID
                    )
                    time.sleep(2)
                evaluate(
                    websocket_url,
                    "chrome.alarms.create('client-heartbeat-alarm', "
                    "{when: Date.now() + 100}); 'alarm-scheduled'",
                )
                heartbeat, heartbeat_replied = read_heartbeat(client)
                status_after = evaluate(
                    websocket_url,
                    "chrome.storage.local.get('NATIVE_HOST_STATUS')",
                )
            finally:
                client.close()
            stderr_tail = terminate_browser(proc, args.keep_browser_open)[-4000:]
            turn_ended_response = turn_ended_response_from_stderr(stderr_tail)
            turn_ended_proof = {
                "session_file": str(session_file) if session_file is not None else None,
                "emitted": (f"emitting turnEnded session={SMOKE_SESSION_ID} turn={SMOKE_TURN_ID}")
                in stderr_tail,
                "extension_response": turn_ended_response,
                "extension_accepted": turn_ended_response_was_successful(turn_ended_response),
            }
            if not args.skip_turn_ended_proof and not (
                turn_ended_proof["emitted"] and turn_ended_proof["extension_accepted"]
            ):
                raise RuntimeError(
                    "turnEnded proof failed; expected host emit trace and successful extension response\n"
                    + stderr_tail
                )
        except BaseException:
            if proc is not None:
                stderr_tail = terminate_browser(proc, args.keep_browser_open)[-4000:]
                if stderr_tail:
                    print(
                        "browser/native-host stderr tail after smoke failure:\n" + stderr_tail,
                        file=sys.stderr,
                    )
            raise
        finally:
            manifest_restored = restore_manifest(manifest_restore)

    return {
        "ok": True,
        "browser": browser.name,
        "browser_command": browser.command,
        "extension_dir": str(extension_dir),
        "extension_id": extension_id,
        "host_path": str(host_path) if args.install_temp_native_manifest else None,
        "native_manifest_path": str(manifest_restore.path)
        if manifest_restore is not None
        else None,
        "native_manifest_restored": manifest_restored,
        "host_process": host_process,
        "extension_target": {
            "type": target.get("type"),
            "url": target.get("url"),
        },
        "native_socket_path": str(socket_path),
        "native_status_before": status_before,
        "client_to_extension_getInfo": get_info,
        "client_to_extension_getTabs": get_tabs,
        "mcp_visible_tab": mcp_visible_tab,
        "client_to_extension_getUserTabs_for_mcp_proof": get_user_tabs_for_mcp_proof,
        "mcp_list_tabs_proof": mcp_list_tabs_proof,
        "cursor_proof": cursor_proof,
        "hacker_news_proof": hacker_news_proof,
        "client_to_extension_getTabs_after_hacker_news": get_tabs_after_hacker_news,
        "extension_to_client_heartbeat": {
            "received": heartbeat,
            "replied": heartbeat_replied,
        },
        "turn_ended_proof": turn_ended_proof,
        "native_status_after": status_after,
        "browser_stderr_tail": stderr_tail,
    }


def main() -> int:
    args = parse_args()
    artifact_dir = create_artifact_dir(args.artifacts_root)
    result = run_smoke(args, artifact_dir)
    artifact_path = write_artifact(artifact_dir, result)
    result["artifact_path"] = str(artifact_path)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
