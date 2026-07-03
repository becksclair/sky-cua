"""Tests for the shared overlay-host helpers.

Focus: the socket-scoped leftover-host matcher, whose whole point is to never
match a service-owned host on a different socket (a live operator session).
"""

from __future__ import annotations

from pathlib import Path

import _overlay_host

HARNESS_SOCK = "/tmp/sky-cua-overlay-motion.sock"
SERVICE_SOCK = "/run/user/1000/sky-cua/agent-cursor.sock"


def _host_argv(socket_path: str) -> list[str]:
    return ["sky-cua-overlay-host", "serve", "--socket", socket_path]


def test_matches_host_on_our_socket() -> None:
    assert _overlay_host.host_argv_matches_socket(
        "sky-cua-overlay", _host_argv(HARNESS_SOCK), HARNESS_SOCK
    )


def test_does_not_match_service_host_on_a_different_socket() -> None:
    # The safety property: a running operator service host on the canonical
    # socket must NEVER be matched when we scope to our private socket.
    assert not _overlay_host.host_argv_matches_socket(
        "sky-cua-overlay", _host_argv(SERVICE_SOCK), HARNESS_SOCK
    )


def test_does_not_match_wrong_comm() -> None:
    # A python harness or shell whose command line mentions the socket is not
    # an overlay host and must never be signalled.
    assert not _overlay_host.host_argv_matches_socket(
        "python3",
        ["python3", "overlay_motion_animations.py", "--socket", HARNESS_SOCK],
        HARNESS_SOCK,
    )


def test_does_not_match_without_serve_subcommand() -> None:
    assert not _overlay_host.host_argv_matches_socket(
        "sky-cua-overlay", ["sky-cua-overlay-host", "--socket", HARNESS_SOCK], HARNESS_SOCK
    )


def test_does_not_match_dangling_socket_flag() -> None:
    assert not _overlay_host.host_argv_matches_socket(
        "sky-cua-overlay", ["sky-cua-overlay-host", "serve", "--socket"], HARNESS_SOCK
    )


def test_does_not_match_socket_as_bare_token() -> None:
    # The path present but not as the value of --socket (e.g. a different flag)
    # does not match — only an exact --socket <path> pair does.
    assert not _overlay_host.host_argv_matches_socket(
        "sky-cua-overlay", ["sky-cua-overlay-host", "serve", "--log", HARNESS_SOCK], HARNESS_SOCK
    )


def test_terminate_scans_proc_and_signals_only_matching(monkeypatch, tmp_path: Path) -> None:
    # Build a fake /proc with three pids: our host, a service host on a
    # different socket, and a python harness. Only the first must be signalled.
    proc = tmp_path / "proc"
    layout = {
        "101": ("sky-cua-overlay", _host_argv(HARNESS_SOCK)),
        "202": ("sky-cua-overlay", _host_argv(SERVICE_SOCK)),
        "303": ("python3", ["python3", "overlay_motion_animations.py"]),
        "not-a-pid": ("junk", ["junk"]),
    }
    for name, (comm, argv) in layout.items():
        d = proc / name
        d.mkdir(parents=True)
        (d / "comm").write_text(comm + "\n", encoding="utf-8")
        (d / "cmdline").write_bytes(b"\0".join(a.encode() for a in argv) + b"\0")

    monkeypatch.setattr(
        _overlay_host, "Path", lambda p="/proc": proc if str(p) == "/proc" else Path(p)
    )
    killed: list[int] = []
    monkeypatch.setattr(_overlay_host.os, "kill", lambda pid, sig: killed.append(pid))
    monkeypatch.setattr(_overlay_host.os, "getpid", lambda: 999)

    signalled = _overlay_host.terminate_leftover_hosts(Path(HARNESS_SOCK))

    assert signalled == [101]
    assert killed == [101]
