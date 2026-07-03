#!/usr/bin/env python3
"""Overlay motion-animation visual test harness for the DESKTOP agent cursor.

Desktop analogue of ``scripts/overlay_pointer_animations.py``: drives the wgpu
layer-shell overlay host DIRECTLY over its JSON-lines Unix-socket protocol
(one connection per message) and records the desktop so the vehicle-steering
glide, mid-flight redirect curves, eased nose rotation, arrival-gated tap
ripple, and swipe trail can be reviewed by eye. This is a *visual* harness: it
produces a recording and frame contact-sheets rather than a pass/fail line
(the structured glide check lives in
``live_agent_cursor_kde_smoke.py --mode layer-shell-motion-glide``).

- Target: ``target/release/sky-cua-overlay-host serve --socket
  /tmp/sky-cua-overlay-motion.sock`` — a private socket owned by this process.
  NO ``sky-cua-service``, no operator daemon, and no real input is ever
  dispatched; only the overlay's visuals move.
- Recording: the KDE ScreenCast portal (``scripts/_kde_screencast.py``) piped
  into ``gst-launch-1.0`` for an MP4 (primary), or a ~2 fps ``spectacle``
  stills loop (fallback); the harness always prints which recorder ran. The
  first portal run shows one KDE share dialog; the restore token is persisted
  under the gitignored artifacts dir so later runs are silent.
- Artifacts: ``artifacts/overlay-motion-animations/`` — the raw MP4 (or
  stills) plus a contact sheet per run. Recordings capture the operator's
  LIVE DESKTOP: they are sensitive and must never be committed.
- Offline: ``--offline`` skips the live desktop entirely — it runs the gated
  ``capture_motion_frames_when_requested`` renderer test (deterministic dense
  frames from the real motion driver) and montages the result under
  ``artifacts/overlay-motion-animations/offline/``.

Usage:
  uv run python scripts/overlay_motion_animations.py
  uv run python scripts/overlay_motion_animations.py --scenario redirect
  uv run python scripts/overlay_motion_animations.py --recorder stills
  uv run python scripts/overlay_motion_animations.py --offline
  uv run python scripts/overlay_motion_animations.py --build

Safety: the host is spawned, driven, shut down (protocol ``shutdown`` +
SIGTERM), and reaped by this single process. A host orphaned by a previous run
is cleared by socket scope — only a host bound to this harness's own private
socket is signalled (``_overlay_host.terminate_leftover_hosts``), so an
operator's live service-owned overlay host on a different socket is never
touched.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import math
import os
import secrets
import shutil
import subprocess
import tempfile
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import _kde_screencast
import _overlay_host
import _session_lock
import deploy_freshness
from _contact_sheets import make_contact_sheet, montage_frames

REPO_ROOT = Path(__file__).resolve().parents[1]
HOST_BIN = REPO_ROOT / "target" / "release" / "sky-cua-overlay-host"
ARTIFACTS_DIR = REPO_ROOT / "artifacts" / "overlay-motion-animations"
SOCKET_PATH = Path("/tmp/sky-cua-overlay-motion.sock")
RESTORE_TOKEN_PATH = ARTIFACTS_DIR / ".screencast-restore-token"
OVERLAY_HOST_PROTOCOL_VERSION = 2
#: Opaque to the host; required alongside a fresh ``updated_at_ms`` or the
#: cursor reads as decayed and renders nothing.
SNAPSHOT_ID = "overlay-motion-animations"
DEFAULT_SCENARIOS = ["corners", "redirect", "swipes", "tap_settle"]


# ---------------------------------------------------------------------------
# Move scenarios (pure functions of the logical screen size, unit-testable)
# ---------------------------------------------------------------------------


@dataclass
class Move:
    """One scripted overlay action in desktop-logical pixels.

    ``kind`` is ``"glide"`` (a ``set_cursor`` retarget the mover sails to),
    ``"tap"`` (an ``animate_gesture`` Tap, one point) or ``"swipe"`` (an
    ``animate_gesture`` Drag, start and end). ``pause_s`` is the dwell held
    after dispatching so the animation can play — or, for the redirect and
    fast-flick scenarios, a short delay so the next move retargets the cursor
    mid-flight and the momentum bows the path.
    """

    kind: str
    points: list[tuple[float, float]]
    pause_s: float
    duration_ms: int = 0


def corner_moves(w: float, h: float, margin: float = 0.18, pause_s: float = 1.1) -> list[Move]:
    """Big corner-to-corner glides: long sails that curve, rotate, then settle."""
    fractions = [
        (margin, margin),
        (1.0 - margin, 1.0 - margin),
        (1.0 - margin, margin),
        (margin, 1.0 - margin),
        (0.5, 0.5),
    ]
    return [Move("glide", [(fx * w, fy * h)], pause_s) for fx, fy in fractions]


def redirect_moves(w: float, h: float, pause_s: float = 0.32) -> list[Move]:
    """Rapid retargets that fire faster than a glide settles, so the cursor is
    redirected mid-flight and the momentum bows the path hard — the signature
    curve evidence."""
    fractions = [
        (0.20, 0.25),
        (0.80, 0.30),
        (0.25, 0.75),
        (0.78, 0.72),
        (0.50, 0.20),
        (0.50, 0.80),
        (0.20, 0.50),
        (0.80, 0.50),
    ]
    return [Move("glide", [(fx * w, fy * h)], pause_s) for fx, fy in fractions]


def swipe_moves(w: float, h: float, pause_s: float = 2.8) -> list[Move]:
    """Diagonal and horizontal drags: the cursor sails to the start, then the
    feedback chases the moving head while the trail traces the path."""
    return [
        Move("swipe", [(0.2 * w, 0.7 * h), (0.8 * w, 0.4 * h)], pause_s, duration_ms=1100),
        Move("swipe", [(0.8 * w, 0.55 * h), (0.2 * w, 0.55 * h)], pause_s, duration_ms=1100),
    ]


def fan_moves(
    w: float, h: float, count: int = 8, radius: float = 0.30, pause_s: float = 0.9
) -> list[Move]:
    """Star/fan glides out to a ring and back to centre, exercising every heading."""
    cx, cy = w / 2.0, h / 2.0
    reach = radius * min(w, h)
    moves: list[Move] = []
    for i in range(count):
        angle = 2.0 * math.pi * i / count
        moves.append(
            Move("glide", [(cx + math.cos(angle) * reach, cy + math.sin(angle) * reach)], pause_s)
        )
        moves.append(Move("glide", [(cx, cy)], pause_s))
    return moves


def tap_settle_moves(w: float, h: float, pause_s: float = 2.2) -> list[Move]:
    """A far ``set_cursor`` immediately chased by a Tap at the same target: on
    video the ripple visibly waits for the cursor's arrival (arrival-gated
    feedback), instead of firing at dispatch."""
    fractions = [
        (0.80, 0.75),
        (0.20, 0.25),
        (0.75, 0.20),
        (0.25, 0.80),
    ]
    moves: list[Move] = []
    for fx, fy in fractions:
        point = (fx * w, fy * h)
        moves.append(Move("glide", [point], 0.0))
        moves.append(Move("tap", [point], pause_s, duration_ms=380))
    return moves


def fast_flick_moves(w: float, h: float, pause_s: float = 0.15) -> list[Move]:
    """Rapid far retarget pairs, pointer-telemetry style: judges how the glide
    reads when the target jumps as fast as real mouse deltas (perceived lag)."""
    fractions = [
        (0.15, 0.50),
        (0.85, 0.50),
        (0.20, 0.20),
        (0.80, 0.80),
        (0.80, 0.20),
        (0.20, 0.80),
        (0.50, 0.15),
        (0.50, 0.85),
    ]
    return [Move("glide", [(fx * w, fy * h)], pause_s) for fx, fy in fractions]


SCENARIOS: dict[str, Callable[[float, float], list[Move]]] = {
    "corners": corner_moves,
    "redirect": redirect_moves,
    "swipes": swipe_moves,
    "fan": fan_moves,
    "tap_settle": tap_settle_moves,
    "fast_flick": fast_flick_moves,
}


def moves_for(scenarios: list[str], w: float, h: float) -> list[Move]:
    """Concatenates the move lists for the requested scenarios."""
    moves: list[Move] = []
    for name in scenarios:
        moves.extend(SCENARIOS[name](w, h))
    return moves


def recording_seconds(
    moves: list[Move],
    lead_in: float = 1.0,
    tail: float = 1.6,
    per_move_overhead: float = 0.05,
) -> int:
    """Recording length that covers the lead-in, every move, and a settle tail.

    ``per_move_overhead`` budgets the per-move connect/dispatch latency on the
    local Unix socket on top of each move's ``pause_s`` so the recorder
    outlasts the drive loop and the final glide/settle is never truncated.
    """
    total = lead_in + tail + sum(move.pause_s for move in moves) + per_move_overhead * len(moves)
    return max(3, min(600, math.ceil(total)))


# ---------------------------------------------------------------------------
# Screen geometry (desktop-logical union of the enabled outputs)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ScreenGeometry:
    """Union bounding rect of the enabled outputs, in desktop-logical pixels.

    ``x``/``y`` can be negative (a monitor left of/above the primary); scenario
    builders stay pure in ``(width, height)`` and the dispatch loop translates
    their points by the origin.
    """

    x: float
    y: float
    width: float
    height: float


def union_logical_geometry(outputs: list[dict[str, Any]]) -> ScreenGeometry:
    """Union rect of the enabled outputs from ``kscreen-doctor -j``: ``pos`` is
    already logical; ``size`` is the current mode in device pixels, divided by
    the per-output ``scale``."""
    rects: list[tuple[float, float, float, float]] = []
    for output in outputs:
        if not output.get("enabled"):
            continue
        pos = output.get("pos") or {}
        size = output.get("size") or {}
        scale = float(output.get("scale") or 1.0) or 1.0
        width = float(size.get("width") or 0.0) / scale
        height = float(size.get("height") or 0.0) / scale
        if width <= 0.0 or height <= 0.0:
            continue
        x = float(pos.get("x") or 0.0)
        y = float(pos.get("y") or 0.0)
        rects.append((x, y, x + width, y + height))
    if not rects:
        raise SystemExit(
            "kscreen-doctor reported no enabled outputs; pass --width/--height explicitly"
        )
    min_x = min(rect[0] for rect in rects)
    min_y = min(rect[1] for rect in rects)
    max_x = max(rect[2] for rect in rects)
    max_y = max(rect[3] for rect in rects)
    return ScreenGeometry(min_x, min_y, max_x - min_x, max_y - min_y)


def detect_screen_geometry() -> ScreenGeometry:
    completed = subprocess.run(
        ["kscreen-doctor", "-j"], capture_output=True, text=True, check=False
    )
    if completed.returncode != 0 or not completed.stdout.strip():
        raise SystemExit(
            "kscreen-doctor -j failed (KDE-only geometry probe); "
            "pass --width/--height for other compositors"
        )
    data = json.loads(completed.stdout)
    outputs = data.get("outputs")
    return union_logical_geometry(outputs if isinstance(outputs, list) else [])


# ---------------------------------------------------------------------------
# Overlay host protocol (one JSON line per Unix-socket connection)
# ---------------------------------------------------------------------------


def send_host_message(
    payload: dict[str, Any], *, socket_path: Path = SOCKET_PATH, timeout: float = 5.0
) -> dict[str, Any]:
    """One request/one reply per connection, matching the host's transport."""
    return _overlay_host.call_host(
        socket_path, payload, timeout=timeout, context=f"kind={payload.get('kind')!r}"
    )


