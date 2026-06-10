#!/usr/bin/env python3
"""Live KDE smoke for the desktop agent cursor contract.

The synthetic mode is intentionally non-destructive: it starts a private
sky-cua service socket, captures the desktop, sets an agent cursor point in
screenshot-pixel coordinates, captures again, and verifies that the second
model-facing screenshot contains a localized marker near the requested point.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from PIL import Image, ImageChops

import _kwin_effect
from _kwin_effect import (
    KWIN_EFFECT_CURSOR_ASSET,
    KWIN_EFFECT_ID,
    compute_effect_build_id,
    parse_kwin_effect_list,
    set_effect_enabled_config,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = REPO_ROOT / "artifacts" / "codex-e2e" / "agent-cursor-kde"
SERVICE_BIN = Path(
    os.environ.get("SKY_CUA_SERVICE_BIN", REPO_ROOT / "target" / "debug" / "sky-cua-service")
)
OVERLAY_HOST_BIN = Path(
    os.environ.get(
        "SKY_CUA_OVERLAY_HOST_BIN", REPO_ROOT / "target" / "debug" / "sky-cua-overlay-host"
    )
)
POINTER_FIXTURE = REPO_ROOT / "scripts" / "gtk_pointer_smoke_fixture.py"
MODE_ARTIFACT_SLUGS = {
    "synthetic": "syn",
    "layer-shell-debug-visible": "vis",
    "layer-shell-hide-for-capture": "hide",
    "layer-shell-click-through": "click",
    "layer-shell-ydotool-click-through": "ydotool-click",
    "x11-debug-visible": "x11",
    "kwin-effect-static": "kwin",
    "kwin-effect-nested": "kwin-nested",
    "kwin-effect-nested-user-install": "kwin-user",
    "kwin-effect-system-install": "kwin-system",
}
CURSOR_ASSET_SOURCE_WIDTH = 46
CURSOR_ASSET_SOURCE_HEIGHT = 48
CURSOR_ASSET_WIDTH = 23
CURSOR_ASSET_HEIGHT = 24
CURSOR_ASSET_HOTSPOT_X = 10
CURSOR_ASSET_HOTSPOT_Y = 11
KWIN_EFFECT_NESTED_POINT = (420.0, 260.0)


@dataclass(frozen=True)
class MarkerProbe:
    found: bool
    changed_pixels_near_hotspot: int
    max_channel_delta_near_hotspot: int
    checked_box: tuple[int, int, int, int]


def artifact_dir_for_mode(mode: str) -> Path:
    override = os.environ.get("SKY_CUA_KWIN_SYSTEM_INSTALL_ARTIFACT_DIR")
    if mode == "kwin-effect-system-install" and override:
        return Path(override)
    return (
        ARTIFACT_ROOT / f"{datetime.now(UTC).strftime('%m%d%H%M%S%f')}-{MODE_ARTIFACT_SLUGS[mode]}"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Live smoke for the sky-cua agent cursor overlay.")
    parser.add_argument(
        "--mode",
        choices=[
            "synthetic",
            "layer-shell-debug-visible",
            "layer-shell-hide-for-capture",
            "layer-shell-click-through",
            "layer-shell-ydotool-click-through",
            "x11-debug-visible",
            "kwin-effect-static",
            "kwin-effect-nested",
            "kwin-effect-nested-user-install",
            "kwin-effect-system-install",
        ],
        default="synthetic",
    )
    parser.add_argument(
        "--allow-non-kde",
        action="store_true",
        help="Run even when the current session does not look like KDE Wayland.",
    )
    parser.add_argument(
        "--request-timeout",
        type=float,
        default=120.0,
        help="Seconds to wait for each service IPC request.",
    )
    parser.add_argument(
        "--allow-kwin-effect-install",
        action="store_true",
        help="Install and load the user-level KWin compositor-painting effect proof.",
    )
    parser.add_argument(
        "--allow-kwin-effect-system-install",
        action="store_true",
        help=(
            "Install the KWin effect into system paths with sudo, restart the VM Plasma "
            "session, prove discovery/load/rendering, then uninstall it."
        ),
    )
    args = parser.parse_args()

    if args.mode == "kwin-effect-static":
        return run_kwin_effect_static_smoke(args)
    if args.mode == "kwin-effect-nested":
        return run_kwin_effect_nested_smoke(args)
    if args.mode == "kwin-effect-nested-user-install":
        return run_kwin_effect_nested_user_install_smoke(args)
    if args.mode == "kwin-effect-system-install":
        return run_kwin_effect_system_install_smoke(args)
    if args.mode == "layer-shell-debug-visible":
        return run_layer_shell_fixture_visible_smoke(args)
    if args.mode in {"layer-shell-click-through", "layer-shell-ydotool-click-through"}:
        return run_layer_shell_click_through_smoke(args)

    require_kde_wayland(allow_non_kde=args.allow_non_kde)
    build_service()

    artifact_dir = artifact_dir_for_mode(args.mode)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    socket_path = artifact_dir / "svc.sock"
    service = start_service(socket_path, artifact_dir, mode=args.mode)
    try:
        wait_for_socket(socket_path, deadline=time.time() + 15)
        health = service_call(socket_path, {"type": "health"}, timeout=args.request_timeout)
        first = service_call(socket_path, {"type": "get_app_state"}, timeout=args.request_timeout)
        first_snapshot = first["snapshot"]
        capture = require_capture(first_snapshot)
        point = center_point(capture)
        native_point = native_point_from_capture(capture, point)
        state: dict[str, Any] = {
            "visible": True,
            "sequence": 0,
            "model_point": {
                "x": point[0],
                "y": point[1],
                "coordinate_space": "stream_pixels",
                "mapping_id": capture.get("mapping_id"),
            },
            "snapshot_id": first_snapshot["snapshot_id"],
            "source_action": "click",
            "updated_at_ms": 0,
        }
        if native_point is not None:
            state["native_point"] = native_point
        set_response = service_call(
            socket_path,
            {
                "type": "set_agent_cursor",
                "state": state,
            },
            timeout=args.request_timeout,
        )
        status_response = service_call(
            socket_path,
            {"type": "agent_cursor_status"},
            timeout=args.request_timeout,
        )
        assert_no_host_diagnostics(set_response, status_response)
        expected_backend = expected_overlay_backend(args.mode)
        if expected_backend is not None:
            require_cursor_backend_capabilities(
                set_response, status_response, expected_backend=expected_backend
            )
        if args.mode in {
            "layer-shell-debug-visible",
            "layer-shell-hide-for-capture",
            "x11-debug-visible",
        }:
            # The overlay host has committed by now, but KWin/portal capture can still
            # race the next compositor presentation frame without a small settle.
            time.sleep(0.25)
        second = service_call(socket_path, {"type": "get_app_state"}, timeout=args.request_timeout)
        second_snapshot = second["snapshot"]
        second_capture = require_capture(second_snapshot)
        after_path = Path(require_str(second_capture, "screenshot_path"))
        first_path = Path(require_str(capture, "screenshot_path"))
        if args.mode in {"layer-shell-debug-visible", "x11-debug-visible"}:
            before_path = first_path
        else:
            before_path = agent_cursor_source_path(after_path)
        probe = probe_marker(before_path, after_path, point)
        leak_probe = None
        if args.mode == "layer-shell-hide-for-capture":
            leak_probe = probe_marker(first_path, before_path, point)

        summary = {
            "mode": args.mode,
            "ok": probe.found and (leak_probe is None or not leak_probe.found),
            "synthetic_cursor_found": probe.found
            if args.mode not in {"layer-shell-debug-visible", "x11-debug-visible"}
            else False,
            "visible_overlay_captured": probe.found
            if args.mode in {"layer-shell-debug-visible", "x11-debug-visible"}
            else False,
            "native_overlay_hidden_for_capture": None
            if leak_probe is None
            else not leak_probe.found,
            "requested_point": {"x": point[0], "y": point[1]},
            "requested_native_point": native_point,
            "observed_marker_probe": {
                "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
                "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
                "checked_box": list(probe.checked_box),
            },
            "native_overlay_leak_probe": None
            if leak_probe is None
            else {
                "changed_pixels_near_hotspot": leak_probe.changed_pixels_near_hotspot,
                "max_channel_delta_near_hotspot": leak_probe.max_channel_delta_near_hotspot,
                "checked_box": list(leak_probe.checked_box),
            },
            "backend": cursor_backend(status_response),
            "artifact_dir": str(artifact_dir),
            "before_screenshot": str(copy_artifact(before_path, artifact_dir, "before")),
            "after_screenshot": str(copy_artifact(after_path, artifact_dir, "after")),
            "health": health,
            "set_agent_cursor": set_response,
            "agent_cursor_status": status_response,
            "first_snapshot_id": first_snapshot["snapshot_id"],
            "second_snapshot_id": second_snapshot["snapshot_id"],
            "second_agent_cursor": second_snapshot.get("agent_cursor"),
        }
        write_summary(artifact_dir, summary)
        return 0 if summary["ok"] else 1
    finally:
        terminate_service(service)
        socket_path.unlink(missing_ok=True)


def require_kde_wayland(*, allow_non_kde: bool) -> None:
    session_type = os.environ.get("XDG_SESSION_TYPE", "").lower()
    desktop = os.environ.get("XDG_CURRENT_DESKTOP", "").lower()
    desktop_session = os.environ.get("DESKTOP_SESSION", "").lower()
    looks_kde = "kde" in desktop or "plasma" in desktop_session
    if session_type == "wayland" and looks_kde:
        return
    if allow_non_kde:
        return
    raise SystemExit(
        "This smoke is KWin-first and expects a KDE Wayland session. "
        "Pass --allow-non-kde to run the synthetic-only proof elsewhere."
    )


def build_service() -> None:
    if os.environ.get("SKY_CUA_SKIP_LOCAL_BUILD") == "1":
        missing = [str(path) for path in (SERVICE_BIN, OVERLAY_HOST_BIN) if not path.exists()]
        if missing:
            raise RuntimeError(
                "SKY_CUA_SKIP_LOCAL_BUILD=1 but required host-built binaries are missing: "
                + ", ".join(missing)
            )
        return
    subprocess.run(
        [
            "cargo",
            "build",
            "--package",
            "sky-cua-service",
            "--package",
            "sky-cua-overlay-host",
        ],
        cwd=REPO_ROOT,
        check=True,
    )


def start_service(socket_path: Path, artifact_dir: Path, *, mode: str) -> subprocess.Popen[bytes]:
    env = dict(os.environ)
    env["SKY_CUA_SERVICE_SOCKET_PATH"] = str(socket_path)
    env.setdefault("SKY_CUA_AGENT_CURSOR", "always")
    if mode == "kwin-effect-static":
        env["SKY_CUA_AGENT_CURSOR"] = "never"
        env["SKY_CUA_OVERLAY_BACKEND"] = "none"
        env["SKY_CUA_SCREENSHOT_CURSOR"] = "never"
        env["SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE"] = "never"
    if mode in {
        "layer-shell-debug-visible",
        "layer-shell-hide-for-capture",
        "layer-shell-click-through",
        "layer-shell-ydotool-click-through",
    }:
        env["SKY_CUA_OVERLAY_BACKEND"] = "wayland-layer-shell"
    elif mode == "x11-debug-visible":
        env["SKY_CUA_OVERLAY_BACKEND"] = "x11"
    else:
        env.setdefault("SKY_CUA_OVERLAY_BACKEND", "auto")
    env.setdefault("SKY_CUA_OVERLAY_HOST_PATH", str(OVERLAY_HOST_BIN))
    if mode in {
        "layer-shell-debug-visible",
        "layer-shell-click-through",
        "layer-shell-ydotool-click-through",
        "x11-debug-visible",
    }:
        env["SKY_CUA_SCREENSHOT_CURSOR"] = "never"
        env["SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE"] = "never"
    else:
        env.setdefault("SKY_CUA_SCREENSHOT_CURSOR", "always")
        env.setdefault("SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE", "auto")
    stderr = (artifact_dir / "service.stderr.log").open("wb")
    return subprocess.Popen(
        [str(SERVICE_BIN), "daemon"],
        cwd=REPO_ROOT,
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=stderr,
    )


def run_layer_shell_click_through_smoke(args: argparse.Namespace) -> int:
    require_kde_wayland(allow_non_kde=args.allow_non_kde)
    if args.mode == "layer-shell-ydotool-click-through":
        require_commands("ydotool")
    build_service()

    artifact_dir = artifact_dir_for_mode(args.mode)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    socket_path = artifact_dir / "svc.sock"
    pointer_state_path = artifact_dir / "pointer-state.json"
    fixture = start_pointer_fixture(pointer_state_path)
    service = start_service(socket_path, artifact_dir, mode=args.mode)
    try:
        fixture_state = wait_for_stable_pointer_fixture(
            pointer_state_path, deadline=time.time() + 20
        )
        click_logical = fixture_point(fixture_state, "click_button")
        wait_for_socket(socket_path, deadline=time.time() + 15)
        health = service_call(socket_path, {"type": "health"}, timeout=args.request_timeout)
        first = service_call(socket_path, {"type": "get_app_state"}, timeout=args.request_timeout)
        first_snapshot = first["snapshot"]
        capture = require_capture(first_snapshot)
        point = model_point_from_logical_capture(capture, click_logical)
        native_point = native_point_from_capture(capture, point)
        state: dict[str, Any] = {
            "visible": True,
            "sequence": 0,
            "model_point": {
                "x": point[0],
                "y": point[1],
                "coordinate_space": "stream_pixels",
                "mapping_id": capture.get("mapping_id"),
            },
            "snapshot_id": first_snapshot["snapshot_id"],
            "source_action": "click",
            "updated_at_ms": 0,
        }
        if native_point is not None:
            state["native_point"] = native_point
        set_response = service_call(
            socket_path,
            {
                "type": "set_agent_cursor",
                "state": state,
            },
            timeout=args.request_timeout,
        )
        status_response = service_call(
            socket_path,
            {"type": "agent_cursor_status"},
            timeout=args.request_timeout,
        )
        assert_no_host_diagnostics(set_response, status_response)
        require_cursor_backend_capabilities(
            set_response, status_response, expected_backend="wayland_layer_shell"
        )
        before_path = Path(require_str(capture, "screenshot_path"))
        visible, visible_capture, visible_probe = capture_until_marker(
            socket_path,
            before_path,
            point,
            request_timeout=args.request_timeout,
            deadline=time.time() + 3.0,
        )
        visible_snapshot = visible["snapshot"]
        visible_path = Path(require_str(visible_capture, "screenshot_path"))

        click_response: dict[str, Any]
        click_succeeded = False
        if args.mode == "layer-shell-ydotool-click-through":
            click_response = ydotool_click(click_logical)
            click_succeeded = click_response.get("success") is True
        else:
            click_response = service_call(
                socket_path,
                execute_click_request(visible_snapshot["snapshot_id"], point),
                timeout=args.request_timeout,
            )
            click_outcome = click_response.get("outcome")
            if not isinstance(click_outcome, Mapping):
                raise RuntimeError(
                    "execute_action did not return an outcome: "
                    + json.dumps(click_response, indent=2, sort_keys=True)
                )
            click_succeeded = click_outcome.get("success") is True
        clicked_state = wait_for_pointer_click(pointer_state_path, deadline=time.time() + 8)
        requires_portal_visible_overlay = args.mode == "layer-shell-click-through"
        click_through_proved = (
            click_succeeded
            and clicked_state is not None
            and (visible_probe.found or not requires_portal_visible_overlay)
        )
        summary = {
            "mode": args.mode,
            "ok": click_through_proved,
            "visible_overlay_captured": visible_probe.found,
            "click_through_proved": click_through_proved,
            "target_clicked": clicked_state is not None,
            "click_mechanism": "ydotool"
            if args.mode == "layer-shell-ydotool-click-through"
            else "portal_execute_action",
            "requested_point": {"x": point[0], "y": point[1]},
            "requested_logical_point": click_logical,
            "requested_native_point": native_point,
            "observed_marker_probe": {
                "changed_pixels_near_hotspot": visible_probe.changed_pixels_near_hotspot,
                "max_channel_delta_near_hotspot": visible_probe.max_channel_delta_near_hotspot,
                "checked_box": list(visible_probe.checked_box),
            },
            "backend": cursor_backend(status_response),
            "artifact_dir": str(artifact_dir),
            "before_screenshot": str(copy_artifact(before_path, artifact_dir, "before")),
            "visible_screenshot": str(copy_artifact(visible_path, artifact_dir, "visible")),
            "health": health,
            "set_agent_cursor": set_response,
            "agent_cursor_status": status_response,
            "execute_action": click_response,
            "pointer_fixture_initial_state": fixture_state,
            "pointer_fixture_clicked_state": clicked_state,
            "first_snapshot_id": first_snapshot["snapshot_id"],
            "visible_snapshot_id": visible_snapshot["snapshot_id"],
            "visible_agent_cursor": visible_snapshot.get("agent_cursor"),
        }
        write_summary(artifact_dir, summary)
        return 0 if summary["ok"] else 1
    finally:
        terminate_service(service)
        terminate_process(fixture, name="GTK pointer fixture")
        socket_path.unlink(missing_ok=True)


def run_layer_shell_fixture_visible_smoke(args: argparse.Namespace) -> int:
    require_kde_wayland(allow_non_kde=args.allow_non_kde)
    build_service()

    artifact_dir = artifact_dir_for_mode(args.mode)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    socket_path = artifact_dir / "svc.sock"
    pointer_state_path = artifact_dir / "pointer-state.json"
    fixture = start_pointer_fixture(pointer_state_path)
    service = start_service(socket_path, artifact_dir, mode=args.mode)
    try:
        fixture_state = wait_for_stable_pointer_fixture(
            pointer_state_path, deadline=time.time() + 20
        )
        point_logical = fixture_point(fixture_state, "click_button")
        wait_for_socket(socket_path, deadline=time.time() + 15)
        health = service_call(socket_path, {"type": "health"}, timeout=args.request_timeout)
        first = service_call(socket_path, {"type": "get_app_state"}, timeout=args.request_timeout)
        first_snapshot = first["snapshot"]
        capture = require_capture(first_snapshot)
        point = model_point_from_logical_capture(capture, point_logical)
        native_point = native_point_from_capture(capture, point)
        state: dict[str, Any] = {
            "visible": True,
            "sequence": 0,
            "model_point": {
                "x": point[0],
                "y": point[1],
                "coordinate_space": "stream_pixels",
                "mapping_id": capture.get("mapping_id"),
            },
            "snapshot_id": first_snapshot["snapshot_id"],
            "source_action": "click",
            "updated_at_ms": 0,
        }
        if native_point is not None:
            state["native_point"] = native_point
        set_response = service_call(
            socket_path,
            {
                "type": "set_agent_cursor",
                "state": state,
            },
            timeout=args.request_timeout,
        )
        status_response = service_call(
            socket_path,
            {"type": "agent_cursor_status"},
            timeout=args.request_timeout,
        )
        assert_no_host_diagnostics(set_response, status_response)
        require_cursor_backend_capabilities(
            set_response, status_response, expected_backend="wayland_layer_shell"
        )
        before_path = Path(require_str(capture, "screenshot_path"))
        visible, visible_capture, visible_probe = capture_until_marker(
            socket_path,
            before_path,
            point,
            request_timeout=args.request_timeout,
            deadline=time.time() + 3.0,
        )
        visible_snapshot = visible["snapshot"]
        visible_path = Path(require_str(visible_capture, "screenshot_path"))
        summary = {
            "mode": args.mode,
            "ok": visible_probe.found,
            "synthetic_cursor_found": False,
            "visible_overlay_captured": visible_probe.found,
            "native_overlay_hidden_for_capture": None,
            "requested_point": {"x": point[0], "y": point[1]},
            "requested_logical_point": point_logical,
            "requested_native_point": native_point,
            "observed_marker_probe": {
                "changed_pixels_near_hotspot": visible_probe.changed_pixels_near_hotspot,
                "max_channel_delta_near_hotspot": visible_probe.max_channel_delta_near_hotspot,
                "checked_box": list(visible_probe.checked_box),
            },
            "backend": cursor_backend(status_response),
            "artifact_dir": str(artifact_dir),
            "before_screenshot": str(copy_artifact(before_path, artifact_dir, "before")),
            "visible_screenshot": str(copy_artifact(visible_path, artifact_dir, "visible")),
            "health": health,
            "set_agent_cursor": set_response,
            "agent_cursor_status": status_response,
            "pointer_fixture_initial_state": fixture_state,
            "first_snapshot_id": first_snapshot["snapshot_id"],
            "visible_snapshot_id": visible_snapshot["snapshot_id"],
            "visible_agent_cursor": visible_snapshot.get("agent_cursor"),
        }
        write_summary(artifact_dir, summary)
        return 0 if summary["ok"] else 1
    finally:
        terminate_service(service)
        terminate_process(fixture, name="GTK pointer fixture")
        socket_path.unlink(missing_ok=True)


def run_kwin_effect_static_smoke(args: argparse.Namespace) -> int:
    if not args.allow_kwin_effect_install:
        raise SystemExit(
            "kwin-effect-static installs and loads a user-level KWin C++ effect. "
            "Pass --allow-kwin-effect-install to run the explicit compositor proof."
        )

    require_kde_wayland(allow_non_kde=args.allow_non_kde)
    build_service()

    artifact_dir = artifact_dir_for_mode(args.mode)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    socket_path = artifact_dir / "svc.sock"
    installed_files: list[Path] = []
    cleanup: dict[str, Any] | None = None
    discovery_before_install: dict[str, Any] | None = None
    discovery_after_install: dict[str, Any] | None = None
    service = start_service(socket_path, artifact_dir, mode=args.mode)
    try:
        wait_for_socket(socket_path, deadline=time.time() + 15)
        health = service_call(socket_path, {"type": "health"}, timeout=args.request_timeout)
        first = service_call(socket_path, {"type": "get_app_state"}, timeout=args.request_timeout)
        first_snapshot = first["snapshot"]
        capture = require_capture(first_snapshot)
        point = center_point(capture)
        first_path = Path(require_str(capture, "screenshot_path"))

        discovery_before_install = kwin_effect_discovery()
        install = build_and_install_kwin_effect(artifact_dir)
        installed_files = install["installed_files"]
        discovery_after_install = kwin_effect_discovery()
        load = load_kwin_effect()
        time.sleep(0.75)

        second = service_call(socket_path, {"type": "get_app_state"}, timeout=args.request_timeout)
        second_snapshot = second["snapshot"]
        second_capture = require_capture(second_snapshot)
        after_path = Path(require_str(second_capture, "screenshot_path"))
        probe = probe_marker(first_path, after_path, point)
    finally:
        cleanup = cleanup_kwin_effect(installed_files)
        terminate_service(service)
        socket_path.unlink(missing_ok=True)

    summary = {
        "mode": args.mode,
        "ok": probe.found and cleanup.get("effect_loaded_after_cleanup") is False,
        "kwin_effect_static_marker_found": probe.found,
        "visible_overlay_captured": probe.found,
        "requested_point": {"x": point[0], "y": point[1]},
        "observed_marker_probe": {
            "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
            "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
            "checked_box": list(probe.checked_box),
        },
        "artifact_dir": str(artifact_dir),
        "before_screenshot": str(copy_artifact(first_path, artifact_dir, "before")),
        "after_screenshot": str(copy_artifact(after_path, artifact_dir, "after")),
        "health": health,
        "kwin_effect_install": {
            "build_dir": str(install["build_dir"]),
            "installed_files": [str(path) for path in installed_files],
        },
        "kwin_effect_discovery_before_install": discovery_before_install,
        "kwin_effect_discovery_after_install": discovery_after_install,
        "kwin_effect_load": load,
        "kwin_effect_cleanup": cleanup,
        "first_snapshot_id": first_snapshot["snapshot_id"],
        "second_snapshot_id": second_snapshot["snapshot_id"],
    }
    write_summary(artifact_dir, summary)
    return 0 if summary["ok"] else 1


def run_kwin_effect_nested_smoke(args: argparse.Namespace) -> int:
    require_commands(
        "cargo", "cmake", "dbus-run-session", "kwin_wayland", "ninja", "python3", "qdbus6"
    )

    artifact_dir = artifact_dir_for_mode(args.mode)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    install_prefix = artifact_dir / "kwin-effect-prefix"
    install = build_and_install_kwin_effect(artifact_dir, install_prefix=install_prefix)
    overlay_host_path = build_overlay_host_binary()
    session = run_nested_kwin_effect_session(artifact_dir, install_prefix, overlay_host_path)

    capture_path: Path | None = None
    probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    conversion_error: str | None = None
    try:
        capture_path = convert_kwin_raw_screenshot(artifact_dir)
        probe = probe_cursor_asset_presence(capture_path, KWIN_EFFECT_NESTED_POINT)
    except Exception as error:
        conversion_error = str(error)

    loaded_text = (
        (artifact_dir / "nested-effect-loaded.txt").read_text(encoding="utf-8").strip()
        if (artifact_dir / "nested-effect-loaded.txt").exists()
        else ""
    )
    overlay_host_set_reply = read_json_if_exists(
        artifact_dir / "nested-overlay-host-set-state.json"
    )
    overlay_host_ok = (
        isinstance(overlay_host_set_reply, Mapping)
        and overlay_host_set_reply.get("ok") is True
        and isinstance(overlay_host_set_reply.get("capabilities"), Mapping)
        and overlay_host_set_reply["capabilities"].get("backend") == "kwin_effect"
    )
    accept_ipc_only = os.environ.get("SKY_CUA_KWIN_NESTED_ACCEPT_IPC_ONLY") == "1"
    code_level_ok = session.returncode == 0 and loaded_text.lower() == "true" and overlay_host_ok
    summary = {
        "mode": args.mode,
        "ok": code_level_ok and (probe.found or accept_ipc_only),
        "accepted_without_headless_pixel_proof": code_level_ok
        and accept_ipc_only
        and not probe.found,
        "artifact_dir": str(artifact_dir),
        "kwin_nested_effect_marker_found": probe.found,
        "visible_overlay_captured": probe.found,
        "requested_point": {
            "x": KWIN_EFFECT_NESTED_POINT[0],
            "y": KWIN_EFFECT_NESTED_POINT[1],
        },
        "observed_marker_probe": {
            "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
            "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
            "checked_box": list(probe.checked_box),
        },
        "capture_screenshot": str(capture_path) if capture_path else None,
        "capture_conversion_error": conversion_error,
        "kwin_effect_install": {
            "build_dir": str(install["build_dir"]),
            "install_prefix": str(install_prefix),
            "installed_files": [str(path) for path in install["installed_files"]],
        },
        "overlay_host_path": str(overlay_host_path),
        "overlay_host_set_reply": overlay_host_set_reply,
        "nested_kwin": {
            "returncode": session.returncode,
            "stdout": session.stdout.strip(),
            "stderr": session.stderr.strip(),
            "effect_list": read_text_if_exists(artifact_dir / "nested-effects-list.txt"),
            "load_stdout": read_text_if_exists(artifact_dir / "nested-effect-load.txt"),
            "set_state_stdout": read_text_if_exists(
                artifact_dir / "nested-overlay-host-set-state.json"
            ),
            "state_readback": read_text_if_exists(artifact_dir / "nested-effect-state.json"),
            "effect_loaded": loaded_text,
        },
    }
    write_summary(artifact_dir, summary)
    return 0 if summary["ok"] else 1


def run_kwin_effect_nested_user_install_smoke(args: argparse.Namespace) -> int:
    require_commands(
        "cargo", "cmake", "dbus-run-session", "kwin_wayland", "ninja", "python3", "qdbus6"
    )

    artifact_dir = artifact_dir_for_mode(args.mode)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    nested_home = artifact_dir / "home"
    install_prefix = nested_home / ".local"
    install = build_and_install_kwin_effect(artifact_dir, install_prefix=install_prefix)
    overlay_host_path = build_overlay_host_binary()
    session = run_nested_kwin_effect_session(
        artifact_dir,
        install_prefix,
        overlay_host_path,
        force_plugin_paths=False,
        nested_home=nested_home,
    )

    loaded_text = read_text_if_exists(artifact_dir / "nested-effect-loaded.txt") or ""
    load_stdout = read_text_if_exists(artifact_dir / "nested-effect-load.txt") or ""
    effect_list = parse_kwin_effect_list(
        read_text_if_exists(artifact_dir / "nested-effects-list.txt") or ""
    )
    overlay_host_set_reply = read_json_if_exists(
        artifact_dir / "nested-overlay-host-set-state.json"
    )
    discovered = KWIN_EFFECT_ID in effect_list
    loaded = loaded_text.lower() == "true"

    capture_path: Path | None = None
    probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    conversion_error: str | None = None
    if loaded:
        try:
            capture_path = convert_kwin_raw_screenshot(artifact_dir)
            probe = probe_cursor_asset_presence(capture_path, KWIN_EFFECT_NESTED_POINT)
        except Exception as error:
            conversion_error = str(error)

    summary = {
        "mode": args.mode,
        "ok": session.returncode == 0,
        "artifact_dir": str(artifact_dir),
        "nested_home": str(nested_home),
        "kwin_user_install_discovered": discovered,
        "kwin_user_install_loaded": loaded,
        "kwin_user_install_load_stdout": load_stdout,
        "kwin_nested_effect_marker_found": probe.found,
        "visible_overlay_captured": probe.found,
        "requested_point": {
            "x": KWIN_EFFECT_NESTED_POINT[0],
            "y": KWIN_EFFECT_NESTED_POINT[1],
        },
        "observed_marker_probe": {
            "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
            "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
            "checked_box": list(probe.checked_box),
        },
        "capture_screenshot": str(capture_path) if capture_path else None,
        "capture_conversion_error": conversion_error,
        "kwin_effect_install": {
            "build_dir": str(install["build_dir"]),
            "install_prefix": str(install_prefix),
            "installed_files": [str(path) for path in install["installed_files"]],
        },
        "overlay_host_path": str(overlay_host_path),
        "overlay_host_set_reply": overlay_host_set_reply,
        "nested_kwin": {
            "returncode": session.returncode,
            "stdout": session.stdout.strip(),
            "stderr": session.stderr.strip(),
            "effect_list": "\n".join(effect_list),
            "load_stdout": load_stdout,
            "set_state_stdout": read_text_if_exists(
                artifact_dir / "nested-overlay-host-set-state.json"
            ),
            "state_readback": read_text_if_exists(artifact_dir / "nested-effect-state.json"),
            "effect_loaded": loaded_text,
        },
    }
    write_summary(artifact_dir, summary)
    return 0 if summary["ok"] else 1


def run_kwin_effect_system_install_smoke(args: argparse.Namespace) -> int:
    if not args.allow_kwin_effect_system_install:
        raise SystemExit(
            "kwin-effect-system-install writes KWin effect files under /usr with sudo, "
            "restarts the VM Plasma session, and removes those files afterward. "
            "Pass --allow-kwin-effect-system-install to run it."
        )
    require_kde_wayland(allow_non_kde=args.allow_non_kde)
    require_commands("cmake", "ninja", "python3", "qdbus6", "sudo")

    artifact_dir = artifact_dir_for_mode(args.mode)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    discovery_before_install = kwin_effect_discovery()
    install = build_and_install_kwin_effect(
        artifact_dir,
        install_prefix=Path("/usr"),
        install_command_prefix=["sudo"],
    )
    installed_files = install["installed_files"]
    restart_after_install = restart_testing_vm_plasma_session()
    wait_for_kwin_dbus(deadline=time.time() + 60)
    discovery_after_restart = kwin_effect_discovery()
    load = load_kwin_effect()
    overlay_host_path = build_overlay_host_binary()
    set_reply = run_overlay_host_message(
        overlay_host_path,
        kwin_effect_overlay_host_set_cursor_json(KWIN_EFFECT_NESTED_POINT),
    )
    host_framebuffer_expected = (
        os.environ.get("SKY_CUA_KWIN_SYSTEM_INSTALL_HOST_FRAMEBUFFER_PROOF") == "1"
    )
    hold_seconds = float(os.environ.get("SKY_CUA_KWIN_SYSTEM_INSTALL_HOLD_SECONDS", "0") or "0")
    ready_path: Path | None = None
    if host_framebuffer_expected:
        ready_path = artifact_dir / "host-framebuffer-ready.json"
        ready_path.write_text(
            json.dumps(
                {
                    "ready": True,
                    "requested_point": {
                        "x": KWIN_EFFECT_NESTED_POINT[0],
                        "y": KWIN_EFFECT_NESTED_POINT[1],
                    },
                    "overlay_host_set_reply": set_reply,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
    if hold_seconds > 0:
        time.sleep(hold_seconds)
    time.sleep(1.0)
    state_readback = run_kwin_agent_cursor_state()
    capture_error: str | None = None
    capture_path: Path | None = None
    probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    if host_framebuffer_expected:
        capture_error = "host framebuffer proof expected by VM runner"
    else:
        try:
            capture_kwin_workspace_raw(artifact_dir)
            capture_path = convert_kwin_raw_screenshot(artifact_dir)
            probe = probe_cursor_asset_presence(capture_path, KWIN_EFFECT_NESTED_POINT)
        except Exception as error:
            capture_error = f"{type(error).__name__}: {error}"

    hide_reply = run_overlay_host_message(
        overlay_host_path,
        json.dumps({"version": 1, "kind": "hide", "reason": "system-install cleanup"}),
        check=False,
    )
    cleanup = cleanup_system_kwin_effect(installed_files)
    restart_after_cleanup = restart_testing_vm_plasma_session()
    wait_for_kwin_dbus(deadline=time.time() + 60)
    discovery_after_cleanup = kwin_effect_discovery()
    leftovers = find_system_kwin_effect_leftovers()

    overlay_host_ok = (
        set_reply.get("ok") is True
        and isinstance(set_reply.get("capabilities"), Mapping)
        and set_reply["capabilities"].get("backend") == "kwin_effect"
        and set_reply["capabilities"].get("system_cursor_hidden") is True
    )
    cleanup_ok = (
        not discovery_after_cleanup["listed"]
        and not discovery_after_cleanup["loaded"]
        and not leftovers
    )
    summary = {
        "mode": args.mode,
        "ok": (
            discovery_after_restart["listed"]
            and load["effect_loaded"]
            and overlay_host_ok
            and probe.found
            and cleanup_ok
        ),
        "artifact_dir": str(artifact_dir),
        "kwin_system_install_marker_found": probe.found,
        "host_framebuffer_proof_expected": host_framebuffer_expected,
        "host_framebuffer_ready": str(ready_path) if ready_path else None,
        "visible_overlay_captured": probe.found,
        "requested_point": {
            "x": KWIN_EFFECT_NESTED_POINT[0],
            "y": KWIN_EFFECT_NESTED_POINT[1],
        },
        "observed_marker_probe": {
            "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
            "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
            "checked_box": list(probe.checked_box),
        },
        "capture_screenshot": str(capture_path) if capture_path else None,
        "capture_error": capture_error,
        "kwin_effect_install": {
            "build_dir": str(install["build_dir"]),
            "install_prefix": "/usr",
            "installed_files": [str(path) for path in installed_files],
        },
        "kwin_effect_discovery_before_install": discovery_before_install,
        "kwin_effect_discovery_after_restart": discovery_after_restart,
        "kwin_effect_load": load,
        "overlay_host_path": str(overlay_host_path),
        "overlay_host_set_reply": set_reply,
        "overlay_host_hide_reply": hide_reply,
        "state_readback": state_readback,
        "cleanup": cleanup,
        "restart_after_install": process_summary(restart_after_install),
        "restart_after_cleanup": process_summary(restart_after_cleanup),
        "kwin_effect_discovery_after_cleanup": discovery_after_cleanup,
        "system_leftovers_after_cleanup": [str(path) for path in leftovers],
    }
    if host_framebuffer_expected:
        summary["ok"] = (
            discovery_after_restart["listed"]
            and load["effect_loaded"]
            and overlay_host_ok
            and cleanup_ok
        )
    write_summary(artifact_dir, summary)
    return 0 if summary["ok"] else 1


def build_and_install_kwin_effect(
    artifact_dir: Path,
    *,
    install_prefix: Path | None = None,
    install_command_prefix: list[str] | None = None,
) -> dict[str, Any]:
    if not KWIN_EFFECT_CURSOR_ASSET.exists():
        raise RuntimeError(f"KWin effect cursor asset is missing: {KWIN_EFFECT_CURSOR_ASSET}")
    build_dir = artifact_dir / "kwin-effect-build"
    install_prefix = install_prefix or Path.home() / ".local"
    subprocess.run(
        _kwin_effect.cmake_configure_command(
            build_dir,
            install_prefix=install_prefix,
            build_id=compute_effect_build_id(),
        ),
        cwd=REPO_ROOT,
        check=True,
    )
    subprocess.run(_kwin_effect.cmake_build_command(build_dir), cwd=REPO_ROOT, check=True)
    subprocess.run(
        _kwin_effect.cmake_install_command(build_dir, sudo_cmd=install_command_prefix or []),
        cwd=REPO_ROOT,
        check=True,
    )
    manifest = build_dir / "install_manifest.txt"
    installed_files = [
        Path(line) for line in manifest.read_text(encoding="utf-8").splitlines() if line.strip()
    ]
    return {"build_dir": build_dir, "installed_files": installed_files}


def build_overlay_host_binary() -> Path:
    prebuilt = os.environ.get("SKY_CUA_OVERLAY_HOST_PATH") or os.environ.get(
        "SKY_CUA_DEBUG_OVERLAY_HOST_PATH"
    )
    if os.environ.get("SKY_CUA_USE_PREBUILT_RUNTIMES") == "1" and prebuilt:
        binary = Path(prebuilt)
        if not binary.exists():
            raise RuntimeError(f"prebuilt overlay host binary does not exist: {binary}")
        return binary
    subprocess.run(
        ["cargo", "build", "-p", "sky-cua-overlay-host"],
        cwd=REPO_ROOT,
        check=True,
    )
    binary = REPO_ROOT / "target" / "debug" / "sky-cua-overlay-host"
    if not binary.exists():
        raise RuntimeError(f"overlay host binary was not built: {binary}")
    return binary


def kwin_effect_cursor_state_json(point: tuple[float, float]) -> str:
    return json.dumps(
        {
            "visible": True,
            "sequence": 1,
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
            "updated_at_ms": int(time.time() * 1000),
        },
        separators=(",", ":"),
        sort_keys=True,
    )


def kwin_effect_overlay_host_set_cursor_json(point: tuple[float, float]) -> str:
    return json.dumps(
        {
            "version": 1,
            "kind": "set_cursor",
            "state": json.loads(kwin_effect_cursor_state_json(point)),
        },
        separators=(",", ":"),
        sort_keys=True,
    )


def run_overlay_host_message(
    overlay_host_path: Path,
    message_json: str,
    *,
    check: bool = True,
) -> dict[str, Any]:
    completed = subprocess.run(
        [str(overlay_host_path), "serve"],
        cwd=REPO_ROOT,
        input=message_json + "\n",
        text=True,
        capture_output=True,
        check=False,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(
            "overlay host message failed:\n"
            f"returncode={completed.returncode}\nstdout={completed.stdout}\nstderr={completed.stderr}"
        )
    first_line = completed.stdout.splitlines()[0] if completed.stdout.splitlines() else "{}"
    reply = json.loads(first_line)
    if not isinstance(reply, dict):
        raise RuntimeError(f"overlay host reply was not an object: {reply!r}")
    if completed.stderr.strip():
        reply.setdefault("stderr", completed.stderr.strip())
    if completed.returncode != 0:
        reply.setdefault("returncode", completed.returncode)
    return reply


def run_kwin_agent_cursor_state() -> str:
    completed = subprocess.run(
        [
            "qdbus6",
            "org.kde.KWin",
            "/com/skycua/AgentCursor",
            "com.skycua.AgentCursor.StateJson",
        ],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    return completed.stdout.strip()


def restart_testing_vm_plasma_session() -> subprocess.CompletedProcess[str]:
    selector = REPO_ROOT / "scripts" / "testing-vm" / "select-session.sh"
    if not selector.exists():
        raise RuntimeError(
            "kwin-effect-system-install requires scripts/testing-vm/select-session.sh "
            "inside the testing VM checkout"
        )
    return subprocess.run(
        ["sudo", str(selector), "plasma"],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )


def wait_for_kwin_dbus(*, deadline: float) -> None:
    while time.time() < deadline:
        ready = subprocess.run(
            ["qdbus6", "org.kde.KWin", "/KWin", "currentDesktop"],
            cwd=REPO_ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        if ready.returncode == 0:
            return
        time.sleep(0.5)
    raise RuntimeError("timed out waiting for KWin DBus after Plasma session restart")


def capture_kwin_workspace_raw(artifact_dir: Path) -> None:
    capture_script = r"""
