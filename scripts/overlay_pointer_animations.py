#!/usr/bin/env python3
"""Overlay pointer-animation visual test harness for sky-cua.

Drives the phone-side agent overlay on a neutral grid surface and records the
result so the glide / rotation / curve, the screen-edge glow and inward wave,
and the tap ripple / swipe trail can be reviewed by eye. This is a *visual*
harness: it produces a recording and frame contact-sheets rather than a
pass/fail line, because animation quality is judged by a human, not asserted.

- Target app: the companion ``GridTestActivity`` (a white screen with a light
  grey grid) so the overlay is exercised on a clean canvas, never the operator's
  real apps.
- Daemon: the isolated release daemon on a private socket
  (``/tmp/sky-cua-overlay-anim.sock``), so the operator's installed daemon is
  never touched.
- Artifacts: ``artifacts/overlay-pointer-animations/`` — the raw MP4 plus a
  contact sheet per scenario (when ``ffmpeg`` / ImageMagick ``montage`` are
  available; otherwise the MP4 is kept and a note is printed).

Usage:
  uv run python scripts/overlay_pointer_animations.py --serial <serial>
  uv run python scripts/overlay_pointer_animations.py --serial <serial> --scenario redirect
  uv run python scripts/overlay_pointer_animations.py --serial <serial> --skip-build
  uv run python scripts/overlay_pointer_animations.py --serial <serial> --build-daemon

By default the companion APK is rebuilt and reinstalled (the overlay lives in the
APK) and the existing release daemon is reused. Pass ``--skip-build`` to use the
already-installed APK, or ``--build-daemon`` to also rebuild the release daemon.
"""

from __future__ import annotations