def cursor_state(point: tuple[float, float], sequence: int) -> dict[str, Any]:
    """A desktop-logical cursor state the host will actually render: both
    points logical, an opaque snapshot id, and a ~now ``updated_at_ms`` (a
    stale/zero timestamp reads as a decayed cursor and draws nothing)."""
    return _overlay_host.agent_cursor_state(point, sequence=sequence, snapshot_id=SNAPSHOT_ID)


class HostSession:
    """Owns the overlay host child on a private socket for the run's lifetime."""

    def __init__(self, host_bin: Path, socket_path: Path = SOCKET_PATH) -> None:
        self._host_bin = host_bin
        self._socket_path = socket_path
        self._child: subprocess.Popen[bytes] | None = None
        self._sequence = 0

    def start(self) -> None:
        # Clear only a host orphaned on THIS harness's socket — never an
        # operator's live service-owned host on a different socket.
        _overlay_host.terminate_leftover_hosts(self._socket_path)
        self._socket_path.unlink(missing_ok=True)
        env = dict(os.environ)
        env["SKY_CUA_OVERLAY_BACKEND"] = "wayland-layer-shell"
        self._child = subprocess.Popen(
            [str(self._host_bin), "serve", "--socket", str(self._socket_path)],
            cwd=str(REPO_ROOT),
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.time() + 10.0
        while time.time() < deadline and not self._socket_path.exists():
            if self._child.poll() is not None:
                raise SystemExit(f"overlay host exited at startup (rc={self._child.returncode})")
            time.sleep(0.1)
        if not self._socket_path.exists():
            raise SystemExit(f"overlay host socket never appeared: {self._socket_path}")
        hello = self.call("hello")
        capabilities: dict[str, Any] = hello.get("capabilities") or {}
        backend = capabilities.get("backend")
        if backend != "wayland_layer_shell":
            raise SystemExit(
                "overlay host did not select the wgpu layer-shell backend "
                f"(backend={backend!r}, reason={capabilities.get('reason')!r})"
            )
        print(f"[host] backend={backend} renderer={capabilities.get('renderer_backend')}")

    def call(self, kind: str, **fields: Any) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "version": OVERLAY_HOST_PROTOCOL_VERSION,
            "kind": kind,
            **fields,
        }
        return send_host_message(payload, socket_path=self._socket_path)

    def _next_sequence(self) -> int:
        self._sequence += 1
        return self._sequence

    def show(self, point: tuple[float, float]) -> dict[str, Any]:
        return self.call("show", state=cursor_state(point, self._next_sequence()))

    def set_cursor(self, point: tuple[float, float]) -> dict[str, Any]:
        return self.call("set_cursor", state=cursor_state(point, self._next_sequence()))

    def gesture(
        self, kind: str, points: list[tuple[float, float]], duration_ms: int
    ) -> dict[str, Any]:
        sequence = self._next_sequence()
        return self.call(
            "animate_gesture",
            gesture={
                "event_id": f"motion-anim-{sequence}-{secrets.token_hex(4)}",
                "sequence": sequence,
                "kind": kind,
                "coordinate_space": "desktop_logical",
                "points": [{"x": x, "y": y} for x, y in points],
                "duration_ms": duration_ms,
            },
        )

    def close(self) -> None:
        child = self._child
        self._child = None
        with contextlib.suppress(Exception):
            self.call("shutdown")
        if child is not None and child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                with contextlib.suppress(subprocess.TimeoutExpired):
                    child.wait(timeout=5)
        _overlay_host.terminate_leftover_hosts(self._socket_path)
        self._socket_path.unlink(missing_ok=True)