import json
import os
from pathlib import Path

import dbus

out = Path(os.environ["SHOT_DIR"])
bus = dbus.SessionBus()
proxy = bus.get_object("org.kde.KWin.ScreenShot2", "/org/kde/KWin/ScreenShot2")
iface = dbus.Interface(proxy, "org.kde.KWin.ScreenShot2")
rfd, wfd = os.pipe()
try:
    result = iface.CaptureWorkspace({}, dbus.types.UnixFd(wfd), timeout=10)
finally:
    os.close(wfd)
chunks = []
expected_size = int(result["stride"]) * int(result["height"])
received = 0
while received < expected_size:
    chunk = os.read(rfd, 1024 * 1024)
    if not chunk:
        break
    chunks.append(chunk)
    received += len(chunk)
os.close(rfd)
attrs = {str(key): str(value) for key, value in dict(result).items()}
(out / "nested-screenshot-attrs.json").write_text(
    json.dumps(attrs, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
(out / "nested-screenshot.raw").write_bytes(b"".join(chunks))
"""
    env = dict(os.environ)
    env["SHOT_DIR"] = str(artifact_dir)
    env["KWIN_SCREENSHOT_NO_PERMISSION_CHECKS"] = "1"
    subprocess.run(
        [gtk_fixture_python(), "-c", capture_script],
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=True,
    )


def run_nested_kwin_effect_session(
    artifact_dir: Path,
    install_prefix: Path,
    overlay_host_path: Path,
    *,
    force_plugin_paths: bool = True,
    nested_home: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    session_script = artifact_dir / "nested-kwin-session.sh"
    session_script.write_text(
        """#!/usr/bin/env bash
set -eu
for i in $(seq 1 80); do
  if qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.listOfEffects >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.listOfEffects >"$SHOT_DIR/nested-effects-list.txt"
qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.loadEffect "$KWIN_EFFECT_ID" >"$SHOT_DIR/nested-effect-load.txt"
sleep 0.2
if grep -qi '^true$' "$SHOT_DIR/nested-effect-load.txt"; then
  printf '%s\\n' "$KWIN_OVERLAY_HOST_SET_CURSOR_JSON" | "$SKY_CUA_OVERLAY_HOST_PATH" serve >"$SHOT_DIR/nested-overlay-host-set-state.json"
  qdbus6 org.kde.KWin /com/skycua/AgentCursor com.skycua.AgentCursor.StateJson >"$SHOT_DIR/nested-effect-state.json"
  sleep 1
else
  printf '{"skipped":true,"reason":"KWin effect was not loaded"}\\n' >"$SHOT_DIR/nested-overlay-host-set-state.json"
fi
"$SKY_CUA_SYSTEM_PYTHON" - <<'PY'
import json
import os
from pathlib import Path

import dbus

out = Path(os.environ["SHOT_DIR"])
try:
    bus = dbus.SessionBus()
    proxy = bus.get_object("org.kde.KWin.ScreenShot2", "/org/kde/KWin/ScreenShot2")
    iface = dbus.Interface(proxy, "org.kde.KWin.ScreenShot2")
    rfd, wfd = os.pipe()
    try:
        result = iface.CaptureWorkspace({}, dbus.types.UnixFd(wfd), timeout=10)
    finally:
        os.close(wfd)
    chunks = []
    expected_size = int(result["stride"]) * int(result["height"])
    received = 0
    while received < expected_size:
        chunk = os.read(rfd, 1024 * 1024)
        if not chunk:
            break
        chunks.append(chunk)
        received += len(chunk)
    os.close(rfd)
    attrs = {str(key): str(value) for key, value in dict(result).items()}
    (out / "nested-screenshot-attrs.json").write_text(json.dumps(attrs, indent=2, sort_keys=True) + "\\n")
    (out / "nested-screenshot.raw").write_bytes(b"".join(chunks))
except Exception as error:
    (out / "nested-screenshot-error.txt").write_text(f"{type(error).__name__}: {error}\\n")
PY
if [ ! -s "$SHOT_DIR/nested-screenshot.raw" ] && command -v grim >/dev/null 2>&1; then
  grim "$SHOT_DIR/nested-kwin-effect-capture.png" || true
fi
qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.isEffectLoaded "$KWIN_EFFECT_ID" >"$SHOT_DIR/nested-effect-loaded.txt" || true
""",
        encoding="utf-8",
    )
    session_script.chmod(0o755)
    env = dict(os.environ)
    env["KWIN_EFFECT_ID"] = KWIN_EFFECT_ID
    env["KWIN_SCREENSHOT_NO_PERMISSION_CHECKS"] = "1"
    env["KWIN_OVERLAY_HOST_SET_CURSOR_JSON"] = kwin_effect_overlay_host_set_cursor_json(
        KWIN_EFFECT_NESTED_POINT
    )
    if nested_home is not None:
        env["HOME"] = str(nested_home)
        env["XDG_DATA_HOME"] = str(nested_home / ".local" / "share")
    if force_plugin_paths:
        env["QT_PLUGIN_PATH"] = str(install_prefix / "lib" / "qt6" / "plugins")
    env["SHOT_DIR"] = str(artifact_dir)
    env["SKY_CUA_OVERLAY_HOST_PATH"] = str(overlay_host_path)
    env["SKY_CUA_SYSTEM_PYTHON"] = gtk_fixture_python()
    if force_plugin_paths:
        env["XDG_DATA_DIRS"] = (
            f"{install_prefix / 'share'}:{os.environ.get('XDG_DATA_DIRS', '/usr/local/share:/usr/share')}"
        )
    elif "XDG_DATA_DIRS" not in env:
        env["XDG_DATA_DIRS"] = "/usr/local/share:/usr/share"
    command = [
        "dbus-run-session",
        "--",
        "kwin_wayland",
        "--virtual",
        "--width",
        "640",
        "--height",
        "480",
        "--no-lockscreen",
        "--no-global-shortcuts",
        "--socket",
        f"sky-cua-kwin-effect-{artifact_dir.name}",
        "--exit-with-session",
        str(session_script),
    ]
    stdout_path = artifact_dir / "nested-kwin.stdout.log"
    stderr_path = artifact_dir / "nested-kwin.stderr.log"
    with (
        stdout_path.open("w", encoding="utf-8") as stdout,
        stderr_path.open("w", encoding="utf-8") as stderr,
    ):
        completed = subprocess.run(
            command,
            cwd=REPO_ROOT,
            env=env,
            text=True,
            stdout=stdout,
            stderr=stderr,
            timeout=60,
            check=False,
        )
    return subprocess.CompletedProcess(
        args=completed.args,
        returncode=completed.returncode,
        stdout=stdout_path.read_text(encoding="utf-8"),
        stderr=stderr_path.read_text(encoding="utf-8"),
    )


def load_kwin_effect() -> dict[str, Any]:
    disable_kwin_effect_config()
    run_kwin_effect_command("unloadEffect", check=False)
    run_kwin_reconfigure(check=False)
    after_reconfigure = kwin_effect_discovery()
    load = run_kwin_effect_command("loadEffect", check=False)
    time.sleep(0.5)
    supported = kwin_effect_supported()
    loaded = kwin_effect_loaded()
    return {
        "load_stdout": load.stdout.strip(),
        "load_stderr": load.stderr.strip(),
        "load_returncode": load.returncode,
        "effect_supported": supported,
        "effect_loaded": loaded,
        "discovery_after_reconfigure": after_reconfigure,
        "discovery_after_load": kwin_effect_discovery(),
    }


def cleanup_kwin_effect(installed_files: list[Path]) -> dict[str, Any]:
    unload = run_kwin_effect_command("unloadEffect", check=False)
    disable_kwin_effect_config()
    reconfigure = run_kwin_reconfigure(check=False)
    time.sleep(0.25)
    loaded_after_cleanup = kwin_effect_loaded()
    removed_files: list[str] = []
    manual_cleanup: list[str] = []
    if loaded_after_cleanup:
        manual_cleanup = [
            f"qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.unloadEffect {KWIN_EFFECT_ID}",
            f"kwriteconfig6 --file kwinrc --group Plugins --key {KWIN_EFFECT_ID}Enabled false",
            "qdbus6 org.kde.KWin /KWin reconfigure",
        ]
    else:
        for path in reversed(installed_files):
            if path.is_file():
                path.unlink()
                removed_files.append(str(path))
        prune_empty_parent(KWIN_EFFECT_ID)
    return {
        "unload_stdout": unload.stdout.strip(),
        "unload_stderr": unload.stderr.strip(),
        "reconfigure_returncode": reconfigure.returncode,
        "effect_loaded_after_cleanup": loaded_after_cleanup,
        "discovery_after_cleanup": kwin_effect_discovery(),
        "removed_files": removed_files,
        "manual_cleanup": manual_cleanup,
    }


def cleanup_system_kwin_effect(installed_files: list[Path]) -> dict[str, Any]:
    unload = run_kwin_effect_command("unloadEffect", check=False)
    disable_kwin_effect_config()
    removed_files: list[str] = []
    failed_files: list[dict[str, str]] = []
    for path in reversed(installed_files):
        remove = subprocess.run(
            ["sudo", "rm", "-f", str(path)],
            cwd=REPO_ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        if remove.returncode == 0:
            removed_files.append(str(path))
        else:
            failed_files.append(
                {
                    "path": str(path),
                    "stderr": remove.stderr.strip(),
                    "stdout": remove.stdout.strip(),
                }
            )
    prune = subprocess.run(
        [
            "sudo",
            "rm",
            "-rf",
            f"/usr/share/kwin/effects/{KWIN_EFFECT_ID}",
            f"/usr/share/kwin-wayland/effects/{KWIN_EFFECT_ID}",
        ],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    return {
        "unload_stdout": unload.stdout.strip(),
        "unload_stderr": unload.stderr.strip(),
        "removed_files": removed_files,
        "failed_files": failed_files,
        "prune_stdout": prune.stdout.strip(),
        "prune_stderr": prune.stderr.strip(),
        "prune_returncode": prune.returncode,
    }


def find_system_kwin_effect_leftovers() -> list[Path]:
    roots = [
        Path("/usr/lib/qt6/plugins/kwin"),
        Path("/usr/share/kwin"),
        Path("/usr/share/kwin-wayland"),
    ]
    leftovers: list[Path] = []
    for root in roots:
        if not root.exists():
            continue
        leftovers.extend(root.glob(f"**/*{KWIN_EFFECT_ID}*"))
    return sorted(leftovers)


def process_summary(completed: subprocess.CompletedProcess[str]) -> dict[str, Any]:
    return {
        "returncode": completed.returncode,
        "stdout": completed.stdout.strip(),
        "stderr": completed.stderr.strip(),
    }


def disable_kwin_effect_config() -> subprocess.CompletedProcess[str]:
    # The smoke disables kwinrc persistence on purpose so effect loading stays
    # under explicit loadEffect control; the deploy lane enables it instead.
    return set_effect_enabled_config(False)


def _raise_for_check(result: subprocess.CompletedProcess[str]) -> subprocess.CompletedProcess[str]:
    if result.returncode != 0:
        raise subprocess.CalledProcessError(
            result.returncode, result.args, output=result.stdout, stderr=result.stderr
        )
    return result


def run_kwin_reconfigure(*, check: bool) -> subprocess.CompletedProcess[str]:
    result = _kwin_effect.run_kwin_reconfigure()
    return _raise_for_check(result) if check else result


def run_kwin_effect_command(method: str, *, check: bool) -> subprocess.CompletedProcess[str]:
    result = _kwin_effect.run_kwin_effect_command(method)
    return _raise_for_check(result) if check else result


def kwin_effect_loaded() -> bool:
    return _kwin_effect.kwin_effect_loaded()


def kwin_effect_supported() -> bool:
    return _kwin_effect.kwin_effect_supported()


def kwin_effect_discovery() -> dict[str, Any]:
    listing = run_kwin_effects_property("listOfEffects")
    effects = parse_kwin_effect_list(listing.stdout)
    loaded = run_kwin_effects_property("loadedEffects")
    loaded_effects = parse_kwin_effect_list(loaded.stdout)
    return {
        "list_returncode": listing.returncode,
        "list_stderr": listing.stderr.strip(),
        "listed": KWIN_EFFECT_ID in effects,
        "loaded_returncode": loaded.returncode,
        "loaded_stderr": loaded.stderr.strip(),
        "loaded": KWIN_EFFECT_ID in loaded_effects,
        "effect_count": len(effects),
        "loaded_effect_count": len(loaded_effects),
        "matching_effects": [effect for effect in effects if KWIN_EFFECT_ID in effect],
    }


def run_kwin_effects_property(property_name: str) -> subprocess.CompletedProcess[str]:
    return _kwin_effect.run_kwin_effects_property(property_name)


def prune_empty_parent(effect_id: str) -> None:
    for namespace in ("kwin", "kwin-wayland"):
        for path in [
            Path.home() / ".local" / "share" / namespace / "effects" / effect_id / "assets",
            Path.home() / ".local" / "share" / namespace / "effects" / effect_id / "qml",
            Path.home() / ".local" / "share" / namespace / "effects" / effect_id,
        ]:
            with contextlib.suppress(OSError):
                path.rmdir()


def write_summary(artifact_dir: Path, summary: Mapping[str, Any]) -> None:
    (artifact_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, sort_keys=True))


def read_text_if_exists(path: Path) -> str | None:
    if not path.exists():
        return None
    return path.read_text(encoding="utf-8").strip()


def read_json_if_exists(path: Path) -> Any:
    text = read_text_if_exists(path)
    if text is None:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return text


def require_commands(*names: str) -> None:
    missing = [name for name in names if shutil.which(name) is None]
    if missing:
        raise SystemExit(f"missing command(s) for this smoke: {', '.join(missing)}")


def wait_for_socket(socket_path: Path, *, deadline: float) -> None:
    while time.time() < deadline:
        if socket_path.exists():
            return
        time.sleep(0.1)
    raise RuntimeError(f"timed out waiting for service socket {socket_path}")


def service_call(
    socket_path: Path, request: Mapping[str, Any], *, timeout: float
) -> dict[str, Any]:
    encoded = json.dumps(request).encode("utf-8") + b"\n"
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(timeout)
        client.connect(str(socket_path))
        client.sendall(encoded)
        chunks: list[bytes] = []
        while True:
            chunk = client.recv(65536)
            if not chunk:
                break
            chunks.append(chunk)
            if b"\n" in chunk:
                break
    raw = b"".join(chunks).strip()
    if not raw:
        raise RuntimeError(f"empty response for request {request!r}")
    response = json.loads(raw.decode("utf-8"))
    if response.get("type") == "error":
        raise RuntimeError(json.dumps(response, indent=2, sort_keys=True))
    return response


def assert_no_host_diagnostics(*responses: Mapping[str, Any]) -> None:
    for response in responses:
        diagnostics = response.get("diagnostics")
        if not isinstance(diagnostics, list):
            continue
        host_failures = [
            entry
            for entry in diagnostics
            if isinstance(entry, dict)
            and isinstance(entry.get("code"), str)
            and entry["code"].startswith("AgentCursorHost")
        ]
        if host_failures:
            raise RuntimeError(
                "overlay host IPC reported diagnostics: "
                + json.dumps(host_failures, indent=2, sort_keys=True)
            )


def expected_overlay_backend(mode: str) -> str | None:
    if mode in {
        "layer-shell-debug-visible",
        "layer-shell-hide-for-capture",
        "layer-shell-click-through",
        "layer-shell-ydotool-click-through",
    }:
        return "wayland_layer_shell"
    if mode == "x11-debug-visible":
        return "x11_shaped_window"
    return None


def cursor_backend(response: Mapping[str, Any]) -> str | None:
    capabilities = response.get("capabilities")
    if isinstance(capabilities, Mapping):
        backend = capabilities.get("backend")
        if isinstance(backend, str):
            return backend
    return None


def require_cursor_backend_capabilities(
    *responses: Mapping[str, Any], expected_backend: str
) -> None:
    for response in responses:
        capabilities = response.get("capabilities")
        if not isinstance(capabilities, Mapping):
            raise RuntimeError(
                "agent cursor response did not include capabilities: "
                + json.dumps(response, indent=2, sort_keys=True)
            )
        expected = {
            "backend": expected_backend,
            "visible_overlay": True,
            "click_through": True,
        }
        for key, value in expected.items():
            if capabilities.get(key) != value:
                raise RuntimeError(
                    f"agent cursor capability {key!r} was {capabilities.get(key)!r}, "
                    f"expected {value!r} for mode backend {expected_backend!r}.\n"
                    f"response={json.dumps(response, indent=2, sort_keys=True)}"
                )


def require_capture(snapshot: Mapping[str, Any]) -> dict[str, Any]:
    capture = snapshot.get("capture")
    if not isinstance(capture, dict):
        raise RuntimeError("snapshot did not include capture metadata")
    screenshot_path = capture.get("screenshot_path")
    if not isinstance(screenshot_path, str) or not screenshot_path:
        raise RuntimeError("capture did not include a screenshot_path")
    if not Path(screenshot_path).exists():
        raise RuntimeError(f"capture screenshot does not exist: {screenshot_path}")
    return capture


def center_point(capture: Mapping[str, Any]) -> tuple[float, float]:
    pixel_size = capture.get("pixel_size")
    if not isinstance(pixel_size, dict):
        raise RuntimeError("capture did not include pixel_size")
    width = require_number(pixel_size, "width")
    height = require_number(pixel_size, "height")
    return (width / 2.0, height / 2.0)


def native_point_from_capture(
    capture: Mapping[str, Any], point: tuple[float, float]
) -> dict[str, Any] | None:
    pixel_size = capture.get("pixel_size")
    logical_rect = capture.get("logical_rect")
    if not isinstance(pixel_size, dict) or not isinstance(logical_rect, dict):
        return None
    pixel_width = require_number(pixel_size, "width")
    pixel_height = require_number(pixel_size, "height")
    rect_width = require_number(logical_rect, "width")
    rect_height = require_number(logical_rect, "height")
    if pixel_width <= 0 or pixel_height <= 0 or rect_width <= 0 or rect_height <= 0:
        return None
    backend = capture.get("backend")
    rect_x = 0.0 if backend == "portal_pipe_wire" else require_number(logical_rect, "x")
    rect_y = 0.0 if backend == "portal_pipe_wire" else require_number(logical_rect, "y")
    coordinate_space = logical_rect.get("space")
    if not isinstance(coordinate_space, str) or not coordinate_space:
        return None
    if backend == "portal_pipe_wire":
        coordinate_space = "stream_logical"
    return {
        "x": rect_x + ((point[0] / pixel_width) * rect_width),
        "y": rect_y + ((point[1] / pixel_height) * rect_height),
        "coordinate_space": coordinate_space,
        "mapping_id": capture.get("mapping_id"),
    }


def model_point_from_logical_capture(
    capture: Mapping[str, Any], point: Mapping[str, Any]
) -> tuple[float, float]:
    pixel_size = capture.get("pixel_size")
    logical_rect = capture.get("logical_rect")
    if not isinstance(pixel_size, dict) or not isinstance(logical_rect, dict):
        raise RuntimeError("capture did not include pixel_size and logical_rect")
    pixel_width = require_number(pixel_size, "width")
    pixel_height = require_number(pixel_size, "height")
    rect_x = require_number(logical_rect, "x")
    rect_y = require_number(logical_rect, "y")
    rect_width = require_number(logical_rect, "width")
    rect_height = require_number(logical_rect, "height")
    if pixel_width <= 0 or pixel_height <= 0 or rect_width <= 0 or rect_height <= 0:
        raise RuntimeError("capture dimensions must be positive")
    x = ((require_number(point, "x") - rect_x) / rect_width) * pixel_width
    y = ((require_number(point, "y") - rect_y) / rect_height) * pixel_height
    if x < 0.0 or y < 0.0 or x >= pixel_width or y >= pixel_height:
        raise RuntimeError(
            f"logical point {dict(point)!r} maps outside capture pixel bounds "
            f"{pixel_width}x{pixel_height}"
        )
    return (x, y)


def execute_click_request(snapshot_id: str, point: tuple[float, float]) -> dict[str, Any]:
    return {
        "type": "execute_action",
        "request": {
            "action": "click",
            "snapshot_id": snapshot_id,
            "arguments": {"x": point[0], "y": point[1]},
        },
    }


def ydotool_click(point: Mapping[str, Any]) -> dict[str, Any]:
    x = str(round(require_number(point, "x")))
    y = str(round(require_number(point, "y")))
    move = subprocess.run(
        ["ydotool", "mousemove", "--absolute", "-x", x, "-y", y],
        text=True,
        capture_output=True,
        check=False,
    )
    time.sleep(0.15)
    click = subprocess.run(
        ["ydotool", "click", "0xC0"],
        text=True,
        capture_output=True,
        check=False,
    )
    return {
        "success": move.returncode == 0 and click.returncode == 0,
        "target_logical_point": {"x": float(x), "y": float(y)},
        "mousemove": {
            "returncode": move.returncode,
            "stdout": move.stdout.strip(),
            "stderr": move.stderr.strip(),
        },
        "click": {
            "returncode": click.returncode,
            "stdout": click.stdout.strip(),
            "stderr": click.stderr.strip(),
        },
    }


def require_str(mapping: Mapping[str, Any], key: str) -> str:
    value = mapping.get(key)
    if not isinstance(value, str) or not value:
        raise RuntimeError(f"expected string field {key}")
    return value


def require_number(mapping: Mapping[str, Any], key: str) -> float:
    value = mapping.get(key)
    if not isinstance(value, int | float):
        raise RuntimeError(f"expected numeric field {key}")
    return float(value)


def agent_cursor_source_path(after_path: Path) -> Path:
    name = after_path.name
    marker = ".agent-cursor."
    if marker not in name:
        raise RuntimeError(f"synthetic screenshot path did not include {marker!r}: {after_path}")
    before_path = after_path.with_name(name.replace(marker, ".", 1))
    if not before_path.exists():
        raise RuntimeError(f"source screenshot for synthetic marker is missing: {before_path}")
    return before_path


def probe_marker(before_path: Path, after_path: Path, point: tuple[float, float]) -> MarkerProbe:
    before = Image.open(before_path).convert("RGB")
    after = Image.open(after_path).convert("RGB")
    if before.size != after.size:
        raise RuntimeError(f"screenshot size changed from {before.size} to {after.size}")
    hotspot_x = round(point[0])
    hotspot_y = round(point[1])
    left = max(0, hotspot_x - CURSOR_ASSET_HOTSPOT_X)
    top = max(0, hotspot_y - CURSOR_ASSET_HOTSPOT_Y)
    right = min(after.width, left + CURSOR_ASSET_WIDTH)
    bottom = min(after.height, top + CURSOR_ASSET_HEIGHT)
    before_crop = before.crop((left, top, right, bottom))
    after_crop = after.crop((left, top, right, bottom))
    diff = ImageChops.difference(before_crop, after_crop)
    # Per-pixel max channel delta using lighter(r, g) then lighter(rg, b)
    diff_r, diff_g, diff_b = diff.split()
    max_rg = ImageChops.lighter(diff_r, diff_g)
    max_rgb = ImageChops.lighter(max_rg, diff_b)
    # Count pixels with max delta >= 40 using histogram of thresholded image
    threshold_map = [0] * 40 + [255] * 216
    thresholded = max_rgb.point(threshold_map, mode="1")
    hist = thresholded.histogram()
    changed = hist[-1] if len(hist) >= 2 else 0
    # Find max delta from histogram (avoids imprecise getextrema type stubs)
    max_delta = 0
    for value, count in enumerate(max_rgb.histogram()[::-1], start=0):
        if count > 0:
            max_delta = 255 - value
            break
    return MarkerProbe(
        found=changed >= 24 and max_delta >= 40,
        changed_pixels_near_hotspot=changed,
        max_channel_delta_near_hotspot=max_delta,
        checked_box=(left, top, right, bottom),
    )


def capture_until_marker(
    socket_path: Path,
    before_path: Path,
    point: tuple[float, float],
    *,
    request_timeout: float,
    deadline: float,
) -> tuple[dict[str, Any], dict[str, Any], MarkerProbe]:
    while True:
        visible = service_call(socket_path, {"type": "get_app_state"}, timeout=request_timeout)
        visible_snapshot = visible["snapshot"]
        visible_capture = require_capture(visible_snapshot)
        visible_path = Path(require_str(visible_capture, "screenshot_path"))
        probe = probe_marker(before_path, visible_path, point)
        if probe.found or time.time() >= deadline:
            return visible, visible_capture, probe
        time.sleep(0.2)


def convert_kwin_raw_screenshot(artifact_dir: Path) -> Path:
    grim_path = artifact_dir / "nested-kwin-effect-capture.png"
    if grim_path.exists():
        return grim_path
    attrs_path = artifact_dir / "nested-screenshot-attrs.json"
    raw_path = artifact_dir / "nested-screenshot.raw"
    if not attrs_path.exists() or not raw_path.exists():
        raise RuntimeError("nested KWin screenshot artifacts are missing")
    attrs = json.loads(attrs_path.read_text(encoding="utf-8"))
    width = int(attrs["width"])
    height = int(attrs["height"])
    stride = int(attrs["stride"])
    image_format = int(attrs["format"])
    if image_format != 6:
        raise RuntimeError(f"unsupported KWin raw screenshot QImage format: {image_format}")
    raw = raw_path.read_bytes()
    expected_minimum = stride * height
    if len(raw) < expected_minimum:
        raise RuntimeError(
            f"raw screenshot was {len(raw)} bytes, expected at least {expected_minimum}"
        )
    tightly_packed = b"".join(raw[y * stride : y * stride + width * 4] for y in range(height))
    image = Image.frombytes("RGBA", (width, height), tightly_packed, "raw", "BGRA")
    output_path = artifact_dir / "nested-kwin-effect-capture.png"
    image.save(output_path)
    return output_path


def probe_cursor_asset_presence(image_path: Path, point: tuple[float, float]) -> MarkerProbe:
    screenshot = Image.open(image_path).convert("RGBA")
    asset = (
        Image.open(KWIN_EFFECT_CURSOR_ASSET)
        .convert("RGBA")
        .resize((CURSOR_ASSET_WIDTH, CURSOR_ASSET_HEIGHT), Image.Resampling.LANCZOS)
    )
    hotspot_x = round(point[0])
    hotspot_y = round(point[1])
    left = max(0, hotspot_x - CURSOR_ASSET_HOTSPOT_X)
    top = max(0, hotspot_y - CURSOR_ASSET_HOTSPOT_Y)
    right = min(screenshot.width, left + CURSOR_ASSET_WIDTH)
    bottom = min(screenshot.height, top + CURSOR_ASSET_HEIGHT)
    matched = 0
    max_channel = 0
    for y in range(top, bottom):
        for x in range(left, right):
            asset_pixel = asset.getpixel((x - left, y - top))
            if not isinstance(asset_pixel, tuple) or len(asset_pixel) < 4 or asset_pixel[3] < 24:
                continue
            screen_pixel = screenshot.getpixel((x, y))
            if not isinstance(screen_pixel, tuple) or len(screen_pixel) < 3:
                continue
            channel = max(int(screen_pixel[0]), int(screen_pixel[1]), int(screen_pixel[2]))
            max_channel = max(max_channel, channel)
            if channel >= 24:
                matched += 1
    return MarkerProbe(
        found=matched >= 48 and max_channel >= 40,
        changed_pixels_near_hotspot=matched,
        max_channel_delta_near_hotspot=max_channel,
        checked_box=(left, top, right, bottom),
    )


def rgb_pixel(image: Image.Image, x: int, y: int) -> tuple[int, int, int]:
    pixel = image.getpixel((x, y))
    if not isinstance(pixel, tuple) or len(pixel) < 3:
        raise RuntimeError(f"expected RGB pixel at {(x, y)}")
    return (int(pixel[0]), int(pixel[1]), int(pixel[2]))


def copy_artifact(path: Path, artifact_dir: Path, stem: str) -> Path:
    destination = artifact_dir / f"{stem}{path.suffix}"
    shutil.copy2(path, destination)
    return destination


def start_pointer_fixture(state_path: Path) -> subprocess.Popen[str]:
    env = dict(os.environ)
    env["SKY_CUA_POINTER_FULLSCREEN"] = "1"
    python = gtk_fixture_python()
    stdout = (state_path.parent / "pointer.stdout.log").open("w", encoding="utf-8")
    stderr = (state_path.parent / "pointer.stderr.log").open("w", encoding="utf-8")
    return subprocess.Popen(
        [python, str(POINTER_FIXTURE), str(state_path)],
        stdout=stdout,
        stderr=stderr,
        text=True,
        cwd=REPO_ROOT,
        env=env,
    )


def gtk_fixture_python() -> str:
    for candidate in ("/usr/bin/python3", shutil.which("python3"), sys.executable):
        if candidate:
            return candidate
    return sys.executable


def load_pointer_state(state_path: Path) -> dict[str, Any] | None:
    if not state_path.exists():
        return None
    try:
        state = json.loads(state_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None
    return state if isinstance(state, dict) else None


def wait_for_stable_pointer_fixture(state_path: Path, *, deadline: float) -> dict[str, Any]:
    candidate: dict[str, Any] | None = None
    while time.time() < deadline:
        state = load_pointer_state(state_path)
        if state is None:
            time.sleep(0.15)
            continue
        width = int(state.get("window_size", {}).get("width", 0) or 0)
        height = int(state.get("window_size", {}).get("height", 0) or 0)
        points = state.get("points")
        if (
            not state.get("ready")
            or width < 1000
            or height < 700
            or not isinstance(points, dict)
            or "click_button" not in points
        ):
            time.sleep(0.15)
            continue
        if candidate is None:
            candidate = state
            time.sleep(0.35)
            continue
        if candidate.get("window_size") == state.get("window_size") and candidate.get(
            "points"
        ) == state.get("points"):
            return state
        candidate = state
        time.sleep(0.35)
    raise RuntimeError("timed out waiting for stable fullscreen pointer-fixture geometry")


def fixture_point(state: Mapping[str, Any], name: str) -> dict[str, Any]:
    points = state.get("points")
    if not isinstance(points, Mapping):
        raise RuntimeError("pointer fixture state did not include points")
    point = points.get(name)
    if not isinstance(point, Mapping):
        raise RuntimeError(f"pointer fixture did not include point {name!r}")
    return {"x": require_number(point, "x"), "y": require_number(point, "y")}


def wait_for_pointer_click(state_path: Path, *, deadline: float) -> dict[str, Any] | None:
    while time.time() < deadline:
        state = load_pointer_state(state_path)
        if state is not None and state.get("clicked") is True:
            return state
        time.sleep(0.15)
    return None


def terminate_process(proc: subprocess.Popen[Any], *, name: str) -> None:
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def terminate_service(proc: subprocess.Popen[bytes]) -> None:
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
