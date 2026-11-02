"""Tests for the DevTools-pipe extension loader in the Chrome bridge helpers."""

from __future__ import annotations

import json
import os
from collections.abc import Iterator
from contextlib import suppress
from pathlib import Path

import pytest

import _chrome_bridge


@pytest.fixture
def fake_browser() -> Iterator[tuple[int, int, int, int]]:
    to_browser_read, to_browser_write = os.pipe()
    from_browser_read, from_browser_write = os.pipe()
    fds = (to_browser_read, to_browser_write, from_browser_read, from_browser_write)
    yield fds
    for fd in fds:
        with suppress(OSError):
            os.close(fd)


def read_request(fd: int) -> dict[str, object]:
    frame = os.read(fd, 65536)
    assert frame.endswith(b"\0")
    return json.loads(frame[:-1])


def test_load_unpacked_sends_request_and_skips_unrelated_frames(
    fake_browser: tuple[int, int, int, int],
) -> None:
    to_browser_read, to_browser_write, from_browser_read, from_browser_write = fake_browser
    os.write(
        from_browser_write,
        b'{"method":"Target.attachedToTarget"}\0{"id":1,"result":{"id":"abc"}}\0',
    )

    extension_id = _chrome_bridge.load_unpacked_via_devtools_pipe(
        to_browser_write, from_browser_read, Path("/ext")
    )

    assert extension_id == "abc"
    assert read_request(to_browser_read) == {
        "id": 1,
        "method": "Extensions.loadUnpacked",
        "params": {"path": "/ext"},
    }


def test_load_unpacked_reports_protocol_error(fake_browser: tuple[int, int, int, int]) -> None:
    _, to_browser_write, from_browser_read, from_browser_write = fake_browser
    os.write(from_browser_write, b'{"id":1,"error":{"message":"Method not available."}}\0')

    with pytest.raises(RuntimeError, match="Method not available"):
        _chrome_bridge.load_unpacked_via_devtools_pipe(
            to_browser_write, from_browser_read, Path("/ext")
        )


def test_load_unpacked_reports_closed_pipe(fake_browser: tuple[int, int, int, int]) -> None:
    _, to_browser_write, from_browser_read, from_browser_write = fake_browser
    os.close(from_browser_write)

    with pytest.raises(RuntimeError, match="closed its DevTools pipe"):
        _chrome_bridge.load_unpacked_via_devtools_pipe(
            to_browser_write, from_browser_read, Path("/ext")
        )


def test_load_unpacked_times_out_without_reply(fake_browser: tuple[int, int, int, int]) -> None:
    _, to_browser_write, from_browser_read, _ = fake_browser

    with pytest.raises(TimeoutError):
        _chrome_bridge.load_unpacked_via_devtools_pipe(
            to_browser_write, from_browser_read, Path("/ext"), timeout=0.05
        )