def dispatch_moves(session: HostSession, moves: list[Move], origin: tuple[float, float]) -> int:
    """Dispatches the moves (translated by the desktop-logical origin) and
    returns how many were rejected by the host."""
    rejected = 0
    for move in moves:
        points = [(origin[0] + x, origin[1] + y) for x, y in move.points]
        if move.kind == "swipe":
            reply = session.gesture("drag", points, move.duration_ms or 1100)
        elif move.kind == "tap":
            reply = session.gesture("tap", points[:1], move.duration_ms or 380)
        else:
            reply = session.set_cursor(points[0])
        if not reply.get("ok", False):
            rejected += 1
        time.sleep(move.pause_s)
    return rejected


# ---------------------------------------------------------------------------
# Recorders
# ---------------------------------------------------------------------------


def stills_frame_path(frames_dir: Path, index: int) -> Path:
    """``frame_0001.png``-style naming for the spectacle stills fallback."""
    return frames_dir / f"frame_{index:04d}.png"


class StillsRecorder:
    """~2 fps ``spectacle -b -n`` full-desktop stills on a background thread —
    the fallback recorder when the ScreenCast portal stack is unavailable."""

    def __init__(self, frames_dir: Path, interval_s: float = 0.5) -> None:
        self._frames_dir = frames_dir
        self._interval_s = interval_s
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def _loop(self) -> None:
        index = 0
        while not self._stop.is_set():
            frame = stills_frame_path(self._frames_dir, index)
            subprocess.run(
                ["spectacle", "-b", "-n", "-f", "-o", str(frame)],
                capture_output=True,
                check=False,
            )
            index += 1
            self._stop.wait(self._interval_s)

    def start(self) -> None:
        if shutil.which("spectacle") is None:
            raise SystemExit("stills recorder needs spectacle on PATH")
        if self._frames_dir.exists():
            shutil.rmtree(self._frames_dir)
        self._frames_dir.mkdir(parents=True, exist_ok=True)
        self._stop.clear()
        self._thread = threading.Thread(target=self._loop, name="stills-recorder", daemon=True)
        self._thread.start()

    def stop(self) -> list[Path]:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=15)
            self._thread = None
        return sorted(self._frames_dir.glob("frame_*.png"))


