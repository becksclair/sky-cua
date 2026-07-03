"""Overlay-host transport and cursor-state helpers shared by the desktop harnesses.

Both the motion visual harness (``overlay_motion_animations.py``) and the KDE
live smoke (``live_agent_cursor_kde_smoke.py``) speak the same JSON-lines
protocol to ``sky-cua-overlay-host serve --socket``: one connection per
message, one request line, one reply line. They also build the same
desktop-logical ``AgentCursorState`` payload, which the host only renders when
it carries a snapshot id and a ~now ``updated_at_ms`` (a stale/zero timestamp
reads as a decayed cursor and draws nothing). Keeping both here stops the two
harnesses drifting apart on the wire contract.
"""

from __future__ import annotations

import json
import os
import signal
import socket
import time
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

# `sky-cua-overlay-host` truncated to /proc/<pid>/comm's 15-char limit.
_OVERLAY_HOST_COMM = "sky-cua-overlay"


def host_argv_matches_socket(comm: str, argv: Sequence[str], socket_path: str) -> bool:
    """Whether a process is ``sky-cua-overlay-host serve --socket <socket_path>``.

    Matches by the truncated comm plus an EXACT ``--socket <socket_path>`` argv
    pair, so it only ever identifies a host bound to *this* socket — never the
    service-owned host on a different socket (a live operator sky-cua session),
    nor an unrelated process whose command line merely mentions the name.
    """
    if comm != _OVERLAY_HOST_COMM or "serve" not in argv:
        return False
    return any(
        tok == "--socket" and i + 1 < len(argv) and argv[i + 1] == socket_path
        for i, tok in enumerate(argv)
    )


def terminate_leftover_hosts(socket_path: Path, *, exclude_pid: int | None = None) -> list[int]:
    """SIGTERM any overlay host bound to ``socket_path`` (Linux ``/proc`` only).

    Clears a host orphaned by a previous run of the SAME harness — identified by
    an exact ``--socket <socket_path>`` argv — so a service-owned host on a
    different socket is never touched. This replaces a blanket
    ``pkill -x sky-cua-overlay``, which would also kill an operator's live
    overlay host. Best-effort: unreadable ``/proc`` entries and already-dead
    pids are skipped. Returns the pids signalled.
    """
    target = str(socket_path)
    signalled: list[int] = []
    proc_root = Path("/proc")
    if not proc_root.is_dir():
        return signalled
    self_pid = os.getpid()
    for entry in proc_root.iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        if pid in (self_pid, exclude_pid):
            continue
        try:
            comm = (entry / "comm").read_text(encoding="utf-8", errors="replace").strip()
            raw = (entry / "cmdline").read_bytes()
        except OSError:
            continue
        argv = [tok for tok in raw.decode("utf-8", "replace").split("\0") if tok]
        if not host_argv_matches_socket(comm, argv, target):
            continue
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            continue
        signalled.append(pid)
    return signalled


def call_host(
    socket_path: Path,
    payload: Mapping[str, Any],
    *,
    timeout: float,
    context: str,
) -> dict[str, Any]:
    """One JSON-lines request/reply per Unix-socket CONNECTION — the transport
    contract of ``sky-cua-overlay-host serve --socket`` (connect per message).

    ``context`` labels the request in the empty-reply error so each caller
    keeps its own diagnostic wording.
    """
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(timeout)
        client.connect(str(socket_path))
        client.sendall(json.dumps(payload).encode("utf-8") + b"\n")
        buffer = b""
        while b"\n" not in buffer:
            chunk = client.recv(65536)
            if not chunk:
                break
            buffer += chunk
    raw = buffer.partition(b"\n")[0].strip()
    if not raw:
        raise RuntimeError(f"empty overlay host reply for {context}")
    reply = json.loads(raw.decode("utf-8"))
    if not isinstance(reply, dict):
        raise RuntimeError(f"overlay host reply was not an object: {reply!r}")
    return reply


def agent_cursor_state(
    point: tuple[float, float],
    *,
    sequence: int,
    snapshot_id: str,
) -> dict[str, Any]:
    """A desktop-logical cursor state the host will actually render: both
    points logical, an opaque snapshot id, and a ~now ``updated_at_ms``."""
    x, y = point
    logical = {"x": x, "y": y, "coordinate_space": "desktop_logical"}
    return {
        "visible": True,
        "sequence": sequence,
        "model_point": dict(logical),
        "native_point": dict(logical),
        "snapshot_id": snapshot_id,
        "source_action": "click",
        "updated_at_ms": int(time.time() * 1000),
    }
