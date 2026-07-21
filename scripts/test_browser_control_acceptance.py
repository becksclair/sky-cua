"""Process-level deterministic acceptance for the persistent browser owner."""

from __future__ import annotations

import json
import queue
import socket
import stat
import threading
import time
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest

from _browser_control_acceptance import (
    BrowserControlHarness,
    FakeNativeHost,
    browser_request,
    debug_service_binary,
)

REPO_ROOT = Path(__file__).resolve().parents[1]


def _require_debug_service() -> Path:
    binary = debug_service_binary(REPO_ROOT)
    if not binary.is_file():
        pytest.skip(
            f"deterministic browser-control acceptance requires {binary}; "
            "build it with `cargo build -p sky-cua-service`"
        )
    return binary


@pytest.fixture(params=["hybrid", "strict"])
def harness(tmp_path: Path, request: pytest.FixtureRequest) -> Iterator[BrowserControlHarness]:
    _require_debug_service()
    with BrowserControlHarness(
        REPO_ROOT, tmp_path / str(request.param), mode=str(request.param)
    ) as value:
        yield value


def _host(harness: BrowserControlHarness) -> FakeNativeHost:
    assert harness.host is not None
    return harness.host


def _raw_params(session: str, tab: str | None = None) -> dict[str, Any]:
    params: dict[str, Any] = {
        "session_id": session,
        "thread_id": f"thread-{session}",
        "turn_id": f"turn-{session}",
    }
    if tab is not None:
        params["tabId"] = tab
    return params


def _group(snapshot: dict[str, Any], group_id: str) -> dict[str, Any]:
    return next(group for group in snapshot["scheduler"]["groups"] if group["group_id"] == group_id)


def test_modes_share_one_actor_preserve_raw_wire_and_heartbeat(
    harness: BrowserControlHarness,
) -> None:
    host = _host(harness)
    with harness.codex_client() as codex:
        info = codex.call("getInfo", _raw_params("codex-info"))
        assert info == {
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": 7,
                "browser": "Brave",
                "codexAppBuildFlavor": "acceptance-fake",
                "capabilities": {
                    "tab": [
                        {
                            "id": "botDetection",
                            "description": (
                                "Report detected anti-bot challenges through the sky-cua daemon"
                            ),
                        },
                        {
                            "id": "browserAuth",
                            "description": (
                                "Request a sky-cua daemon browser-authentication handoff"
                            ),
                        },
                    ]
                },
                "metadata": {
                    "codexSessionId": "codex-info",
                    "skyCuaBridgeTransport": "extension_native_host",
                    "skyCuaCallerProvenance": "codex_desktop",
                    "skyCuaIdentitySynthetic": False,
                },
                "nested": {"preserved": True},
            },
        }
        host.wait_for_connections(1)
        assert host.connection_count == 1
        assert host.hellos[0]["method"] == "skyCuaHost/hello"
        assert host.hellos[0]["params"]["owner_mode"] == harness.mode

        error = codex.call("forceError", _raw_params("codex-error"))
        assert error == {
            "jsonrpc": "2.0",
            "id": 2,
            "error": {
                "code": -32123,
                "message": "fake upstream exact error",
                "data": {"x": 1},
            },
        }

        notification = {
            "jsonrpc": "2.0",
            "method": "Browser.downloadProgress",
            "params": {"guid": "exact-guid", "state": "inProgress", "receivedBytes": 17},
        }
        host.send(notification)
        assert codex.receive() == notification

        # The actor heartbeat runs independently while every client is idle.
        deadline = time.monotonic() + 2.5
        while time.monotonic() < deadline:
            transcript = harness.transcript.path.read_text(encoding="utf-8")
            if '"method": "ping"' in transcript:
                break
            time.sleep(0.05)
        else:
            pytest.fail("persistent actor emitted no idle heartbeat")

    assert host.connection_count == 1
    assert harness.stderr_path.is_file()
    records = [json.loads(line) for line in harness.transcript.path.read_text().splitlines()]
    assert {record["lane"] for record in records} >= {"native_host", "codex"}