def select_recorder(requested: str) -> tuple[str, str | None]:
    """Resolves ``--recorder`` to a concrete recorder name, plus a fallback note."""
    if requested == "stills":
        return "stills", None
    reason = _kde_screencast.probe()
    if reason is None:
        return "portal", None
    if requested == "portal":
        raise SystemExit(f"portal recorder unavailable: {reason}")
    return "stills", f"portal recorder unavailable ({reason}); falling back to spectacle stills"


# ---------------------------------------------------------------------------
# Options / binary freshness
# ---------------------------------------------------------------------------


@dataclass
class Options:
    scenarios: list[str]
    recorder: str = "auto"
    width: float | None = None
    height: float | None = None
    fps: int = 10
    offline: bool = False
    build: bool = False
    allow_stale: bool = False
    unlock_screen: bool = True
    out_dir: Path = field(default_factory=lambda: ARTIFACTS_DIR)


def ensure_host_binary(*, build: bool, allow_stale: bool) -> Path:
    """Builds the release host when asked/missing, then gates on freshness.

    This harness spawns the host binary directly (not through
    ``_mcp_stdio.McpClient``), so the shared deploy-freshness choke point does
    not fire automatically — enforce it here per scripts/AGENTS.md policy.
    """
    if build or not HOST_BIN.exists():
        print("[build] cargo build --release -p sky-cua-overlay-host")
        code = subprocess.run(
            ["cargo", "build", "--release", "-p", "sky-cua-overlay-host"],
            cwd=REPO_ROOT,
            check=False,
        ).returncode
        if code != 0 or not HOST_BIN.exists():
            raise SystemExit(f"cargo build --release failed (exit {code}); host not at {HOST_BIN}")
    if not allow_stale and not deploy_freshness.allow_stale():
        freshness = deploy_freshness.check_client_freshness(
            HOST_BIN, deploy_command="cargo build --release -p sky-cua-overlay-host"
        )
        if not freshness.fresh:
            raise SystemExit(
                f"deploy-freshness: {freshness.summary} — {freshness.advice}; "
                "or pass --allow-stale / --build"
            )
    return HOST_BIN


