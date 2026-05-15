#!/usr/bin/env python3
"""Wayland layer-shell live smoke for the agent cursor overlay host.

Target app: the current real Wayland desktop session.
Artifacts: artifacts/codex-e2e/agent-cursor-wayland-layer-shell/<timestamp>/
Proof: the overlay host connects through the layer-shell backend, draws the
Chrome cursor asset into compositor screenshots, and hides it again on request.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from live_agent_cursor_kde_smoke import (  # type: ignore[import-not-found]
    MarkerProbe,
    copy_artifact,
    probe_marker,
    write_summary,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = REPO_ROOT / "artifacts" / "codex-e2e" / "agent-cursor-wayland-layer-shell"
OVERLAY_HOST_BIN = REPO_ROOT / "target" / "debug" / "sky-cua-overlay-host"
DEFAULT_POINT = (360.0, 260.0)


@dataclass(frozen=True)
class OverlayHostProcess:
    process: subprocess.Popen[str]

    def send(self, message: Mapping[str, Any]) -> dict[str, Any]:
        if self.process.stdin is None or self.process.stdout is None:
            raise RuntimeError("overlay host stdio pipes are unavailable")
        self.process.stdin.write(json.dumps(message, sort_keys=True) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            stderr = self.process.stderr.read() if self.process.stderr is not None else ""
            raise RuntimeError(f"overlay host exited without a reply.\nstderr={stderr}")
        reply = json.loads(line)
        if not isinstance(reply, dict):
            raise RuntimeError(f"overlay host reply was not an object: {reply!r}")
        return reply

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                self.send({"version": 1, "kind": "shutdown"})
                self.process.wait(timeout=3)
            except Exception:
                self.process.terminate()
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=3)
        stderr = self.process.stderr.read() if self.process.stderr is not None else ""
        if stderr.strip():
            print(f"overlay host stderr: {stderr.strip()}", file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Live smoke for the Wayland layer-shell agent cursor overlay."
    )
    parser.add_argument(
        "--wayland-display",
        default=os.environ.get("WAYLAND_DISPLAY", ""),
        help="Wayland socket name. Defaults to WAYLAND_DISPLAY or auto-detects a session socket.",
    )
    parser.add_argument(
        "--capture-command",
        default=os.environ.get("SKY_CUA_WAYLAND_CAPTURE_COMMAND", "grim"),
        help="Screenshot command for the current compositor. Defaults to grim.",
    )
    parser.add_argument(
        "--capture-output",
        default=os.environ.get("SKY_CUA_WAYLAND_CAPTURE_OUTPUT", ""),
        help="Optional compositor output name for screenshot tools such as grim.",
    )
    parser.add_argument(
        "--point",
        default=f"{DEFAULT_POINT[0]},{DEFAULT_POINT[1]}",
        help="Desktop logical point to draw the cursor at, formatted as x,y.",
    )
    parser.add_argument(
        "--allow-no-visible-overlay",
        action="store_true",
        help="Return success when layer-shell is unavailable, recording the negative proof.",
    )
    args = parser.parse_args()

    require_wayland_session()
    require_installed(args.capture_command)
    overlay_binary = build_overlay_host()
    point = parse_point(args.point)
    wayland_display = resolve_wayland_display(args.wayland_display)
    env = wayland_overlay_env(wayland_display)
    capture_output = args.capture_output.strip() or detect_capture_output(env, args.capture_command)

    artifact_dir = ARTIFACT_ROOT / datetime.now(UTC).strftime("%Y%m%dT%H%M%S%fZ")
    artifact_dir.mkdir(parents=True, exist_ok=True)
    summary: dict[str, Any] = {
        "mode": "real-session-wayland-layer-shell",
        "artifact_dir": str(artifact_dir),
        "desktop": os.environ.get("XDG_CURRENT_DESKTOP", ""),
        "session_type": os.environ.get("XDG_SESSION_TYPE", ""),
        "wayland_display": wayland_display,
        "capture_command": args.capture_command,
        "capture_output": capture_output,
        "requested_point": {"x": point[0], "y": point[1]},
        "overlay_host": str(overlay_binary),
    }

    overlay: OverlayHostProcess | None = None
    try:
        before_path = artifact_dir / "before.png"
        visible_path = artifact_dir / "visible.png"
        hidden_path = artifact_dir / "hidden.png"

        capture_wayland(
            before_path,
            env,
            command=args.capture_command,
            output_name=capture_output,
        )
        overlay = start_overlay_host(env, overlay_binary)
        capabilities_reply = overlay.send({"version": 1, "kind": "capabilities"})
        set_reply = overlay.send(cursor_message(point, sequence=1))
        time.sleep(0.35)
        capture_wayland(
            visible_path,
            env,
            command=args.capture_command,
            output_name=capture_output,
        )
        visible_probe = probe_marker(before_path, visible_path, point)

        hide_reply = overlay.send({"version": 1, "kind": "hide", "reason": "Wayland capture guard"})
        time.sleep(0.2)
        capture_wayland(
            hidden_path,
            env,
            command=args.capture_command,
            output_name=capture_output,
        )
        hidden_probe = probe_marker(before_path, hidden_path, point)

        backend = backend_from_reply(set_reply)
        visible_overlay = capability_bool(set_reply, "visible_overlay")
        unavailable_ok = (
            args.allow_no_visible_overlay
            and backend == "none"
            and visible_overlay is False
            and not visible_probe.found
        )
        ok = (
            backend == "wayland_layer_shell"
            and visible_overlay is True
            and visible_probe.found
            and not hidden_probe.found
        ) or unavailable_ok

        summary.update(
            {
                "ok": ok,
                "backend": backend,
                "visible_overlay_capability": visible_overlay,
                "click_through_capability": capability_bool(set_reply, "click_through"),
                "system_cursor_hide_supported": capability_bool(
                    set_reply, "system_cursor_hide_supported"
                ),
                "system_cursor_hidden_after_set": capability_bool(
                    set_reply, "system_cursor_hidden"
                ),
                "system_cursor_hidden_after_hide": capability_bool(
                    hide_reply, "system_cursor_hidden"
                ),
                "visible_overlay_captured": visible_probe.found,
                "hidden_overlay_captured": hidden_probe.found,
                "capabilities": capabilities_reply,
                "set_cursor": set_reply,
                "hide": hide_reply,
                "visible_marker_probe": marker_probe_json(visible_probe),
                "hidden_marker_probe": marker_probe_json(hidden_probe),
                "before_screenshot": str(copy_artifact(before_path, artifact_dir, "root-before")),
                "visible_screenshot": str(
                    copy_artifact(visible_path, artifact_dir, "root-visible")
                ),
                "hidden_screenshot": str(copy_artifact(hidden_path, artifact_dir, "root-hidden")),
            }
        )
        write_summary(artifact_dir, summary)
        return 0 if ok else 1
    finally:
        if overlay is not None:
            overlay.close()


def require_wayland_session() -> None:
    session_type = os.environ.get("XDG_SESSION_TYPE", "").strip().lower()
    if session_type != "wayland":
        raise SystemExit(f"this smoke requires XDG_SESSION_TYPE=wayland, got {session_type!r}")
    runtime_dir = Path(os.environ.get("XDG_RUNTIME_DIR", ""))
    if not runtime_dir.is_dir():
        raise SystemExit(f"XDG_RUNTIME_DIR is not a directory: {runtime_dir}")


def require_installed(binary: str) -> None:
    if shutil.which(binary) is None:
        raise RuntimeError(f"required binary is not installed: {binary}")


def build_overlay_host() -> Path:
    prebuilt = overlay_host_path_from_env()
    if os.environ.get("SKY_CUA_USE_PREBUILT_RUNTIMES") == "1" and prebuilt:
        if not prebuilt.exists():
            raise RuntimeError(f"prebuilt overlay host binary does not exist: {prebuilt}")
        return prebuilt
    subprocess.run(
        ["cargo", "build", "--package", "sky-cua-overlay-host"],
        cwd=REPO_ROOT,
        check=True,
    )
    if not OVERLAY_HOST_BIN.exists():
        raise RuntimeError(f"overlay host binary was not built: {OVERLAY_HOST_BIN}")
    return OVERLAY_HOST_BIN


def overlay_host_path_from_env() -> Path | None:
    value = os.environ.get("SKY_CUA_OVERLAY_HOST_PATH") or os.environ.get(
        "SKY_CUA_DEBUG_OVERLAY_HOST_PATH"
    )
    return Path(value) if value else None


def resolve_wayland_display(requested: str) -> str:
    runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
    requested = requested.strip()
    if requested and (runtime_dir / requested).is_socket():
        return requested
    sockets = sorted(
        (path for path in runtime_dir.glob("wayland-*") if path.is_socket()),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    if sockets:
        return sockets[0].name
    if requested:
        raise RuntimeError(f"Wayland socket does not exist: {runtime_dir / requested}")
    raise RuntimeError(f"no wayland-* session socket found under {runtime_dir}")


def parse_point(value: str) -> tuple[float, float]:
    try:
        x_text, y_text = value.split(",", maxsplit=1)
        return (float(x_text), float(y_text))
    except ValueError as exc:
        raise argparse.ArgumentTypeError("--point must be formatted as x,y") from exc


def wayland_overlay_env(wayland_display: str) -> dict[str, str]:
    env = dict(os.environ)
    env["WAYLAND_DISPLAY"] = wayland_display
    env["XDG_SESSION_TYPE"] = "wayland"
    env["SKY_CUA_OVERLAY_BACKEND"] = "layer-shell"
    env["QT_QPA_PLATFORM"] = "wayland"
    env["GDK_BACKEND"] = "wayland"
    env.pop("DISPLAY", None)
    if "hyprland" in env.get("XDG_CURRENT_DESKTOP", "").lower():
        signature = resolve_hyprland_signature(env)
        if signature:
            env["HYPRLAND_INSTANCE_SIGNATURE"] = signature
    return env


def resolve_hyprland_signature(env: Mapping[str, str]) -> str | None:
    signature = env.get("HYPRLAND_INSTANCE_SIGNATURE")
    if signature:
        return signature
    runtime_dir = Path(env["XDG_RUNTIME_DIR"])
    hypr_dir = runtime_dir / "hypr"
    if not hypr_dir.is_dir():
        return None
    candidates = sorted(
        (path for path in hypr_dir.iterdir() if (path / ".socket.sock").is_socket()),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    return candidates[0].name if candidates else None


def detect_capture_output(env: Mapping[str, str], capture_command: str) -> str:
    if Path(capture_command).name != "grim":
        return ""
    if "hyprland" not in env.get("XDG_CURRENT_DESKTOP", "").lower():
        return ""
    try:
        completed = subprocess.run(
            ["hyprctl", "monitors", "-j"],
            cwd=REPO_ROOT,
            env=dict(env),
            check=True,
            text=True,
            capture_output=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return ""
    monitors = json.loads(completed.stdout)
    if not isinstance(monitors, list):
        return ""
    usable = [
        monitor
        for monitor in monitors
        if isinstance(monitor, Mapping)
        and isinstance(monitor.get("name"), str)
        and int(monitor.get("width") or 0) > 0
        and int(monitor.get("height") or 0) > 0
    ]
    focused = [monitor for monitor in usable if monitor.get("focused") is True]
    selected = (focused or usable)[0] if usable else None
    name = selected.get("name") if isinstance(selected, Mapping) else None
    return name if isinstance(name, str) else ""


def start_overlay_host(env: Mapping[str, str], binary: Path) -> OverlayHostProcess:
    process = subprocess.Popen(
        [str(binary), "serve"],
        cwd=REPO_ROOT,
        env=dict(env),
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return OverlayHostProcess(process)


def capture_wayland(
    path: Path,
    env: Mapping[str, str],
    *,
    command: str,
    output_name: str,
) -> None:
    command_args = [command]
    if output_name and Path(command).name == "grim":
        command_args.extend(["-o", output_name])
    command_args.append(str(path))
    subprocess.run(command_args, cwd=REPO_ROOT, env=dict(env), check=True)


def cursor_message(point: tuple[float, float], *, sequence: int) -> dict[str, Any]:
    return {
        "version": 1,
        "kind": "set_cursor",
        "state": {
            "visible": True,
            "sequence": sequence,
            "native_point": {
                "x": point[0],
                "y": point[1],
                "coordinate_space": "desktop_logical",
            },
            "model_point": {
                "x": point[0],
                "y": point[1],
                "coordinate_space": "stream_pixels",
            },
            "source_action": "click",
            "updated_at_ms": int(time.time() * 1000),
        },
    }


def backend_from_reply(reply: Mapping[str, Any]) -> str | None:
    capabilities = reply.get("capabilities")
    if isinstance(capabilities, Mapping):
        backend = capabilities.get("backend")
        if isinstance(backend, str):
            return backend
    return None


def capability_bool(reply: Mapping[str, Any], key: str) -> bool | None:
    capabilities = reply.get("capabilities")
    if isinstance(capabilities, Mapping):
        value = capabilities.get(key)
        if isinstance(value, bool):
            return value
    return None


def marker_probe_json(probe: MarkerProbe) -> dict[str, Any]:
    return {
        "found": probe.found,
        "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
        "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
        "checked_box": list(probe.checked_box),
    }


if __name__ == "__main__":
    raise SystemExit(main())