def test_explicit_ordinary_provenance_groups_and_foreign_ownership(
    harness: BrowserControlHarness,
) -> None:
    host = _host(harness)
    callers = ["open_claw", "open_code", "pi", "direct_mcp"]
    clients = [harness.ordinary_client(f"ordinary-{caller}") for caller in callers]
    try:
        for index, (caller, client) in enumerate(zip(callers, clients, strict=True)):
            response = client.call(
                browser_request(
                    caller,
                    f"connection-{caller}",
                    f"operation-{caller}",
                    {"type": "open", "url": f"https://{caller}.test"},
                    session_id=f"session-{caller}",
                    thread_id=f"thread-{index}",
                )
            )
            assert response.get("type") == "browser", response

        assert host.connection_count == 1
        operations = {
            request["params"]["_sky_cua_host_request"]["operation_id"]
            for request in host.requests
            if request.get("method") == "createTab"
        }
        assert len(operations) == len(callers)
        for caller in callers:
            assert any(
                operation.startswith(f"operation-{caller}:bridge-subrequest:")
                for operation in operations
            )

        owner = clients[0]
        foreign = clients[1]
        claimed = owner.call(
            browser_request(
                "open_claw",
                "connection-open_claw",
                "claim-owner",
                {"type": "claim_tab", "tab_id": "101"},
                session_id="session-open_claw",
            )
        )
        assert claimed.get("type") == "browser", claimed
        rejected = foreign.call(
            browser_request(
                "open_code",
                "connection-open_code",
                "claim-foreign",
                {"type": "claim_tab", "tab_id": "101"},
                session_id="session-open_code",
            )
        )
        assert rejected["type"] == "error"
        assert rejected["code"] == "BrowserOwnershipRejected"
        assert "another logical browser group" in rejected["message"]
    finally:
        for client in clients:
            client.close()


def test_raw_same_tab_fifo_separate_tab_overlap_and_cancellation(
    harness: BrowserControlHarness,
) -> None:
    host = _host(harness)

    def responder(request: dict[str, Any]) -> dict[str, Any] | None:
        if request.get("method") == "hold":
            return None
        return host._default_response(request)

    host.responder = responder
    with harness.codex_client() as codex:
        assert "result" in codex.call("claimTab", _raw_params("group-a", "101"))
        assert "result" in codex.call("claimTab", _raw_params("group-a", "102"))

        first_id = codex.send_request("hold", _raw_params("group-a", "101"))
        host.wait_for_requests(1, method="hold")
        same_tab_id = codex.send_request("hold", _raw_params("group-a", "101"))
        other_tab_id = codex.send_request("hold", _raw_params("group-a", "102"))

        first_wave = host.wait_for_requests(2, method="hold")
        dispatched_tabs = [request["params"]["tabId"] for request in first_wave]
        assert dispatched_tabs.count("101") == 1
        assert dispatched_tabs.count("102") == 1

        for request in first_wave:
            host.send(
                {
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": {"tabId": request["params"]["tabId"], "released": True},
                }
            )
        all_held = host.wait_for_requests(3, method="hold")
        third = all_held[2]
        assert third["params"]["tabId"] == "101"
        host.send(
            {
                "jsonrpc": "2.0",
                "id": third["id"],
                "result": {"tabId": "101", "released": True},
            }
        )
        responses = {codex.receive()["id"], codex.receive()["id"], codex.receive()["id"]}
        assert responses == {first_id, same_tab_id, other_tab_id}

        # Fill the same-tab lane, queue a second operation, then disconnect.
        before = len(host.wait_for_requests(3, method="hold"))
        codex.send_request("hold", _raw_params("group-a", "101"))
        host.wait_for_requests(before + 1, method="hold")
        codex.send_request("neverDispatch", _raw_params("group-a", "101"))
    time.sleep(0.2)
    assert not any(request.get("method") == "neverDispatch" for request in host.requests)
    with harness.codex_client() as replacement:
        orphaned = replacement.call("claimTab", _raw_params("replacement-during-grace", "101"))
        assert orphaned["error"]["code"] == -32072
        assert "another logical browser group" in orphaned["error"]["message"]
    with harness.ordinary_client("orphan-introspection") as status_client:
        snapshot = status_client.call(
            browser_request(
                "direct_mcp",
                "orphan-introspection",
                "orphan-status",
                {"type": "status"},
            )
        )
    assert "orphan" in json.dumps(snapshot).lower()