# ---------------------------------------------------------------------------
# Live capture
# ---------------------------------------------------------------------------


def run_live(opts: Options) -> int:
    host_bin = ensure_host_binary(build=opts.build, allow_stale=opts.allow_stale)
    if opts.width is not None and opts.height is not None:
        geometry = ScreenGeometry(0.0, 0.0, opts.width, opts.height)
    else:
        geometry = detect_screen_geometry()
    print(
        f"[screen] {geometry.width:.0f}x{geometry.height:.0f} logical "
        f"at ({geometry.x:.0f},{geometry.y:.0f})"
    )
    moves = moves_for(opts.scenarios, geometry.width, geometry.height)
    seconds = recording_seconds(moves)
    recorder_name, note = select_recorder(opts.recorder)
    if note:
        print(f"[note] {note}")
    print(
        f"[record] recorder={recorder_name} {seconds}s, {len(moves)} moves: "
        f"{', '.join(opts.scenarios)}"
    )
    opts.out_dir.mkdir(parents=True, exist_ok=True)

    origin = (geometry.x, geometry.y)
    park = (geometry.x + geometry.width / 2.0, geometry.y + geometry.height / 2.0)
    sheet = opts.out_dir / f"contact-{'-'.join(opts.scenarios)}.png"
    session = HostSession(host_bin)
    rejected = 0
    sheets: list[Path] = []
    # Unlock the KDE session for the drive+record window (and relock afterwards
    # if we unlocked): a locked screen makes the recorder film the lock-screen
    # greeter instead of the overlay.
    with _session_lock.screen_unlocked(enabled=opts.unlock_screen):
        try:
            session.start()
            show = session.show(park)  # park at centre; the first placement snaps
            if not show.get("ok", False):
                raise SystemExit(
                    f"overlay host rejected the initial show: {show.get('diagnostics')}"
                )

            if recorder_name == "portal":
                mp4 = opts.out_dir / "overlay-motion-animations.mp4"
                recorder = _kde_screencast.ScreenCastRecorder(RESTORE_TOKEN_PATH)
                try:
                    recorder.start(mp4)
                except _kde_screencast.PortalScreenCastError as error:
                    # start() already released its portal stream fd; stop() is a
                    # no-op belt-and-braces here.
                    recorder.stop()
                    if opts.recorder == "portal":
                        raise SystemExit(f"portal recorder failed to start: {error}") from error
                    print(
                        f"[note] portal recorder failed to start ({error}); "
                        "falling back to spectacle stills"
                    )
                    recorder_name = "stills"
                else:
                    try:
                        time.sleep(1.0)  # lead-in: the parked cursor + cloud bloom
                        rejected = dispatch_moves(session, moves, origin)
                        time.sleep(1.6)  # tail: let the final glide settle on film
                    finally:
                        recorder.stop()
                    print(f"[artifact] {mp4}")
                    sheets = make_contact_sheet(mp4, sheet, fps=opts.fps)
            if recorder_name == "stills":
                frames_dir = opts.out_dir / "stills_frames"
                stills = StillsRecorder(frames_dir)
                stills.start()
                try:
                    time.sleep(1.0)
                    rejected = dispatch_moves(session, moves, origin)
                    time.sleep(1.6)
                finally:
                    frames = stills.stop()
                print(f"[record] {len(frames)} spectacle stills")
                sheets = montage_frames(frames, sheet)
                shutil.rmtree(frames_dir, ignore_errors=True)
        finally:
            session.close()

    if rejected:
        print(f"[note] {rejected} of {len(moves)} dispatches were rejected")
    if sheets:
        for produced in sheets:
            print(f"[artifact] {produced}")
    else:
        print("[note] ffmpeg/montage unavailable or extraction failed; kept the recording only")
    print(f"[done] artifacts in {opts.out_dir} (operator-desktop recording: never commit)")
    return 0


