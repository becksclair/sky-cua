#!/usr/bin/env python3
"""Service-backed Wayland layer-shell live smoke for the agent cursor overlay.

This entrypoint keeps the historical profile name, but proof now goes through
sky-cua's own screenshot service request rather than compositor-specific tools.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Live smoke for the Wayland layer-shell agent cursor overlay."
    )
    parser.add_argument(
        "--wayland-display",
        default=os.environ.get("WAYLAND_DISPLAY", ""),
        help="Wayland socket name. Defaults to WAYLAND_DISPLAY.",
    )
    parser.add_argument(
        "--overlay-backend",
        default=os.environ.get("SKY_CUA_OVERLAY_BACKEND", "wayland-layer-shell"),
        help="Overlay backend to request. Defaults to wayland-layer-shell.",
    )
    parser.add_argument(
        "--request-timeout",
        type=float,
        default=120.0,
        help="Seconds to wait for each service IPC request.",
    )
    parser.add_argument("--capture-command", help=argparse.SUPPRESS)
    parser.add_argument("--capture-output", help=argparse.SUPPRESS)
    parser.add_argument("--point", help=argparse.SUPPRESS)
    parser.add_argument("--allow-no-visible-overlay", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()

    env = dict(os.environ)
    env["XDG_SESSION_TYPE"] = "wayland"
    env["SKY_CUA_OVERLAY_BACKEND"] = args.overlay_backend
    if args.wayland_display:
        env["WAYLAND_DISPLAY"] = args.wayland_display

    command = [
        sys.executable,
        str(REPO_ROOT / "scripts" / "live_agent_cursor_kde_smoke.py"),
        "--mode",
        "layer-shell-debug-visible",
        "--allow-non-kde",
        "--request-timeout",
        str(args.request_timeout),
    ]
    return subprocess.run(command, cwd=REPO_ROOT, env=env, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
