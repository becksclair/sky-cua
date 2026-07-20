"""Deterministic process harness for the service browser control plane.

The harness owns only temporary Unix sockets and files.  It deliberately talks
the real service, Codex compatibility, and native-host wire protocols instead
of importing service implementation details.
"""

from __future__ import annotations

import json
import os
import queue
import shutil
import socket
import stat
import struct
import subprocess
import tempfile
import threading
import time
from collections.abc import Callable, Iterator
from contextlib import AbstractContextManager, suppress
from pathlib import Path
from types import TracebackType
from typing import Any, BinaryIO, Self, cast

JsonObject = dict[str, Any]
HostResponder = Callable[[JsonObject], JsonObject | None]

REQUIRED_HOST_CAPABILITIES = [
    "control_plane",
    "heartbeat",
    "extension_events",
    "private_param_stripping",
    "settlements",
    "settlement_ack",
    "side_panel_requests",
    "owner_release",
]


def debug_service_binary(repo_root: Path) -> Path:
    """Return the debug binary path; callers decide whether absence is a skip."""
    return repo_root / "target" / "debug" / "sky-cua-service"


def _write_frame(stream: socket.socket, payload: JsonObject) -> None:
    body = json.dumps(payload, separators=(",", ":")).encode()
    stream.sendall(struct.pack("=I", len(body)) + body)


def _read_exact(stream: socket.socket, length: int) -> bytes:
    chunks: list[bytes] = []
    remaining = length
    while remaining:
        chunk = stream.recv(remaining)
        if not chunk:
            raise EOFError("socket closed while reading frame")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def _read_frame(stream: socket.socket) -> JsonObject:
    size = struct.unpack("=I", _read_exact(stream, 4))[0]
    value = json.loads(_read_exact(stream, size))
    if not isinstance(value, dict):
        raise TypeError(f"expected JSON object frame, got {value!r}")
    return cast(JsonObject, value)