def test_ordinary_cancellation_before_dispatch_is_owner_scoped(
    harness: BrowserControlHarness,
) -> None:
    host = _host(harness)
    owner = harness.ordinary_client("ordinary-cancel-owner")
    queued = harness.ordinary_client("ordinary-cancel-queued")
    control = harness.ordinary_client("ordinary-cancel-control")
    responses: queue.Queue[dict[str, Any]] = queue.Queue()
    held_request: dict[str, Any] | None = None

    try:
        claim = owner.call(
            browser_request(
                "direct_mcp",
                "cancel-connection",
                "cancel-claim",
                {"type": "claim_tab", "tab_id": "101"},
            )
        )
        assert claim.get("type") == "browser", claim

        def responder(request: dict[str, Any]) -> dict[str, Any] | None:
            nonlocal held_request
            operation = (
                request.get("params", {}).get("_sky_cua_host_request", {}).get("operation_id", "")
            )
            if operation.startswith("cancel-first:bridge-subrequest:") and held_request is None:
                held_request = request
                return None
            return host._default_response(request)

        host.responder = responder

        first_thread = threading.Thread(
            target=lambda: responses.put(
                owner.call(
                    browser_request(
                        "direct_mcp",
                        "cancel-connection",
                        "cancel-first",
                        {
                            "type": "navigate",
                            "tab_id": "101",
                            "url": "https://first.test",
                        },
                    )
                )
            ),
            daemon=True,
        )
        first_thread.start()
        deadline = time.monotonic() + 5
        while held_request is None and time.monotonic() < deadline:
            time.sleep(0.01)
        assert held_request is not None

        second_thread = threading.Thread(
            target=lambda: responses.put(
                queued.call(
                    browser_request(
                        "direct_mcp",
                        "cancel-connection",
                        "cancel-before-dispatch",
                        {
                            "type": "navigate",
                            "tab_id": "101",
                            "url": "https://never.test",
                        },
                    )
                )
            ),
            daemon=True,
        )
        second_thread.start()
        time.sleep(0.1)
        cancelled = control.call(
            {
                "type": "cancel_browser_operation",
                "connection_id": "cancel-connection",
                "operation_id": "cancel-before-dispatch",
                "reason": "acceptance cancellation",
            }
        )
        assert cancelled == {
            "type": "error",
            "code": "BrowserCancellationAcknowledged",
            "message": "CancelledBeforeDispatch",
        }
        assert not any(
            request.get("params", {})
            .get("_sky_cua_host_request", {})
            .get("operation_id", "")
            .startswith("cancel-before-dispatch:")
            for request in host.requests
        )

        host.send(
            {
                "jsonrpc": "2.0",
                "id": held_request["id"],
                "result": {"frameId": "fake-frame"},
            }
        )
        first_thread.join(timeout=5)
        second_thread.join(timeout=5)
        assert not first_thread.is_alive()
        assert not second_thread.is_alive()
        assert responses.qsize() == 2
    finally:
        owner.close()
        queued.close()
        control.close()


@pytest.mark.parametrize("mode", ["hybrid", "strict"])
def test_connection_only_actor_reconnect_emits_browser_loss_semantics(
    tmp_path: Path, mode: str
) -> None:
    _require_debug_service()
    with BrowserControlHarness(
        REPO_ROOT,
        tmp_path / f"reconnect-{mode}",
        mode=mode,
        host_stability="connection_only",
    ) as harness:
        host = _host(harness)
        with harness.codex_client() as codex:
            assert "result" in codex.call("claimTab", _raw_params("before-reconnect", "101"))
            rejected = codex.call("claimTab", _raw_params("foreign-before", "101"))
            assert rejected["error"]["code"] == -32072
            assert "another logical browser group" in rejected["error"]["message"]

            host.disconnect_all()
            host.wait_for_connections(2)
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                accepted = codex.call("claimTab", _raw_params("foreign-after", "101"))
                if "result" in accepted:
                    break
                time.sleep(0.05)
            else:
                pytest.fail("actor reconnect did not clear browser-lost tab ownership")

        assert host.connection_count == 2


