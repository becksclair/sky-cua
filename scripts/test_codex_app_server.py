"""Tests for the shared codex app-server JSON-RPC client."""

from __future__ import annotations

import json
import queue
import subprocess
from collections.abc import Iterator
from typing import Any

import pytest

from _codex_app_server import AppServerExited, CodexAppServerClient


class FeedStream:
    """Blocking line stream a test can feed incrementally."""

    def __init__(self, lines: list[str] | None = None) -> None:
        self._queue: queue.Queue[str | None] = queue.Queue()
        for line in lines or []:
            self._queue.put(line)

    def feed(self, line: str) -> None:
        self._queue.put(line)

    def feed_json(self, payload: dict[str, Any]) -> None:
        self.feed(json.dumps(payload) + "\n")

    def end(self) -> None:
        self._queue.put(None)

    def __iter__(self) -> Iterator[str]:
        while True:
            item = self._queue.get()
            if item is None:
                return
            yield item


class RecordingStdin:
    def __init__(self) -> None:
        self.lines: list[str] = []
        self.flush_count = 0

    def write(self, data: str) -> int:
        self.lines.append(data)
        return len(data)

    def flush(self) -> None:
        self.flush_count += 1

    def payloads(self) -> list[dict[str, Any]]:
        return [json.loads(line) for line in self.lines]


class FakeProcess:
    def __init__(
        self,
        *,
        stdout: FeedStream | None = None,
        stderr: FeedStream | None = None,
        wait_hangs: bool = False,
    ) -> None:
        self.stdin = RecordingStdin()
        self.stdout = stdout or FeedStream()
        self.stderr = stderr or FeedStream()
        self.returncode: int | None = None
        self.terminated = False
        self.killed = False
        self._wait_hangs = wait_hangs

    def poll(self) -> int | None:
        return self.returncode

    def terminate(self) -> None:
        self.terminated = True
        if not self._wait_hangs:
            self.exit(0)

    def kill(self) -> None:
        self.killed = True
        self.exit(-9)

    def wait(self, timeout: float | None = None) -> int:
        if self.returncode is None:
            raise subprocess.TimeoutExpired(cmd="codex", timeout=timeout or 0.0)
        return self.returncode

    def exit(self, code: int) -> None:
        self.returncode = code
        self.stdout.end()
        self.stderr.end()


def make_client(process: FakeProcess) -> CodexAppServerClient:
    return CodexAppServerClient(["codex", "app-server"], process=process)


def test_request_matches_response_id_and_buffers_notifications() -> None:
    process = FakeProcess()
    process.stdout.feed_json({"method": "thread/tokenUsage/updated", "params": {}})
    process.stdout.feed_json({"jsonrpc": "2.0", "id": 1, "result": {"ok": True}})
    client = make_client(process)
    try:
        result = client.request("initialize", {"capabilities": {}}, timeout=5.0)
    finally:
        client.close()

    assert result == {"ok": True}
    assert client.notifications == [{"method": "thread/tokenUsage/updated", "params": {}}]
    sent = process.stdin.payloads()
    assert sent == [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"capabilities": {}},
        }
    ]


def test_request_ids_increment_and_non_dict_result_is_empty() -> None:
    process = FakeProcess()
    process.stdout.feed_json({"id": 1, "result": {}})
    process.stdout.feed_json({"id": 2, "result": ["not", "a", "dict"]})
    client = make_client(process)
    try:
        client.request("first", timeout=5.0)
        result = client.request("second", timeout=5.0)
    finally:
        client.close()

    assert result == {}
    sent = process.stdin.payloads()
    assert [payload["id"] for payload in sent] == [1, 2]


def test_request_error_response_raises_runtime_error() -> None:
    process = FakeProcess()
    process.stdout.feed_json({"id": 1, "error": {"code": -32600, "message": "bad"}})
    client = make_client(process)
    try:
        with pytest.raises(RuntimeError, match="plugin/install failed"):
            client.request("plugin/install", timeout=5.0)
    finally:
        client.close()


def test_request_timeout_reports_stderr_tail() -> None:
    process = FakeProcess()
    process.stderr.feed("something went sideways\n")
    client = make_client(process)
    try:
        with pytest.raises(TimeoutError) as excinfo:
            client.request("initialize", timeout=0.4)
    finally:
        client.close()

    assert "timed out waiting for initialize" in str(excinfo.value)
    assert "something went sideways" in str(excinfo.value)