class Transcript:
    """Thread-safe JSONL evidence recorder."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self._lock = threading.Lock()

    def append(self, lane: str, direction: str, message: JsonObject) -> None:
        record = {
            "monotonic_ns": time.monotonic_ns(),
            "lane": lane,
            "direction": direction,
            "message": message,
        }
        with self._lock, self.path.open("a", encoding="utf-8") as output:
            output.write(json.dumps(record, sort_keys=True) + "\n")


class FakeNativeHost(AbstractContextManager["FakeNativeHost"]):
    """Controllable fake for the minimum persistent native-host protocol."""

    def __init__(
        self,
        socket_path: Path,
        transcript: Transcript,
        *,
        host_instance_id: str = "fake-host-1",
        browser_instance_id: str = "fake-browser-1",
        stability: str = "stable",
        responder: HostResponder | None = None,
    ) -> None:
        self.socket_path = socket_path
        self.transcript = transcript
        self.host_instance_id = host_instance_id
        self.browser_instance_id = browser_instance_id
        self.stability = stability
        self.responder = responder or self._default_response
        self._next_tab_id = 1000
        self.requests: list[JsonObject] = []
        self.hellos: list[JsonObject] = []
        self.connection_count = 0
        self.error: BaseException | None = None
        self._condition = threading.Condition()
        self._connections: list[socket.socket] = []
        self._stop = threading.Event()
        self._listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._listener.bind(str(socket_path))
        os.chmod(socket_path, 0o600)
        self._listener.listen(8)
        self._listener.settimeout(0.1)
        self._thread = threading.Thread(target=self._serve, name="fake-native-host", daemon=True)
        self._thread.start()

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()

    def _serve(self) -> None:
        try:
            while not self._stop.is_set():
                try:
                    connection, _ = self._listener.accept()
                except TimeoutError:
                    continue
                with self._condition:
                    self.connection_count += 1
                    self._connections.append(connection)
                    self._condition.notify_all()
                thread = threading.Thread(
                    target=self._serve_connection,
                    args=(connection,),
                    name=f"fake-native-host-{self.connection_count}",
                    daemon=True,
                )
                thread.start()
        except BaseException as error:
            if not self._stop.is_set():
                self.error = error
                with self._condition:
                    self._condition.notify_all()

    def _serve_connection(self, connection: socket.socket) -> None:
        connection.settimeout(0.2)
        try:
            while not self._stop.is_set():
                try:
                    frame = _read_frame(connection)
                except TimeoutError:
                    continue
                except (EOFError, OSError):
                    return
                self.transcript.append("native_host", "received", frame)
                method = frame.get("method")
                if method == "skyCuaHost/hello":
                    with self._condition:
                        self.hellos.append(frame)
                        self._condition.notify_all()
                    owner_mode = frame.get("params", {}).get("owner_mode")
                    response = {
                        "jsonrpc": "2.0",
                        "id": frame["id"],
                        "result": {
                            "protocol_version": 1,
                            "mode": owner_mode,
                            "owner_mode": owner_mode,
                            "host_instance_id": self.host_instance_id,
                            "browser_instance_id": self.browser_instance_id,
                            "browser_instance_stability": self.stability,
                            "browser_family": "brave",
                            "capabilities": REQUIRED_HOST_CAPABILITIES,
                        },
                    }
                elif method == "skyCuaHost/settlementAck":
                    response = None
                elif method == "skyCuaHost/release":
                    response = {
                        "jsonrpc": "2.0",
                        "id": frame["id"],
                        "result": {"released": True, "owner_mode": "hybrid"},
                    }
                elif method == "ping":
                    response = {"jsonrpc": "2.0", "id": frame["id"], "result": "pong"}
                else:
                    with self._condition:
                        self.requests.append(frame)
                        self._condition.notify_all()
                    response = self.responder(frame)
                if response is not None:
                    self.transcript.append("native_host", "sent", response)
                    _write_frame(connection, response)
        except BaseException as error:
            if not self._stop.is_set():
                self.error = error
                with self._condition:
                    self._condition.notify_all()

    def _default_response(self, request: JsonObject) -> JsonObject:
        method = request.get("method")
        params = request.get("params")
        if method == "getInfo":
            result: Any = {
                "protocolVersion": 7,
                "browser": "Brave",
                "codexAppBuildFlavor": "acceptance-fake",
                "nested": {"preserved": True},
            }
        elif method in {"getTabs", "getUserTabs"}:
            result = {
                "tabs": [
                    {"tabId": 101, "title": "A", "url": "https://a.test"},
                    {"tabId": 102, "title": "B", "url": "https://b.test"},
                ]
            }
        elif method in {"create", "createTab", "open", "openTab"}:
            self._next_tab_id += 1
            result = {
                "id": self._next_tab_id,
                "title": "Fake acceptance tab",
                "url": "about:blank",
                "active": True,
            }
        elif method in {"claim", "claimTab", "claimUserTab"}:
            result = {"tabId": _find_tab_id(params)}
        elif method == "attach":
            result = {}
        elif method == "forceError":
            return {
                "jsonrpc": "2.0",
                "id": request["id"],
                "error": {"code": -32123, "message": "fake upstream exact error", "data": {"x": 1}},
            }
        else:
            result = {"method": method, "tabId": _find_tab_id(params), "accepted": True}
        return {"jsonrpc": "2.0", "id": request["id"], "result": result}

    def wait_for_requests(
        self,
        count: int,
        *,
        method: str | None = None,
        timeout: float = 5.0,
    ) -> list[JsonObject]:
        deadline = time.monotonic() + timeout
        with self._condition:
            while True:
                if self.error is not None:
                    raise RuntimeError("fake native host failed") from self.error
                matching = [
                    item for item in self.requests if method is None or item.get("method") == method
                ]
                if len(matching) >= count:
                    return matching
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(
                        f"timed out waiting for {count} {method or 'host'} request(s); "
                        f"received={self.requests!r}"
                    )
                self._condition.wait(remaining)

    def wait_for_connections(self, count: int, timeout: float = 5.0) -> None:
        deadline = time.monotonic() + timeout
        with self._condition:
            while self.connection_count < count:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f"expected {count} native-host connection(s)")
                self._condition.wait(remaining)

    def wait_for_hellos(self, count: int, timeout: float = 5.0) -> None:
        deadline = time.monotonic() + timeout
        with self._condition:
            while len(self.hellos) < count:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f"expected {count} native-host hello(s)")
                self._condition.wait(remaining)

    def send(self, message: JsonObject, *, connection: int = -1) -> None:
        with self._condition:
            stream = self._connections[connection]
            self.transcript.append("native_host", "sent", message)
            _write_frame(stream, message)

    def disconnect_all(self) -> None:
        with self._condition:
            connections = list(self._connections)
            self._connections.clear()
        for connection in connections:
            with suppress(OSError):
                connection.shutdown(socket.SHUT_RDWR)
            connection.close()

    def close(self) -> None:
        self._stop.set()
        self.disconnect_all()
        self._listener.close()
        self._thread.join(timeout=2)
        with suppress(FileNotFoundError):
            self.socket_path.unlink()


class ServiceClient(AbstractContextManager["ServiceClient"]):
    """Persistent ordinary service IPC client."""

    def __init__(self, socket_path: Path, transcript: Transcript, name: str) -> None:
        self.name = name
        self.transcript = transcript
        self._socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._socket.connect(str(socket_path))
        self._file = self._socket.makefile("rwb", buffering=0)
        self._lock = threading.Lock()

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()

    def call(self, request: JsonObject) -> JsonObject:
        with self._lock:
            self.transcript.append(self.name, "sent", request)
            self._file.write(json.dumps(request, separators=(",", ":")).encode() + b"\n")
            line = self._file.readline()
            if not line:
                raise EOFError(f"ordinary service client {self.name} disconnected")
            response = json.loads(line)
            if not isinstance(response, dict):
                raise TypeError(f"unexpected service response: {response!r}")
            value = cast(JsonObject, response)
            self.transcript.append(self.name, "received", value)
            return value

    def close(self) -> None:
        self._file.close()
        self._socket.close()


class RawCodexClient(AbstractContextManager["RawCodexClient"]):
    """Raw upstream Codex Browser compatibility client."""

    def __init__(self, socket_path: Path, transcript: Transcript) -> None:
        self.transcript = transcript
        self._socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._socket.settimeout(5)
        self._socket.connect(str(socket_path))
        self._next_id = 1

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()

    def send_request(self, method: str, params: JsonObject | None = None) -> int:
        request_id = self._next_id
        self._next_id += 1
        message = {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": {} if params is None else params,
        }
        self.send(message)
        return request_id

    def send(self, message: JsonObject) -> None:
        self.transcript.append("codex", "sent", message)
        _write_frame(self._socket, message)

    def receive(self, timeout: float = 5.0) -> JsonObject:
        self._socket.settimeout(timeout)
        message = _read_frame(self._socket)
        self.transcript.append("codex", "received", message)
        return message

    def call(self, method: str, params: JsonObject | None = None) -> JsonObject:
        request_id = self.send_request(method, params)
        while True:
            message = self.receive()
            if message.get("id") == request_id:
                return message

    def close(self) -> None:
        self._socket.close()


class BrowserControlHarness(AbstractContextManager["BrowserControlHarness"]):
    """Launch a real debug service against hermetic fake browser endpoints."""

    def __init__(
        self,
        repo_root: Path,
        root: Path,
        *,
        mode: str,
        host_stability: str = "stable",
    ) -> None:
        if mode not in {"hybrid", "strict"}:
            raise ValueError(f"unsupported browser control mode: {mode}")
        self.repo_root = repo_root
        self.root = root
        self.mode = mode
        self.host_stability = host_stability
        self.root.mkdir(mode=0o700, parents=True)
        os.chmod(self.root, 0o700)
        requested_runtime = self.root / "runtime"
        self._short_runtime_dir: Path | None = None
        native_socket_name = f"extension-{os.getpid()}-acceptance.sock"
        if len(os.fsencode(requested_runtime / native_socket_name)) >= 100:
            self._short_runtime_dir = Path(
                tempfile.mkdtemp(prefix="sky-cua-acceptance-", dir="/tmp")
            )
            self.runtime_dir = self._short_runtime_dir
        else:
            self.runtime_dir = requested_runtime
            self.runtime_dir.mkdir(mode=0o700)
        os.chmod(self.runtime_dir, 0o700)
        self.artifact_dir = self.root / "artifacts"
        self.artifact_dir.mkdir(mode=0o700)
        self.state_home = self.root / "state"
        self.state_home.mkdir(mode=0o700)
        self.journal_path = self.state_home / "sky-cua" / "browser-control-recovery-v1.json"
        self.service_socket = self.runtime_dir / "service.sock"
        self.codex_socket = self.runtime_dir / "codex-browser.sock"
        self.native_socket = self.runtime_dir / native_socket_name
        self.stderr_path = self.artifact_dir / "service.stderr.log"
        self.transcript = Transcript(self.artifact_dir / "wire-transcript.jsonl")
        self.host: FakeNativeHost | None = None
        self.process: subprocess.Popen[bytes] | None = None
        self._stderr: BinaryIO | None = None

    def __enter__(self) -> Self:
        self.host = FakeNativeHost(
            self.native_socket, self.transcript, stability=self.host_stability
        )
        self._launch_service()
        self._wait_for_control_plane_ready()
        return self

    def _launch_service(self) -> None:
        binary = debug_service_binary(self.repo_root)
        environment = os.environ.copy()
        environment.update(
            {
                "SKY_CUA_SERVICE_SOCKET_PATH": str(self.service_socket),
                "SKY_CUA_CODEX_BROWSER_SOCKET_PATH": str(self.codex_socket),
                "SKY_CUA_BROWSER_USE_SOCKET_DIR": str(self.runtime_dir),
                "CODEX_BROWSER_USE_SOCKET_DIR": str(self.runtime_dir),
                "SKY_CUA_BROWSER": "all",
                "SKY_CUA_BROWSER_CONTROL_MODE": self.mode,
                "SKY_CUA_CONFIG_PATH": str(self.runtime_dir / "machine-config.toml"),
                "XDG_RUNTIME_DIR": str(self.runtime_dir),
                "XDG_STATE_HOME": str(self.state_home),
                "RUST_LOG": "sky_cua_service=debug",
            }
        )
        self._stderr = self.stderr_path.open("ab")
        self.process = subprocess.Popen(
            [str(binary), "daemon"],
            cwd=self.repo_root,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=self._stderr,
        )
        self._wait_for_socket(self.service_socket)
        self._wait_for_socket(self.codex_socket)
        self._assert_owner_only(self.service_socket)
        self._assert_owner_only(self.codex_socket)

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()

    def ordinary_client(self, name: str) -> ServiceClient:
        return ServiceClient(self.service_socket, self.transcript, name)

    def codex_client(self) -> RawCodexClient:
        return RawCodexClient(self.codex_socket, self.transcript)

    def control_plane_status(self, name: str = "acceptance-status") -> JsonObject:
        """Return the production control-plane snapshot without bridge mutation."""
        with self.ordinary_client(name) as client:
            response = client.call(
                browser_request(
                    "direct_mcp",
                    name,
                    f"{name}-operation",
                    {"type": "status"},
                )
            )
        try:
            return cast(
                JsonObject,
                response["response"]["report"]["control_plane"],
            )
        except (KeyError, TypeError) as error:
            raise AssertionError(f"missing browser control-plane status: {response!r}") from error

    def restart_service(self) -> JsonObject:
        """Restart only the real daemon, preserving browser and persistent state."""
        if self.host is None:
            raise RuntimeError("browser-control harness is not running")
        previous = self.control_plane_status("before-daemon-restart")
        previous_generation = str(previous["daemon_generation"])
        previous_hello_count = len(self.host.hellos)
        self._terminate_service()
        for path in (self.service_socket, self.codex_socket):
            with suppress(FileNotFoundError):
                path.unlink()
        self._launch_service()
        return self._wait_for_control_plane_ready(
            previous_generation=previous_generation,
            minimum_hello_count=previous_hello_count + 1,
        )

    def _wait_for_socket(self, path: Path, timeout: float = 8.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if path.exists() and stat.S_ISSOCK(path.stat().st_mode):
                return
            if self.process is not None and self.process.poll() is not None:
                self._flush_stderr()
                raise RuntimeError(
                    f"sky-cua-service exited with {self.process.returncode}; "
                    f"stderr={self.stderr_path.read_text(errors='replace')}"
                )
            time.sleep(0.02)
        raise TimeoutError(f"sky-cua-service did not create {path}")

    @staticmethod
    def _assert_owner_only(path: Path) -> None:
        assert stat.S_IMODE(path.stat().st_mode) == 0o600

    def _wait_for_control_plane_ready(
        self,
        timeout: float = 8.0,
        *,
        previous_generation: str | None = None,
        minimum_hello_count: int = 1,
    ) -> JsonObject:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            snapshot = self.control_plane_status("bootstrap-status")
            generation = str(snapshot["daemon_generation"])
            actors = cast(list[JsonObject], snapshot.get("actors", []))
            if (
                generation != previous_generation
                and snapshot.get("ready") is True
                and any(actor.get("state") == "ready" for actor in actors)
                and self.host is not None
                and len(self.host.hellos) >= minimum_hello_count
            ):
                return snapshot
            time.sleep(0.02)
        raise TimeoutError("sky-cua control-plane actor did not become ready")

    def _flush_stderr(self) -> None:
        if self._stderr is not None:
            self._stderr.flush()

    def _terminate_service(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.process = None
        self._flush_stderr()
        if self._stderr is not None:
            self._stderr.close()
            self._stderr = None

    def close(self) -> None:
        self._terminate_service()
        if self.host is not None:
            self.host.close()
            self.host = None
        if self._short_runtime_dir is not None:
            shutil.rmtree(self._short_runtime_dir, ignore_errors=True)
            self._short_runtime_dir = None


def browser_request(
    caller: str,
    connection_id: str,
    operation_id: str,
    request: JsonObject,
    *,
    session_id: str | None = None,
    thread_id: str | None = None,
    turn_id: str | None = None,
) -> JsonObject:
    """Build an exact ordinary service browser request with explicit provenance."""
    logical_session = session_id or f"session-{connection_id}"
    logical: JsonObject = {"session_id": logical_session}
    if thread_id is not None:
        logical["thread_id"] = thread_id
    if turn_id is not None:
        logical["turn_id"] = turn_id
    identity: JsonObject = {"session_id": logical_session, "turn_id": turn_id or "turn-1"}
    if thread_id is not None:
        identity["thread_id"] = thread_id
    return {
        "type": "browser",
        "request": request,
        "identity": identity,
        "context": {
            "provenance": {
                "caller": caller,
                "source": "installer_declaration",
                "connection_id": connection_id,
                "declared_caller": caller,
                "client_info": {"name": caller, "version": "acceptance"},
            },
            "logical_identity": logical,
            "operation_identity": {
                "operation_id": operation_id,
                "request_id_fingerprint": f"fingerprint:{operation_id}",
            },
        },
    }


def concurrent_calls(calls: list[Callable[[], JsonObject]]) -> Iterator[JsonObject]:
    """Run blocking client calls concurrently and yield their results."""
    results: queue.Queue[JsonObject | BaseException] = queue.Queue()

    def run(call: Callable[[], JsonObject]) -> None:
        try:
            results.put(call())
        except BaseException as error:
            results.put(error)

    threads = [threading.Thread(target=run, args=(call,), daemon=True) for call in calls]
    for thread in threads:
        thread.start()
    for _ in threads:
        result = results.get(timeout=10)
        if isinstance(result, BaseException):
            raise result
        yield result
    for thread in threads:
        thread.join(timeout=1)


def _find_tab_id(value: Any) -> str | None:
    if isinstance(value, dict):
        for key in ("tabId", "tab_id"):
            if key in value:
                return str(value[key])
        for child in value.values():
            found = _find_tab_id(child)
            if found is not None:
                return found
    elif isinstance(value, list):
        for child in value:
            found = _find_tab_id(child)
            if found is not None:
                return found
    return None