# ---------------------------------------------------------------------------
# Offline deterministic evidence (no desktop, no portal)
# ---------------------------------------------------------------------------


def scenario_group(stem: str) -> str:
    """``corner_glide-f07`` -> ``corner_glide`` (strips the trailing frame tag).

    Contract with ``capture_motion_frames_when_requested``
    (``renderer/motion_capture.rs``): frames are named
    ``<scenario>-f<NN>.rgba``. A ``<scenario>_<index>`` form is grouped too, so
    the gesture dump's frames montage as well if pointed here.
    """
    base, sep, tail = stem.rpartition("-")
    if sep and base and tail.startswith("f") and tail[1:].isdigit():
        return base
    base, sep, tail = stem.rpartition("_")
    if sep and base and tail.isdigit():
        return base
    return stem


def rgba_frames_to_pngs(capture_dir: Path, out_dir: Path) -> dict[str, list[Path]]:
    """Converts the motion dump's raw frames to PNGs grouped by scenario.

    Contract with ``capture_motion_frames_when_requested``: the capture dir
    holds ``<scenario>_<frame>.rgba`` raw frames, a ``dims.txt`` with
    ``<width> <height>``, and a ``manifest.txt``. Frames whose byte length does
    not match the advertised dims (e.g. auxiliary texture dumps) are skipped.
    """
    from PIL import Image

    dims = (capture_dir / "dims.txt").read_text(encoding="utf-8").split()
    width, height = int(dims[0]), int(dims[1])
    expected_len = width * height * 4
    out_dir.mkdir(parents=True, exist_ok=True)
    groups: dict[str, list[Path]] = {}
    for rgba in sorted(capture_dir.glob("*.rgba")):
        raw = rgba.read_bytes()
        if len(raw) != expected_len:
            continue
        png = out_dir / f"{rgba.stem}.png"
        Image.frombytes("RGBA", (width, height), raw).save(png)
        groups.setdefault(scenario_group(rgba.stem), []).append(png)
    return groups


