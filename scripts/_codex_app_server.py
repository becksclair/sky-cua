"""Shared stdio JSON-RPC client for `codex app-server`.

The rich smoke harness (`_app_server_harness.py`) drives `codex app-server`
over line-oriented stdio JSON-RPC. This module owns the transport lifecycle:
process spawn, queue-backed stdout/stderr readers, request id allocation,
initialize/initialized sequencing, timeout reporting with stderr context,
and bounded shutdown.

Two consumption lanes are supported:

- ``request()`` blocks until the matching response arrives, buffering other
  inbound messages into ``notifications`` - a convenience for simple
  request/response callers.
- ``read_message()`` streams every inbound message in arrival order. Used by
  the rich harness, which must record a full transcript and answer
  server-to-client requests itself.
"""

from __future__ import annotations

import contextlib
import json
import queue
import subprocess
import threading
import time
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any, Protocol

_POLL_INTERVAL_SECONDS = 0.25
_SHUTDOWN_WAIT_SECONDS = 5.0
_READER_JOIN_SECONDS = 2.0
_STDERR_TAIL_LINES = 80


class AppServerExited(RuntimeError):
    """The app-server closed stdout before producing an expected message."""


class _WritableStream(Protocol):
    def write(self, data: str, /) -> int: ...

    def flush(self) -> None: ...


class AppServerProcessLike(Protocol):
    """Structural subset of ``subprocess.Popen[str]`` the client uses."""

    @property
    def stdin(self) -> _WritableStream | None: ...

    @property
    def stdout(self) -> Iterable[str] | None: ...

    @property
    def stderr(self) -> Iterable[str] | None: ...

    def poll(self) -> int | None: ...

    def terminate(self) -> None: ...

    def kill(self) -> None: ...

    def wait(self, timeout: float | None = None) -> int: ...


class CodexAppServerClient:
    def __init__(
        self,
        command: Sequence[str],
        *,
        env: dict[str, str] | None = None,
        cwd: Path | None = None,
        process: AppServerProcessLike | None = None,
    ) -> None:
        self.command = list(command)
        if process is None:
            process = subprocess.Popen(
                self.command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
                errors="replace",
                bufsize=1,
                env=env,
                cwd=cwd,
            )
        self.proc = process
        self.next_id = 1
        self.notifications: list[dict[str, Any]] = []
        self._messages: queue.Queue[dict[str, Any]] = queue.Queue()
        self._stderr_lines: list[str] = []
        self._stdout_done = threading.Event()
        assert self.proc.stdout is not None
        assert self.proc.stderr is not None
        self._stdout_thread = threading.Thread(
            target=self._read_stdout,
            args=(self.proc.stdout,),
            daemon=True,
        )
        self._stderr_thread = threading.Thread(
            target=self._read_stderr,
            args=(self.proc.stderr,),
            daemon=True,
        )
        self._stdout_thread.start()
        self._stderr_thread.start()

    def _read_stdout(self, stream: Iterable[str]) -> None:
        try:
            for line in stream:
                if not line.strip():
                    continue
                try:
                    self._messages.put(json.loads(line))
                except json.JSONDecodeError:
                    self._stderr_lines.append(f"non-json stdout: {line.rstrip()}")
        finally:
            self._stdout_done.set()

    def _read_stderr(self, stream: Iterable[str]) -> None:
        for line in stream:
            self._stderr_lines.append(line.rstrip())

    def write(self, payload: dict[str, Any]) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(payload, separators=(",", ":")) + "\n")
        self.proc.stdin.flush()

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        payload: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            payload["params"] = params
        self.write(payload)

    def send_request(self, method: str, params: dict[str, Any] | None = None) -> int:
        """Send a request and return its id without waiting for the response."""
        request_id = self.next_id
        self.next_id += 1
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            payload["params"] = params
        self.write(payload)
        return request_id

    def respond(self, request_id: Any, result: Any) -> None:
        """Answer a server-to-client request."""
        self.write({"jsonrpc": "2.0", "id": request_id, "result": result})

    def read_message(self, *, timeout: float | None = None) -> dict[str, Any]:
        """Return the next inbound message in arrival order.

        Raises ``TimeoutError`` when ``timeout`` elapses with no message and
        ``AppServerExited`` when stdout is closed with nothing left queued.
        """
        deadline = time.monotonic() + timeout if timeout is not None else None
        while True:
            wait = _POLL_INTERVAL_SECONDS
            if deadline is not None:
                wait = min(wait, max(0.0, deadline - time.monotonic()))
            try:
                return self._messages.get(timeout=wait)
            except queue.Empty:
                if self._stdout_done.is_set() and self._messages.empty():
                    raise AppServerExited(
                        "app-server exited unexpectedly.\n"
                        f"command={' '.join(self.command)}\n"
                        f"stderr:\n{self.stderr_tail()}"
                    ) from None
                if deadline is not None and time.monotonic() >= deadline:
                    raise TimeoutError(
                        f"timed out waiting for app-server message after {timeout:.1f}s"
                    ) from None

    def request(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        timeout: float = 60.0,
    ) -> dict[str, Any]:
        """Send a request and block until its response, buffering other messages."""
        request_id = self.send_request(method, params)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                message = self._messages.get(timeout=_POLL_INTERVAL_SECONDS)
            except queue.Empty:
                if self._stdout_done.is_set() and self._messages.empty():
                    raise RuntimeError(
                        f"codex app-server exited before {method} completed.\nstderr:\n"
                        + self.stderr_tail()
                    ) from None
                continue
            if message.get("id") != request_id:
                self.notifications.append(message)
                continue
            if "error" in message:
                raise RuntimeError(f"{method} failed: {json.dumps(message['error'])}")
            result = message.get("result")
            return result if isinstance(result, dict) else {}
        raise TimeoutError(f"timed out waiting for {method}.\nstderr:\n" + self.stderr_tail())

    def initialize(
        self,
        *,
        client_name: str,
        client_version: str,
        client_title: str | None = None,
        capabilities: dict[str, Any] | None = None,
        timeout: float = 60.0,
    ) -> dict[str, Any]:
        """Perform the initialize request plus the initialized notification."""
        client_info: dict[str, Any] = {"name": client_name, "version": client_version}
        if client_title is not None:
            client_info["title"] = client_title
        result = self.request(
            "initialize",
            {"clientInfo": client_info, "capabilities": capabilities or {}},
            timeout=timeout,
        )
        self.notify("initialized", {})
        return result

    def stderr_tail(self, limit: int = _STDERR_TAIL_LINES) -> str:
        return "\n".join(self._stderr_lines[-limit:])

    def stderr_text(self) -> str:
        """Return captured stderr after waiting briefly for the reader to drain."""
        self._stderr_thread.join(timeout=_READER_JOIN_SECONDS)
        if not self._stderr_lines:
            return ""
        return "\n".join(self._stderr_lines) + "\n"

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=_SHUTDOWN_WAIT_SECONDS)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                with contextlib.suppress(subprocess.TimeoutExpired):
                    self.proc.wait(timeout=_SHUTDOWN_WAIT_SECONDS)
        self._stdout_thread.join(timeout=_READER_JOIN_SECONDS)
        self._stderr_thread.join(timeout=_READER_JOIN_SECONDS)