def test_stable_browser_actor_reconnect_preserves_ownership(
    harness: BrowserControlHarness,
) -> None:
    host = _host(harness)
    with harness.codex_client() as codex:
        assert "result" in codex.call("claimTab", _raw_params("stable-owner", "101"))
        before = harness.control_plane_status("stable-before")
        group_id = before["scheduler"]["groups"][0]["group_id"]
        owned_before = _group(before, group_id)
        actor_before = before["actors"][0]
        request_count = len(host.requests)

        host.disconnect_all()
        host.wait_for_connections(2)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            after = harness.control_plane_status("stable-after")
            actor_after = after["actors"][0]
            if (
                actor_after["state"] == "ready"
                and actor_after["actor_generation"] > actor_before["actor_generation"]
            ):
                break
            time.sleep(0.05)
        else:
            pytest.fail("stable browser actor did not reconnect")

        assert actor_after["browser_instance_id"] == actor_before["browser_instance_id"]
        assert actor_after["reconnect_count"] == actor_before["reconnect_count"] + 1
        assert _group(after, group_id) == owned_before
        rejected = codex.call("claimTab", _raw_params("stable-foreign", "101"))
        assert rejected["error"]["code"] == -32072
        assert "another logical browser group" in rejected["error"]["message"]
        assert len(host.requests) == request_count


def test_daemon_restart_recovers_suspended_group_without_replay(
    harness: BrowserControlHarness,
) -> None:
    host = _host(harness)
    with harness.ordinary_client("restart-owner") as owner:
        claimed = owner.call(
            browser_request(
                "direct_mcp",
                "restart-owner",
                "restart-claim",
                {"type": "claim_tab", "tab_id": "101"},
                session_id="restart-session",
            )
        )
        assert claimed.get("type") == "browser", claimed
        opened = owner.call(
            browser_request(
                "direct_mcp",
                "restart-owner",
                "restart-open",
                {"type": "open", "url": "https://restart-recovery.test"},
                session_id="restart-session",
            )
        )
        assert opened.get("type") == "browser", opened

    before = harness.control_plane_status("restart-before")
    group_before = before["scheduler"]["groups"][0]
    generation_before = before["daemon_generation"]
    request_count = len(host.requests)
    hello_count = len(host.hellos)
    process_before = harness.process
    assert process_before is not None
    pid_before = process_before.pid

    deadline = time.monotonic() + 5
    while not harness.journal_path.is_file() and time.monotonic() < deadline:
        time.sleep(0.02)
    assert harness.journal_path == (
        harness.state_home / "sky-cua" / "browser-control-recovery-v1.json"
    )
    assert harness.journal_path.is_file()
    assert stat.S_IMODE(harness.journal_path.stat().st_mode) == 0o600
    journal = json.loads(harness.journal_path.read_text(encoding="utf-8"))
    journal_group = next(
        group for group in journal["groups"] if group["group_id"] == group_before["group_id"]
    )
    assert journal_group["prior_fence"] == group_before["fence"]
    assert journal_group["unresolved_mutation"] is False

    after = harness.restart_service()
    process_after = harness.process
    assert process_after is not None
    assert process_after.pid != pid_before
    assert before["daemon_generation"] == generation_before
    assert after["daemon_generation"] != generation_before
    assert len(host.hellos) >= hello_count + 1
    assert host.browser_instance_id == "fake-browser-1"

    group_after = _group(after, group_before["group_id"])
    assert group_after["lease_state"] == "suspended"
    assert group_after["admission_state"] == "suspended"
    assert group_after["fence"] == group_before["fence"] + 1
    assert group_after["members"] == group_before["members"]
    assert after["scheduler"].get("recent_operations", []) == []
    assert len(host.requests) == request_count

    with harness.ordinary_client("restart-foreign") as foreign:
        rejected = foreign.call(
            browser_request(
                "direct_mcp",
                "restart-foreign",
                "restart-foreign-claim",
                {"type": "claim_tab", "tab_id": "101"},
                session_id="foreign-session",
            )
        )
    assert rejected["type"] == "error"
    assert rejected["code"] == "BrowserOwnershipRejected"
    assert "another logical browser group" in rejected["message"]
    assert len(host.requests) == request_count


def test_newest_stale_candidate_fails_over_without_legacy_connection(tmp_path: Path) -> None:
    _require_debug_service()
    with BrowserControlHarness(REPO_ROOT, tmp_path / "failover", mode="strict") as harness:
        host = _host(harness)
        stale = harness.runtime_dir / "extension-newest-stale.sock"
        stale_listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        stale_listener.bind(str(stale))
        stale_listener.close()
        # The newest candidate is a real socket inode with no listener.
        with harness.codex_client() as codex:
            assert "result" in codex.call("getInfo", _raw_params("failover"))
        assert host.connection_count == 1
        assert not any(
            record.get("lane") == "legacy"
            for record in map(json.loads, harness.transcript.path.read_text().splitlines())
        )
