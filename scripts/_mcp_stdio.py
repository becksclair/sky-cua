"""Small MCP stdio client shared by live smoke harnesses."""

from __future__ import annotations

import json
import os
import select
import signal
import subprocess
import tempfile
import time
from contextlib import ExitStack, suppress
from dataclasses import dataclass
from pathlib import Path
from typing import Any, BinaryIO, cast

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MCP_READ_TIMEOUT_SECONDS = 15.0


def process_ids_with_env_var(
    name: str,
    value: str,
    *,
    proc_root: Path = Path("/proc"),
) -> list[int]:
    if not proc_root.exists():
        return []

    current_pid = os.getpid()
    needle = f"{name}={value}".encode()
    matches: list[int] = []
    for entry in proc_root.iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        if pid == current_pid:
            continue
        try:
            environ = (entry / "environ").read_bytes()
        except OSError:
            continue
        if needle in environ.split(b"\0"):
            matches.append(pid)
    return matches


def stop_service_processes_for_socket(
    service_socket_path: Path,
    *,
    proc_root: Path = Path("/proc"),
) -> None:
    for pid in process_ids_with_env_var(
        "SKY_CUA_SERVICE_SOCKET_PATH",
        str(service_socket_path),
        proc_root=proc_root,
    ):
        terminate_process(pid, proc_root=proc_root)


def terminate_process(pid: int, *, proc_root: Path = Path("/proc")) -> None:
    with suppress(ProcessLookupError, PermissionError):
        os.kill(pid, signal.SIGTERM)

    proc_entry = proc_root / str(pid)
    deadline = time.monotonic() + 5
    while proc_entry.exists() and time.monotonic() < deadline:
        time.sleep(0.05)

    if proc_entry.exists():
        with suppress(ProcessLookupError, PermissionError):
            os.kill(pid, signal.SIGKILL)


@dataclass(frozen=True)
class McpResponse:
    raw: dict[str, Any]

    @property
    def result(self) -> dict[str, Any]:
        result = self.raw.get("result")
        if not isinstance(result, dict):
            raise RuntimeError(
                "MCP call did not return a result payload.\n"
                f"response={json.dumps(self.raw, indent=2, sort_keys=True)}"
            )
        return result


class McpClient:
    def __init__(
        self,
        argv: list[str],
        *,
        extra_env: dict[str, str] | None = None,
        base_env: dict[str, str] | None = None,
        cwd: Path | None = None,
        read_timeout: float = DEFAULT_MCP_READ_TIMEOUT_SECONDS,
        client_name: str = "live-desktop-smoke",
        client_version: str = "0.2.0",
    ) -> None:
        env = dict(os.environ if base_env is None else base_env)
        env.setdefault("SKY_CUA_REPO_ROOT", str(REPO_ROOT))
        if extra_env:
            env.update(extra_env)

        self._stack = ExitStack()
        self._closed = False
        self.read_timeout = read_timeout
        self.client_name = client_name
        self.client_version = client_version
        self.stderr: BinaryIO = self._stack.enter_context(tempfile.TemporaryFile())  # noqa: SIM115
        self.proc: subprocess.Popen[bytes] = subprocess.Popen(
            argv,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr,
            text=False,
            cwd=REPO_ROOT if cwd is None else cwd,
            env=env,
        )

    def close(self) -> None:
        if self._closed:
            return
        try:
            if self.proc.poll() is None:
                self.proc.terminate()
                try:
                    self.proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
                    self.proc.wait(timeout=5)
        finally:
            self._closed = True
            self._stack.close()

    def initialize(self) -> None:
        self.call_raw(
            1,
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": self.client_name, "version": self.client_version},
            },
        )
        self.notify("notifications/initialized", {})

    def notify(self, method: str, params: dict[str, Any]) -> None:
        self._write_message({"jsonrpc": "2.0", "method": method, "params": params})

    def call_raw(self, request_id: int, method: str, params: dict[str, Any]) -> McpResponse:
        self._write_message(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        )
        return McpResponse(self._read_message())

    def tools_list(self) -> list[dict[str, Any]]:
        response = self.call_raw(2, "tools/list", {})
        tools = response.result.get("tools")
        if not isinstance(tools, list):
            raise RuntimeError(f"tools/list did not return tools: {response.raw!r}")
        return cast(list[dict[str, Any]], tools)

    def tools_call(self, request_id: int, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        response = self.call_raw(
            request_id,
            "tools/call",
            {"name": name, "arguments": arguments},
        )
        return response.result

    def _write_message(self, payload: dict[str, Any]) -> None:
        encoded = json.dumps(payload).encode("utf-8")
        message = f"Content-Length: {len(encoded)}\r\n\r\n".encode("ascii") + encoded
        if self.proc.stdin is None:
            raise RuntimeError("MCP client stdin is closed")
        self.proc.stdin.write(message)
        self.proc.stdin.flush()

    def _read_message(self) -> dict[str, Any]:
        if self.proc.stdout is None:
            raise RuntimeError("MCP client stdout is closed")

        stdout_fd = self.proc.stdout.fileno()
        deadline = time.monotonic() + self.read_timeout
        headers: dict[str, str] = {}
        while True:
            line = self._readline_with_timeout(stdout_fd, deadline)
            if not line:
                raise RuntimeError(
                    f"MCP client exited unexpectedly.\nstderr:\n{self._stderr_text()}"
                )
            if line == b"\r\n":
                break
            name, _, value = line.decode("ascii").partition(":")
            headers[name.strip().lower()] = value.strip()

        length = int(headers["content-length"])
        body = self._read_exactly_with_timeout(stdout_fd, length, deadline)
        value = json.loads(body.decode("utf-8"))
        if not isinstance(value, dict):
            raise RuntimeError(f"unexpected MCP response payload: {value!r}")
        return cast(dict[str, Any], value)

    def _readline_with_timeout(self, fd: int, deadline: float) -> bytes:
        chunks: list[bytes] = []
        while True:
            chunk = self._read_from_fd_with_timeout(fd, 1, deadline, "reading MCP headers")
            if not chunk:
                return b"".join(chunks)
            chunks.append(chunk)
            if chunk == b"\n":
                return b"".join(chunks)

    def _read_exactly_with_timeout(self, fd: int, length: int, deadline: float) -> bytes:
        body = bytearray()
        while len(body) < length:
            chunk = self._read_from_fd_with_timeout(
                fd,
                length - len(body),
                deadline,
                "reading MCP body",
            )
            if not chunk:
                raise RuntimeError(
                    f"MCP client exited before sending a complete body.\nstderr:\n{self._stderr_text()}"
                )
            body.extend(chunk)
        return bytes(body)

    def _read_from_fd_with_timeout(
        self, fd: int, size: int, deadline: float, context: str
    ) -> bytes:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise self._timeout_error(context)
            readable, _, _ = select.select([fd], [], [], remaining)
            if not readable:
                raise self._timeout_error(context)
            try:
                return os.read(fd, size)
            except BlockingIOError:
                continue

    def _timeout_error(self, context: str) -> RuntimeError:
        stderr = self._stderr_text()
        self.close()
        return RuntimeError(
            f"MCP client timed out while {context} after {self.read_timeout:g}s.\nstderr:\n{stderr}"
        )

    def _stderr_text(self) -> str:
        if self._closed:
            return ""
        self.stderr.flush()
        self.stderr.seek(0)
        return self.stderr.read().decode(errors="replace")