import argparse
import contextlib
import math
import os
import subprocess
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from _contact_sheets import make_contact_sheet
from _mcp_stdio import McpClient
from live_phone_use_smoke import (
    PhoneSmoke,
    PhoneSmokeOptions,
    connect_session,
    first_diagnostic_message,
    resolve_client_path,
    result_is_error,
    structured,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
COMPANION_DIR = REPO_ROOT / "android" / "phone-companion"
COMPANION_APK = COMPANION_DIR / "app" / "build" / "outputs" / "apk" / "debug" / "app-debug.apk"
RELEASE_DAEMON = REPO_ROOT / "target" / "release" / "sky-cua-service"
ARTIFACTS_DIR = REPO_ROOT / "artifacts" / "overlay-pointer-animations"
GRID_ACTIVITY = "com.skycua.phonecompanion/.ui.GridTestActivity"
ISOLATED_SOCKET = "/tmp/sky-cua-overlay-anim.sock"
DEFAULT_JAVA_HOME = Path("/usr/lib/jvm/java-21-openjdk")


# ---------------------------------------------------------------------------
# Move scenarios (pure: no device, unit-testable)
# ---------------------------------------------------------------------------


@dataclass
class Move:
    """One scripted overlay action in device pixels.

    ``kind`` is ``"tap"`` (one point) or ``"swipe"`` (start, end). ``pause_s`` is
    the delay held after dispatching so the animation can play (or, for the
    redirect scenario, a short delay so the next move redirects the cursor
    mid-flight and the path curves hard).
    """

    kind: str
    points: list[tuple[float, float]]
    pause_s: float


def corner_moves(w: int, h: int, margin: float = 0.18, pause_s: float = 1.1) -> list[Move]:
    """Big corner-to-corner taps: long glides that curve and rotate, then settle."""
    fractions = [
        (margin, margin),
        (1.0 - margin, 1.0 - margin),
        (1.0 - margin, margin),
        (margin, 1.0 - margin),
        (0.5, 0.5),
    ]
    return [Move("tap", [(fx * w, fy * h)], pause_s) for fx, fy in fractions]


def fan_moves(
    w: int, h: int, count: int = 8, radius: float = 0.30, pause_s: float = 0.9
) -> list[Move]:
    """Star/fan taps out to a ring and back to centre, exercising every heading."""
    cx, cy = w / 2.0, h / 2.0
    reach = radius * float(min(w, h))
    moves: list[Move] = []
    for i in range(count):
        angle = 2.0 * math.pi * i / count
        moves.append(
            Move("tap", [(cx + math.cos(angle) * reach, cy + math.sin(angle) * reach)], pause_s)
        )
        moves.append(Move("tap", [(cx, cy)], pause_s))
    return moves


def swipe_moves(w: int, h: int, pause_s: float = 2.8) -> list[Move]:
    """Diagonal and horizontal swipes: the cursor sails the path, trail follows."""
    return [
        Move("swipe", [(0.2 * w, 0.7 * h), (0.8 * w, 0.4 * h)], pause_s),
        Move("swipe", [(0.8 * w, 0.55 * h), (0.2 * w, 0.55 * h)], pause_s),
    ]


def redirect_moves(w: int, h: int, pause_s: float = 0.32) -> list[Move]:
    """Rapid taps that fire faster than a glide settles, so the cursor is
    redirected mid-flight and the momentum bows the path hard."""
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
    return [Move("tap", [(fx * w, fy * h)], pause_s) for fx, fy in fractions]


SCENARIOS: dict[str, Callable[[int, int], list[Move]]] = {
    "corners": corner_moves,
    "fan": fan_moves,
    "swipes": swipe_moves,
    "redirect": redirect_moves,
}


def moves_for(scenarios: list[str], w: int, h: int) -> list[Move]:
    """Concatenates the move lists for the requested scenarios."""
    moves: list[Move] = []
    for name in scenarios:
        moves.extend(SCENARIOS[name](w, h))
    return moves


def recording_seconds(
    moves: list[Move],
    lead_in: float = 1.0,
    tail: float = 1.6,
    per_move_overhead: float = 0.35,
) -> int:
    """Recording length that covers the lead-in glow, every move, and a tail.

    ``per_move_overhead`` budgets the per-move MCP -> service -> adb dispatch
    latency on top of each move's ``pause_s`` so the device-side recorder outlasts
    the host loop and the final glide/settle is never truncated. ``screenrecord``
    caps at 180s, so the result is clamped there.
    """
    total = lead_in + tail + sum(move.pause_s for move in moves) + per_move_overhead * len(moves)
    return max(3, min(180, math.ceil(total)))


# ---------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------


@dataclass
class Options:
    serial: str
    scenarios: list[str]
    skip_build: bool = False
    build_daemon: bool = False
    keep_overlay: bool = False
    fps: int = 10
    out_dir: Path = field(default_factory=lambda: ARTIFACTS_DIR)


# ---------------------------------------------------------------------------
# Subprocess helpers
# ---------------------------------------------------------------------------


def _run(cmd: list[str], *, env: dict[str, str] | None = None, check: bool = False) -> int:
    """Runs a command quietly, returning its exit code (raising if ``check``)."""
    result = subprocess.run(cmd, env=env, capture_output=True, check=check)
    return result.returncode


def _adb(serial: str, args: list[str], *, check: bool = False) -> int:
    return _run(["adb", "-s", serial, *args], check=check)


def _gradle_env() -> dict[str, str]:
    env = dict(os.environ)
    if "JAVA_HOME" not in env and DEFAULT_JAVA_HOME.exists():
        env["JAVA_HOME"] = str(DEFAULT_JAVA_HOME)
    return env


def build_and_install_apk(serial: str) -> None:
    print("[build] companion APK (assembleDebug)")
    code = subprocess.run(
        ["./gradlew", "assembleDebug", "--offline", "-q"],
        cwd=COMPANION_DIR,
        env=_gradle_env(),
        check=False,
    ).returncode
    if code != 0 or not COMPANION_APK.exists():
        raise SystemExit(f"gradle assembleDebug failed (exit {code}); APK not at {COMPANION_APK}")
    print("[install] adb install -r app-debug.apk")
    if _adb(serial, ["install", "-r", str(COMPANION_APK)]) != 0:
        raise SystemExit("adb install -r failed")


def build_daemon() -> None:
    print("[build] release daemon + client (cargo build --release)")
    code = _run(
        ["cargo", "build", "--release", "-p", "sky-cua-service", "-p", "sky-cua-client"],
    )
    if code != 0 or not RELEASE_DAEMON.exists():
        raise SystemExit(
            f"cargo build --release failed (exit {code}); daemon not at {RELEASE_DAEMON}"
        )


# ---------------------------------------------------------------------------
# Capture
# ---------------------------------------------------------------------------


def _client_env() -> dict[str, str]:
    env = dict(os.environ)
    env["SKY_CUA_SERVICE_PATH"] = str(RELEASE_DAEMON)
    env["SKY_CUA_SERVICE_SOCKET_PATH"] = ISOLATED_SOCKET
    env["SKY_CUA_PHONE"] = "1"
    # Keep base64 screenshots off the structured channel.
    env["SKY_CUA_MODEL_SUPPORTS_IMAGES"] = "false"
    # The capture loop reuses one device snapshot for the whole recording (the
    # device is stationary throughout), and screenrecord runs up to 180s. Raise
    # this isolated daemon's snapshot TTL well past the 30s default so dispatches
    # in the tail of a long run still resolve the snapshot instead of being
    # rejected as stale, which would record no motion and leave the animation
    # silently static.
    env["SKY_CUA_PHONE_CAPABILITY_CACHE_TTL_MS"] = "600000"
    return env


def _device_size(smoke: PhoneSmoke, session_id: str) -> tuple[int, int, str]:
    """Returns the device (width, height) and a fresh snapshot id, via a screenshot."""
    shot = smoke.screenshot(session_id)
    if result_is_error(shot):
        raise SystemExit(f"phone_screenshot failed: {first_diagnostic_message(shot)}")
    payload: dict[str, Any] = structured(shot)
    size: dict[str, Any] = payload.get("device_size") or {}
    width = int(size.get("width") or 1440)
    height = int(size.get("height") or 3120)
    snapshot_id = payload.get("phone_snapshot_id")
    if not isinstance(snapshot_id, str) or not snapshot_id:
        raise SystemExit("phone_screenshot returned no snapshot id")
    return width, height, snapshot_id


def _stop_recorder(recorder: subprocess.Popen[bytes]) -> None:
    """Ensures the screenrecord child is gone. It self-stops at ``--time-limit``,
    but an early error or an overrun must not leave it recording on the device."""
    if recorder.poll() is not None:
        return
    recorder.terminate()
    try:
        recorder.wait(timeout=5)
    except subprocess.TimeoutExpired:
        recorder.kill()
        with contextlib.suppress(subprocess.TimeoutExpired):
            recorder.wait(timeout=5)


def capture(opts: Options) -> int:
    if not RELEASE_DAEMON.exists():
        raise SystemExit(
            f"release daemon not found at {RELEASE_DAEMON}; build it first "
            "(cargo build --release -p sky-cua-service -p sky-cua-client) or pass --build-daemon"
        )
    opts.out_dir.mkdir(parents=True, exist_ok=True)
    _run(["adb", "connect", opts.serial])
    # Bring up the neutral grid canvas so the overlay renders on it.
    _adb(opts.serial, ["shell", "am", "start", "-n", GRID_ACTIVITY])
    time.sleep(1.0)

    with contextlib.suppress(FileNotFoundError):
        os.remove(ISOLATED_SOCKET)

    client = McpClient([str(resolve_client_path(installed=False)), "mcp"], base_env=_client_env())
    smoke = PhoneSmoke(client, PhoneSmokeOptions(profile="companion", serial=opts.serial))
    session_id = ""
    try:
        # Inside the try so a handshake failure is reaped by `client.close()`
        # below, matching `live_phone_use_smoke.run_smoke`; otherwise the MCP
        # subprocess spawned in `McpClient.__init__` would leak.
        client.initialize()
        session_id, _ = connect_session(smoke, opts.serial)
        print(f"[connect] session={session_id}")
        width, height, snapshot_id = _device_size(smoke, session_id)
        print(f"[device] {width}x{height}")

        moves = moves_for(opts.scenarios, width, height)
        seconds = recording_seconds(moves)
        mp4_device = "/sdcard/overlay-pointer-animations.mp4"
        print(f"[record] {seconds}s, {len(moves)} moves: {', '.join(opts.scenarios)}")
        recorder = subprocess.Popen(
            [
                "adb",
                "-s",
                opts.serial,
                "shell",
                "screenrecord",
                "--time-limit",
                str(seconds),
                "--bit-rate",
                "16000000",
                mp4_device,
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            time.sleep(1.0)  # lead-in so the glow + parked cursor are captured first
            rejected = 0
            for move in moves:
                if move.kind == "swipe" and len(move.points) >= 2:
                    (sx, sy), (ex, ey) = move.points[0], move.points[1]
                    result = smoke.swipe(session_id, snapshot_id, sx, sy, ex, ey)
                else:
                    px, py = move.points[0]
                    result = smoke.tap(session_id, snapshot_id, px, py)
                # A rejected dispatch (e.g. a stale snapshot) records nothing, so
                # an all-rejected run would otherwise yield a silently static MP4
                # indistinguishable from a clean one. Surface it without failing
                # the human-reviewed visual harness.
                if result_is_error(result):
                    rejected += 1
                time.sleep(move.pause_s)
            if rejected:
                print(f"[note] {rejected} of {len(moves)} dispatches were rejected")
            with contextlib.suppress(subprocess.TimeoutExpired):
                recorder.wait(timeout=seconds + 15)
        finally:
            _stop_recorder(recorder)

        mp4_local = opts.out_dir / "overlay-pointer-animations.mp4"
        if _adb(opts.serial, ["pull", mp4_device, str(mp4_local)]) != 0:
            raise SystemExit("adb pull of the recording failed")
        _adb(opts.serial, ["shell", "rm", "-f", mp4_device])
        print(f"[artifact] {mp4_local}")

        sheet = opts.out_dir / f"contact-{'-'.join(opts.scenarios)}.png"
        sheets = make_contact_sheet(mp4_local, sheet, fps=opts.fps)
        if sheets:
            for produced in sheets:
                print(f"[artifact] {produced}")
        else:
            print("[note] ffmpeg/montage unavailable or extraction failed; kept the MP4 only")
    finally:
        if session_id and not opts.keep_overlay:
            smoke.disconnect(session_id)
        client.close()
    print(f"[done] artifacts in {opts.out_dir}")
    return 0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Overlay pointer-animation visual test harness.")
    parser.add_argument("--serial", required=True, help="adb serial of the target device")
    parser.add_argument(
        "--scenario",
        action="append",
        choices=sorted(SCENARIOS.keys()),
        help="scenario(s) to run; repeatable. Default: corners + redirect + swipes.",
    )
    parser.add_argument(
        "--skip-build", action="store_true", help="use the installed APK; skip the rebuild"
    )
    parser.add_argument(
        "--build-daemon", action="store_true", help="also rebuild the release daemon + client"
    )
    parser.add_argument(
        "--keep-overlay", action="store_true", help="leave the overlay up (do not disconnect)"
    )
    parser.add_argument("--fps", type=int, default=10, help="contact-sheet frame rate (default 10)")
    return parser


def options_from_args(args: argparse.Namespace) -> Options:
    scenarios = args.scenario or ["corners", "redirect", "swipes"]
    return Options(
        serial=args.serial,
        scenarios=scenarios,
        skip_build=args.skip_build,
        build_daemon=args.build_daemon,
        keep_overlay=args.keep_overlay,
        fps=args.fps,
    )


def main(argv: list[str] | None = None) -> int:
    opts = options_from_args(build_parser().parse_args(argv))
    if not opts.skip_build:
        build_and_install_apk(opts.serial)
    if opts.build_daemon:
        build_daemon()
    return capture(opts)


if __name__ == "__main__":
    raise SystemExit(main())