def run_offline(opts: Options) -> int:
    capture_dir = Path(tempfile.mkdtemp(prefix="overlay-motion-frames-"))
    env = dict(os.environ)
    env["SKY_CUA_CAPTURE_MOTION"] = "1"
    env["SKY_CUA_CAPTURE_DIR"] = str(capture_dir)
    command = [
        "cargo",
        "nextest",
        "run",
        "--release",
        "-p",
        "sky-cua-overlay-host",
        "-E",
        "test(capture_motion_frames_when_requested)",
    ]
    print(f"[offline] {' '.join(command)}")
    completed = subprocess.run(command, cwd=REPO_ROOT, env=env, check=False)
    if completed.returncode != 0:
        raise SystemExit(f"motion capture test failed (exit {completed.returncode})")

    out_dir = opts.out_dir / "offline"
    out_dir.mkdir(parents=True, exist_ok=True)
    manifest = capture_dir / "manifest.txt"
    if manifest.exists():
        shutil.copy2(manifest, out_dir / "manifest.txt")
        print(f"[artifact] {out_dir / 'manifest.txt'}")
    try:
        if not (capture_dir / "dims.txt").exists():
            # The gated test passed without writing anything: it self-skips
            # when no usable GPU adapter exists (e.g. a headless box).
            raise SystemExit(
                f"no motion frames were captured under {capture_dir} "
                "(the capture test likely self-skipped: no usable GPU adapter)"
            )
        groups = rgba_frames_to_pngs(capture_dir, out_dir / "frames")
        if not groups:
            raise SystemExit(f"no motion frames were captured under {capture_dir}")
        for scenario, frames in sorted(groups.items()):
            sheets = montage_frames(frames, out_dir / f"contact-{scenario}.png", tile="6x5")
            if sheets:
                for produced in sheets:
                    print(f"[artifact] {produced}")
            else:
                print(f"[note] montage unavailable; kept raw PNG frames for {scenario}")
        print(f"[done] offline artifacts in {out_dir}")
    finally:
        shutil.rmtree(capture_dir, ignore_errors=True)
    return 0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Overlay motion-animation visual test harness (desktop layer-shell overlay)."
    )
    parser.add_argument(
        "--scenario",
        action="append",
        choices=sorted(SCENARIOS.keys()),
        help="scenario(s) to run; repeatable. Default: corners + redirect + swipes + tap_settle.",
    )
    parser.add_argument(
        "--recorder",
        choices=["auto", "portal", "stills"],
        default="auto",
        help="auto probes the ScreenCast portal and falls back to spectacle stills",
    )
    parser.add_argument("--width", type=float, help="logical width override (skips kscreen-doctor)")
    parser.add_argument("--height", type=float, help="logical height override")
    parser.add_argument("--fps", type=int, default=10, help="contact-sheet frame rate (default 10)")
    parser.add_argument(
        "--offline",
        action="store_true",
        help="montage the deterministic offline motion dump instead of recording the desktop",
    )
    parser.add_argument(
        "--build", action="store_true", help="cargo build --release the overlay host first"
    )
    parser.add_argument(
        "--allow-stale",
        action="store_true",
        help="skip the deploy-freshness gate on the host binary",
    )
    parser.add_argument(
        "--no-unlock",
        action="store_true",
        help="do not unlock the KDE session before recording (leaves lock state untouched)",
    )
    return parser


def options_from_args(args: argparse.Namespace) -> Options:
    if (args.width is None) != (args.height is None):
        raise SystemExit("--width and --height must be passed together")
    return Options(
        scenarios=args.scenario or list(DEFAULT_SCENARIOS),
        recorder=args.recorder,
        width=args.width,
        height=args.height,
        fps=args.fps,
        offline=args.offline,
        build=args.build,
        allow_stale=args.allow_stale,
        unlock_screen=not args.no_unlock,
    )


def main(argv: list[str] | None = None) -> int:
    opts = options_from_args(build_parser().parse_args(argv))
    if opts.offline:
        return run_offline(opts)
    return run_live(opts)


if __name__ == "__main__":
    raise SystemExit(main())
