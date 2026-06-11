"""Tests for the Chrome host client smoke helpers."""

from __future__ import annotations

import json
import socket
import sys
import threading
import time
from pathlib import Path

import pytest

import _mcp_stdio
import live_chrome_host_client_smoke


def test_chrome_host_smoke_finds_service_process_by_socket_env(tmp_path: Path) -> None:
    proc_root = tmp_path / "proc"
    matching_proc = proc_root / "123"
    matching_proc.mkdir(parents=True)
    (matching_proc / "environ").write_bytes(
        b"PATH=/usr/bin\0SKY_CUA_SERVICE_SOCKET_PATH=/tmp/sky-cua-smoke.sock\0"
    )

    ignored_proc = proc_root / "456"
    ignored_proc.mkdir()
    (ignored_proc / "environ").write_bytes(b"SKY_CUA_SERVICE_SOCKET_PATH=/tmp/other.sock\0")

    assert _mcp_stdio.process_ids_with_env_var(
        "SKY_CUA_SERVICE_SOCKET_PATH",
        "/tmp/sky-cua-smoke.sock",
        proc_root=proc_root,
    ) == [123]


def test_tab_list_proof_redacts_titles_and_urls() -> None:
    proof = live_chrome_host_client_smoke.redacted_tab_list_proof(
        {
            "id": "client-get-user-tabs-mcp-proof",
            "result": {
                "tabs": [
                    {
                        "id": 42,
                        "title": "Private tab title",
                        "url": "https://private.example.test/path",
                    }
                ]
            },
        },
        expected_tab_id=42,
    )

    assert proof == {
        "id": "client-get-user-tabs-mcp-proof",
        "has_result": True,
        "expected_tab_id": 42,
        "expected_tab_present": True,
        "tabs_count": 1,
    }
    assert "Private tab title" not in json.dumps(proof)
    assert "private.example.test" not in json.dumps(proof)


def test_expected_tab_present_accepts_mcp_tab_id_shape() -> None:
    tabs: list[object] = [
        {"tab_id": "7", "title": "Private tab title", "url": "https://private.example.test"}
    ]

    assert live_chrome_host_client_smoke.expected_tab_present(tabs, 7) is True
    assert live_chrome_host_client_smoke.expected_tab_present(tabs, 8) is False
    assert live_chrome_host_client_smoke.expected_tab_present(tabs, None) is None


def write_cursor_diff_fixture(path: Path, points: list[tuple[int, int]]) -> None:
    from PIL import Image, ImageDraw

    image = Image.new("RGB", (400, 300), "white")
    draw = ImageDraw.Draw(image)
    for x, y in points:
        draw.rectangle((x - 4, y - 4, x + 4, y + 4), fill="black")
    image.save(path)


def write_cursor_rectangle_fixture(path: Path, rectangles: list[tuple[int, int, int, int]]) -> None:
    from PIL import Image, ImageDraw

    image = Image.new("RGB", (400, 300), "white")
    draw = ImageDraw.Draw(image)
    for x, y, width, height in rectangles:
        draw.rectangle((x, y, x + width - 1, y + height - 1), fill="black")
    image.save(path)