def test_request_reports_process_exit_with_stderr() -> None:
    process = FakeProcess()
    process.stderr.feed("fatal: no codex home\n")
    client = make_client(process)
    process.exit(1)
    try:
        with pytest.raises(RuntimeError) as excinfo:
            client.request("initialize", timeout=5.0)
    finally:
        client.close()

    assert "exited before initialize completed" in str(excinfo.value)
    assert "fatal: no codex home" in str(excinfo.value)


def test_request_drains_response_queued_before_exit() -> None:
    process = FakeProcess()
    process.stdout.feed_json({"id": 1, "result": {"done": True}})
    client = make_client(process)
    process.exit(0)

    try:
        assert client.request("shutdown", timeout=5.0) == {"done": True}
    finally:
        client.close()


def test_non_json_stdout_is_recorded_and_skipped() -> None:
    process = FakeProcess()
    process.stdout.feed("warning: plain text line\n")
    process.stdout.feed_json({"id": 1, "result": {}})
    client = make_client(process)
    try:
        assert client.request("initialize", timeout=5.0) == {}
    finally:
        client.close()

    assert "non-json stdout: warning: plain text line" in client.stderr_text()


def test_read_message_streams_in_order_and_raises_on_eof() -> None:
    process = FakeProcess()
    process.stdout.feed_json({"method": "item/started", "params": {}})
    process.stdout.feed_json({"id": 7, "result": {}})
    client = make_client(process)
    try:
        first = client.read_message(timeout=5.0)
        second = client.read_message(timeout=5.0)
        process.exit(0)
        with pytest.raises(AppServerExited, match="app-server exited unexpectedly"):
            client.read_message(timeout=5.0)
    finally:
        client.close()

    assert first == {"method": "item/started", "params": {}}
    assert second == {"id": 7, "result": {}}


def test_read_message_times_out_while_process_is_quiet() -> None:
    process = FakeProcess()
    client = make_client(process)
    try:
        with pytest.raises(TimeoutError, match="timed out waiting for app-server message"):
            client.read_message(timeout=0.3)
    finally:
        client.close()


def test_notify_and_respond_payload_shapes() -> None:
    process = FakeProcess()
    client = make_client(process)
    try:
        client.notify("initialized", {})
        client.notify("bare")
        client.respond(42, {"decision": "accept"})
    finally:
        client.close()

    sent = process.stdin.payloads()
    assert sent[0] == {"jsonrpc": "2.0", "method": "initialized", "params": {}}
    assert sent[1] == {"jsonrpc": "2.0", "method": "bare"}
    assert sent[2] == {"jsonrpc": "2.0", "id": 42, "result": {"decision": "accept"}}
    assert all("id" not in payload for payload in sent[:2])


def test_initialize_sends_request_then_initialized_notification() -> None:
    process = FakeProcess()
    process.stdout.feed_json({"id": 1, "result": {"userAgent": "codex"}})
    client = make_client(process)
    try:
        result = client.initialize(
            client_name="sky-cua-release-deploy",
            client_version="0",
            timeout=5.0,
        )
    finally:
        client.close()

    assert result == {"userAgent": "codex"}
    sent = process.stdin.payloads()
    assert sent[0]["method"] == "initialize"
    assert sent[0]["params"]["clientInfo"] == {
        "name": "sky-cua-release-deploy",
        "version": "0",
    }
    assert sent[1] == {"jsonrpc": "2.0", "method": "initialized", "params": {}}


def test_close_terminates_and_kills_when_wait_hangs() -> None:
    process = FakeProcess(wait_hangs=True)
    client = make_client(process)
    client.close()

    assert process.terminated
    assert process.killed


def test_close_skips_signals_when_process_already_exited() -> None:
    process = FakeProcess()
    client = make_client(process)
    process.exit(0)
    client.close()

    assert not process.terminated
    assert not process.killed


def test_stderr_text_joins_lines_with_trailing_newline() -> None:
    process = FakeProcess()
    process.stderr.feed("line one\n")
    process.stderr.feed("line two\n")
    client = make_client(process)
    process.exit(0)
    client.close()

    assert client.stderr_text() == "line one\nline two\n"
    assert client.stderr_tail() == "line one\nline two"
