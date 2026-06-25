#!/usr/bin/env python3
"""X11 unsupported-contract smoke for the agent cursor overlay host.

Artifacts: artifacts/codex-e2e/agent-cursor-x11-overlay/<timestamp>/
Proof: in a real X11 session, explicit SKY_CUA_OVERLAY_BACKEND=x11 returns
honest Noop capabilities instead of the retired shaped-window renderer.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from collections.abc import Mapping
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from live_agent_cursor_kde_smoke import write_summary  # type: ignore[import-not-found]

REPO_ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = REPO_ROOT / "artifacts" / "codex-e2e" / "agent-cursor-x11-overlay"
OVERLAY_HOST_BIN = REPO_ROOT / "target" / "debug" / "sky-cua-overlay-host"
UNSUPPORTED_REASON = "X11 visible overlay requires a WGPU X11 host"


def main() -> int:
    parser = argparse.ArgumentParser(description="Smoke the retired X11 visible-overlay contract.")
    parser.add_argument(
        "--current-display",
        action="store_true",
        help="Require the current DISPLAY to be a real X11 session before probing.",
    )
    args = parser.parse_args()

    try:
        current_display = require_current_x11_display() if args.current_display else None
    except RuntimeError as exc:
        raise SystemExit(str(exc)) from None

    require_installed("xdpyinfo")
    build_overlay_host()

    artifact_dir = ARTIFACT_ROOT / datetime.now(UTC).strftime("%Y%m%dT%H%M%S%fZ")
    artifact_dir.mkdir(parents=True, exist_ok=True)
    env = x11_overlay_env(current_display or os.environ.get("DISPLAY", ":0"), artifact_dir)
    reply = probe_overlay_host(env)
    require_x11_unsupported_reply(reply)

    summary = {
        "ok": True,
        "mode": "x11-unsupported-contract",
        "artifact_dir": str(artifact_dir),
        "display": env.get("DISPLAY"),
        "backend": backend_from_reply(reply),
        "visible_overlay": capability_bool(reply, "visible_overlay"),
        "renderer_backend": renderer_from_reply(reply),
        "probe": reply,
    }
    write_summary(artifact_dir, summary)
    return 0


def require_installed(binary: str) -> None:
    if shutil.which(binary) is None:
        raise RuntimeError(f"required binary is not installed: {binary}")


def require_current_x11_display() -> str:
    display = os.environ.get("DISPLAY", "").strip()
    if not display:
        raise RuntimeError("--current-display requires DISPLAY to be set")
    session_type = os.environ.get("XDG_SESSION_TYPE", "").strip().lower()
    if session_type != "x11":
        raise RuntimeError(
            "--current-display is only accepted in a real X11 session; "
            f"XDG_SESSION_TYPE={session_type or '<unset>'}"
        )
    if os.environ.get("WAYLAND_DISPLAY"):
        raise RuntimeError(
            "--current-display refused a Wayland/XWayland session because "
            f"WAYLAND_DISPLAY={os.environ['WAYLAND_DISPLAY']!r}"
        )
    return display


def build_overlay_host() -> None:
    prebuilt = overlay_host_path_from_env()
    if os.environ.get("SKY_CUA_USE_PREBUILT_RUNTIMES") == "1" and prebuilt:
        if not prebuilt.exists():
            raise RuntimeError(f"prebuilt overlay host binary does not exist: {prebuilt}")
        return
    subprocess.run(
        ["cargo", "build", "--package", "sky-cua-overlay-host"],
        cwd=REPO_ROOT,
        check=True,
    )


def overlay_host_path_from_env() -> Path | None:
    value = os.environ.get("SKY_CUA_OVERLAY_HOST_PATH") or os.environ.get(
        "SKY_CUA_DEBUG_OVERLAY_HOST_PATH"
    )
    return Path(value) if value else None


def overlay_host_binary() -> Path:
    return overlay_host_path_from_env() or OVERLAY_HOST_BIN


def x11_overlay_env(display: str, runtime_dir: Path) -> dict[str, str]:
    env = dict(os.environ)
    env["DISPLAY"] = display
    env["XDG_SESSION_TYPE"] = "x11"
    env["XDG_RUNTIME_DIR"] = str(runtime_dir)
    env["SKY_CUA_OVERLAY_BACKEND"] = "x11"
    env["GDK_BACKEND"] = "x11"
    env["QT_QPA_PLATFORM"] = "xcb"
    env.pop("WAYLAND_DISPLAY", None)
    return env


def probe_overlay_host(env: Mapping[str, str]) -> dict[str, Any]:
    result = subprocess.run(
        [str(overlay_host_binary()), "probe"],
        cwd=REPO_ROOT,
        env=dict(env),
        text=True,
        capture_output=True,
        check=True,
    )
    reply = json.loads(result.stdout)
    if not isinstance(reply, dict):
        raise RuntimeError(f"overlay host probe was not an object: {reply!r}")
    return reply


def cursor_message(point: tuple[float, float], *, sequence: int) -> dict[str, Any]:
    return {
        "version": 2,
        "kind": "set_cursor",
        "state": {
            "visible": True,
            "sequence": sequence,
            "model_point": {
                "x": point[0],
                "y": point[1],
                "coordinate_space": "stream_pixels",
            },
            "source_action": "click",
            "updated_at_ms": 0,
        },
    }


def require_x11_unsupported_reply(reply: Mapping[str, Any]) -> None:
    capabilities = reply.get("capabilities")
    if not reply.get("ok") or not isinstance(capabilities, dict):
        raise RuntimeError(
            "overlay host did not return usable capabilities: "
            + json.dumps(reply, indent=2, sort_keys=True)
        )
    expected = {
        "backend": "none",
        "renderer_backend": "none",
        "visible_overlay": False,
        "click_through": False,
        "system_cursor_hide_supported": False,
        "system_cursor_hidden": False,
    }
    for key, value in expected.items():
        if capabilities.get(key) != value:
            raise RuntimeError(
                f"overlay host capability {key!r} was {capabilities.get(key)!r}, "
                f"expected {value!r}.\nreply={json.dumps(reply, indent=2, sort_keys=True)}"
            )
    reason = capabilities.get("reason")
    if not isinstance(reason, str) or UNSUPPORTED_REASON not in reason:
        raise RuntimeError(
            "overlay host did not report the retired X11 WGPU-host reason: "
            + json.dumps(reply, indent=2, sort_keys=True)
        )


def backend_from_reply(reply: Mapping[str, Any]) -> str | None:
    capabilities = reply.get("capabilities")
    if isinstance(capabilities, dict):
        backend = capabilities.get("backend")
        if isinstance(backend, str):
            return backend
    return None


def renderer_from_reply(reply: Mapping[str, Any]) -> str | None:
    capabilities = reply.get("capabilities")
    if isinstance(capabilities, dict):
        renderer = capabilities.get("renderer_backend")
        if isinstance(renderer, str):
            return renderer
    return None


def capability_bool(reply: Mapping[str, Any], key: str) -> bool | None:
    capabilities = reply.get("capabilities")
    if isinstance(capabilities, Mapping):
        value = capabilities.get(key)
        if isinstance(value, bool):
            return value
    return None


if __name__ == "__main__":
    raise SystemExit(main())