def test_cursor_diff_accepts_localized_cursor_change(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    write_cursor_diff_fixture(after, [(100, 80)])

    result = live_chrome_host_client_smoke.assert_localized_cursor_diff(
        before,
        after,
        target_x_css=100,
        target_y_css=80,
        device_pixel_ratio=1,
    )

    assert result["ok"] is True
    assert result["near_changed_pixels"] == 81


def test_cursor_diff_accepts_compact_prior_cursor_disappearing(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [(280, 220)])
    write_cursor_diff_fixture(after, [(100, 80)])

    result = live_chrome_host_client_smoke.assert_localized_cursor_diff(
        before,
        after,
        target_x_css=100,
        target_y_css=80,
        device_pixel_ratio=1,
    )

    assert result["ok"] is True
    assert result["near_changed_pixels"] == 81
    assert result["outside_changed_pixels"] == 81


def test_cursor_diff_accepts_full_size_prior_cursor_disappearing(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_rectangle_fixture(before, [(260, 210, 46, 48)])
    write_cursor_diff_fixture(after, [(100, 80)])

    result = live_chrome_host_client_smoke.assert_localized_cursor_diff(
        before,
        after,
        target_x_css=100,
        target_y_css=80,
        device_pixel_ratio=1,
    )

    assert result["ok"] is True
    assert result["near_changed_pixels"] == 81
    assert result["outside_changed_pixels"] == 2208


def test_cursor_diff_rejects_missing_visible_change(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    write_cursor_diff_fixture(after, [])

    with pytest.raises(AssertionError, match="enough changed pixels"):
        live_chrome_host_client_smoke.assert_localized_cursor_diff(
            before,
            after,
            target_x_css=100,
            target_y_css=80,
            device_pixel_ratio=1,
        )


def test_cursor_diff_rejects_far_away_change(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    write_cursor_diff_fixture(after, [(320, 240)])

    with pytest.raises(AssertionError, match="enough changed pixels"):
        live_chrome_host_client_smoke.assert_localized_cursor_diff(
            before,
            after,
            target_x_css=100,
            target_y_css=80,
            device_pixel_ratio=1,
        )


def test_cursor_diff_rejects_broad_unrelated_change(tmp_path: Path) -> None:
    from PIL import Image, ImageDraw

    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    image = Image.new("RGB", (400, 300), "white")
    draw = ImageDraw.Draw(image)
    draw.rectangle((96, 76, 104, 84), fill="black")
    draw.rectangle((250, 20, 390, 260), fill="black")
    image.save(after)

    with pytest.raises(AssertionError, match="outside the cursor region"):
        live_chrome_host_client_smoke.assert_localized_cursor_diff(
            before,
            after,
            target_x_css=100,
            target_y_css=80,
            device_pixel_ratio=1,
        )


def test_cursor_diff_scales_css_coordinates_by_device_pixel_ratio(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    write_cursor_diff_fixture(after, [(200, 160)])

    result = live_chrome_host_client_smoke.assert_localized_cursor_diff(
        before,
        after,
        target_x_css=100,
        target_y_css=80,
        device_pixel_ratio=2,
    )

    assert result["ok"] is True
    assert result["target_pixel"] == {"x": 200, "y": 160}


def test_chrome_host_smoke_accepts_same_origin_web_redirects() -> None:
    assert live_chrome_host_client_smoke.same_requested_origin(
        "http://www.example.com/article",
        "https://example.com/article",
    )


def test_chrome_host_smoke_rejects_unexpected_navigation_origin() -> None:
    assert not live_chrome_host_client_smoke.same_requested_origin(
        "https://example.com/article",
        "chrome-error://chromewebdata/",
    )
    assert not live_chrome_host_client_smoke.same_requested_origin(
        "https://example.com/article",
        "https://other.example.com/article",
    )


def test_chrome_host_smoke_accepts_successful_turn_ended_response() -> None:
    stderr = (
        "[com.openai.codexextension] received unmatched Chrome response "
        "id=native-turn-ended:smoke-session:smoke-turn "
        'payload={"jsonrpc":"2.0","id":"native-turn-ended:smoke-session:smoke-turn"}'
    )

    response = live_chrome_host_client_smoke.turn_ended_response_from_stderr(stderr)

    assert live_chrome_host_client_smoke.turn_ended_response_was_successful(response)


def test_chrome_host_smoke_rejects_turn_ended_error_response() -> None:
    stderr = (
        "[com.openai.codexextension] received unmatched Chrome response "
        "id=native-turn-ended:smoke-session:smoke-turn "
        'payload={"jsonrpc":"2.0","id":"native-turn-ended:smoke-session:smoke-turn",'
        '"error":{"code":-32601,"message":"No handler registered for method"}}'
    )

    response = live_chrome_host_client_smoke.turn_ended_response_from_stderr(stderr)

    assert response is not None
    assert not live_chrome_host_client_smoke.turn_ended_response_was_successful(response)


def test_chrome_mcp_client_times_out_when_process_sends_no_frame() -> None:
    client = live_chrome_host_client_smoke.McpClient(
        [sys.executable, "-c", "import time; time.sleep(60)"],
        extra_env={},
        read_timeout=0.05,
    )

    started_at = time.monotonic()
    with pytest.raises(RuntimeError, match="timed out while reading MCP headers"):
        client._read_message()

    assert time.monotonic() - started_at < 2
    assert client.proc.poll() is not None


def test_chrome_native_request_uses_aggregate_timeout_for_pings() -> None:
    client_sock, server_sock = socket.socketpair()
    stop = threading.Event()

    def serve_pings() -> None:
        try:
            live_chrome_host_client_smoke.read_native_frame(server_sock, timeout=1)
            while not stop.is_set():
                try:
                    live_chrome_host_client_smoke.write_native_frame(
                        server_sock,
                        {"jsonrpc": "2.0", "id": "ping", "method": "ping"},
                    )
                except OSError:
                    break
                time.sleep(0.01)
        finally:
            server_sock.close()

    thread = threading.Thread(target=serve_pings)
    thread.start()
    started_at = time.monotonic()
    try:
        with pytest.raises(TimeoutError, match=r"native request getInfo.*timed out"):
            live_chrome_host_client_smoke.native_request(
                client_sock,
                "getInfo",
                {},
                timeout=0.05,
                request_id="aggregate-timeout",
            )
        assert time.monotonic() - started_at < 2
    finally:
        stop.set()
        client_sock.close()
        thread.join(timeout=2)
