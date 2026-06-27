#!/usr/bin/env python3
"""Live display-targeted screenshot smoke for the real desktop session."""

from __future__ import annotations

import json
import os
import signal
import tempfile
import time
from collections.abc import Mapping, Sequence
from contextlib import suppress
from pathlib import Path
from typing import Any

from _smoke_config import env_flag
from live_desktop_smoke import CLIENT, McpClient, require_ok, run_zenity_input
from live_targeted_screenshot_smoke import (
    gtk_session_env,
    require_capture,
    require_doctor_display_topology,
    require_mapping,
    require_number,
    require_real_graphical_session,
    require_screenshot_file_matches_capture,
    terminate_process,
    terminate_processes_for_temp_socket,
    write_json,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
TARGET_TITLE = "sky-cua display screenshot target"
STRICT_SECONDARY_DISPLAY_ENV = "SKY_CUA_DISPLAY_SCREENSHOT_REQUIRE_SECONDARY"


def require_capture_scope(capture: Mapping[str, Any], expected: str, label: str) -> None:
    actual = capture.get("capture_scope")
    if actual != expected:
        raise RuntimeError(f"{label} capture_scope={actual!r}, expected {expected!r}")


def require_display_id(capture: Mapping[str, Any], expected: str, label: str) -> None:
    display = require_mapping(capture, "display")
    actual = display.get("display_id")
    if actual != expected:
        raise RuntimeError(f"{label} display_id={actual!r}, expected {expected!r}")


def grouped_structured_result(result: Mapping[str, Any]) -> Mapping[str, Any]:
    structured = result.get("structuredContent") or {}
    if not isinstance(structured, Mapping):
        return {}
    nested = structured.get("result")
    return nested if isinstance(nested, Mapping) else structured


def require_positive_capture(capture: Mapping[str, Any], label: str) -> None:
    pixel_size = require_mapping(capture, "pixel_size")
    logical_rect = require_mapping(capture, "logical_rect")
    if require_number(pixel_size, "width") <= 0 or require_number(pixel_size, "height") <= 0:
        raise RuntimeError(f"{label} pixel_size is not positive: {pixel_size!r}")
    if require_number(logical_rect, "width") <= 0 or require_number(logical_rect, "height") <= 0:
        raise RuntimeError(f"{label} logical_rect is not positive: {logical_rect!r}")
    require_screenshot_file_matches_capture(capture)


def require_displays(snapshot: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    environment = require_mapping(snapshot, "environment")
    displays = environment.get("displays")
    if not isinstance(displays, list) or not displays:
        raise RuntimeError(
            "display screenshot smoke requires environment.displays from the screenshot result"
        )
    return [display for display in displays if isinstance(display, Mapping)]


def require_windows(snapshot: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    windows = snapshot.get("windows")
    if not isinstance(windows, list) or not windows:
        raise RuntimeError(
            "display screenshot smoke requires windows from list_resources desktop/windows"
        )
    return [window for window in windows if isinstance(window, Mapping)]


def find_window_by_title_and_pid(
    windows: Sequence[Mapping[str, Any]], title: str, pid: int
) -> Mapping[str, Any]:
    title_matches = [window for window in windows if window.get("title") == title]
    window = next(
        (candidate for candidate in title_matches if candidate.get("pid") == pid),
        title_matches[0] if len(title_matches) == 1 else None,
    )
    if window is None:
        raise RuntimeError(
            f"did not find unique window title={title!r} pid={pid}.\n"
            f"windows={json.dumps(list(windows), indent=2, sort_keys=True)}"
        )
    if not isinstance(window.get("window_id"), str):
        raise RuntimeError(f"window did not expose window_id: {window!r}")
    return window


def primary_display(displays: Sequence[Mapping[str, Any]]) -> Mapping[str, Any]:
    return next(
        (
            display
            for display in displays
            if isinstance(display.get("primary"), bool) and display["primary"]
        ),
        displays[0],
    )


def display_id(display: Mapping[str, Any]) -> str:
    value = display.get("display_id")
    if not isinstance(value, str) or not value:
        raise RuntimeError(f"display did not include display_id: {display!r}")
    return value


def bounds_contains_point(bounds: Mapping[str, Any], x: float, y: float) -> bool:
    return (
        x >= require_number(bounds, "x")
        and y >= require_number(bounds, "y")
        and x <= require_number(bounds, "x") + require_number(bounds, "width")
        and y <= require_number(bounds, "y") + require_number(bounds, "height")
    )


def window_button_point(window: Mapping[str, Any]) -> tuple[float, float]:
    bounds = require_mapping(window, "bounds")
    return (
        require_number(bounds, "x") + require_number(bounds, "width") * 0.76,
        require_number(bounds, "y") + require_number(bounds, "height") * 0.89,
    )


def screenshot_point_for_desktop_point(
    capture: Mapping[str, Any], point: tuple[float, float]
) -> dict[str, float]:
    logical_rect = require_mapping(capture, "logical_rect")
    pixel_size = require_mapping(capture, "pixel_size")
    x = point[0]
    y = point[1]
    if not bounds_contains_point(logical_rect, x, y):
        raise RuntimeError(
            "target point is outside display screenshot logical_rect.\n"
            f"point={point!r}\nlogical_rect={json.dumps(logical_rect, indent=2, sort_keys=True)}"
        )
    rel_x = (x - require_number(logical_rect, "x")) / require_number(logical_rect, "width")
    rel_y = (y - require_number(logical_rect, "y")) / require_number(logical_rect, "height")
    return {
        "x": rel_x * require_number(pixel_size, "width"),
        "y": rel_y * require_number(pixel_size, "height"),
    }


def main() -> int:
    session_backend = require_real_graphical_session()
    gtk_env = gtk_session_env(session_backend)
    require_secondary_display = env_flag(STRICT_SECONDARY_DISPLAY_ENV)

    artifact_root = REPO_ROOT / "artifacts" / "gui-desktop-smoke" / "display-screenshot"
    artifact_dir = artifact_root / time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    artifact_dir.mkdir(parents=True, exist_ok=True)

    client_env = {
        "SKY_CUA_AGENT_CURSOR": "0",
        "SKY_CUA_SCREENSHOT_CURSOR": "0",
    }
    client_env.update(gtk_env)

    with tempfile.TemporaryDirectory(prefix="sky-cua-display-screenshot-") as tmpdir:
        service_socket_path = Path(tmpdir) / "service.sock"
        client_env["SKY_CUA_SERVICE_SOCKET_PATH"] = str(service_socket_path)
        target_dialog = run_zenity_input(
            TARGET_TITLE,
            initial_text="display-screenshot-ok",
            extra_env=gtk_env,
        )
        try:
            client = McpClient([str(CLIENT), "mcp"], extra_env=client_env)
            try:
                client.initialize()
                tools = {tool["name"] for tool in client.tools_list()}
                missing = {"capture_desktop", "desktop_pointer", "list_resources"} - tools
                if missing:
                    raise RuntimeError(
                        f"MCP server did not advertise required tools: {sorted(missing)}"
                    )

                doctor_result = client.tools_call(19, "doctor", {})
                write_json(artifact_dir / "doctor-display-topology.json", doctor_result)
                require_ok(doctor_result, "doctor display topology")
                require_doctor_display_topology(doctor_result)

                time.sleep(0.8)
                windows_result = client.tools_call(
                    20, "list_resources", {"surface": "desktop", "resource": "windows"}
                )
                write_json(artifact_dir / "windows-20.json", windows_result)
                require_ok(windows_result, "list_resources desktop/windows")
                windows_snapshot = grouped_structured_result(windows_result)
                if not isinstance(windows_snapshot, Mapping):
                    raise RuntimeError("desktop/windows list did not return structuredContent")
                displays = require_displays(windows_snapshot)
                windows = require_windows(windows_snapshot)
                target_window = find_window_by_title_and_pid(
                    windows, TARGET_TITLE, target_dialog.pid
                )
                write_json(artifact_dir / "displays.json", {"displays": displays})

                default_result = client.tools_call(21, "capture_desktop", {})
                write_json(artifact_dir / "default-primary-screenshot-result.json", default_result)
                require_ok(default_result, "default primary screenshot")
                default_snapshot = grouped_structured_result(default_result)
                if not isinstance(default_snapshot, Mapping):
                    raise RuntimeError("default screenshot did not return structuredContent")
                primary = primary_display(displays)
                primary_id = display_id(primary)
                default_capture = require_capture(default_snapshot)
                require_capture_scope(default_capture, "primary_display", "default screenshot")
                require_display_id(default_capture, primary_id, "default screenshot")
                require_positive_capture(default_capture, "default screenshot")

                primary_result = client.tools_call(
                    22, "capture_desktop", {"display_id": primary_id}
                )
                write_json(artifact_dir / "explicit-primary-screenshot-result.json", primary_result)
                require_ok(primary_result, "explicit primary display screenshot")
                primary_snapshot = grouped_structured_result(primary_result)
                if not isinstance(primary_snapshot, Mapping):
                    raise RuntimeError(
                        "explicit primary screenshot did not return structuredContent"
                    )
                primary_capture = require_capture(primary_snapshot)
                require_capture_scope(primary_capture, "display", "explicit primary screenshot")
                require_display_id(primary_capture, primary_id, "explicit primary screenshot")
                require_positive_capture(primary_capture, "explicit primary screenshot")

                secondary = next(
                    (display for display in displays if display_id(display) != primary_id),
                    None,
                )
                if secondary is None:
                    skip_reason = "VM session exposed only one display; explicit secondary display capture not applicable"
                    write_json(
                        artifact_dir / "secondary-display-skip.json",
                        {
                            "skipped": True,
                            "reason": skip_reason,
                            "display_count": len(displays),
                        },
                    )
                    if require_secondary_display:
                        raise RuntimeError(
                            f"strict display screenshot smoke requires at least two displays: {skip_reason}"
                        )
                else:
                    secondary_id = display_id(secondary)
                    secondary_result = client.tools_call(
                        23, "capture_desktop", {"display_id": secondary_id}
                    )
                    write_json(
                        artifact_dir / "explicit-secondary-screenshot-result.json",
                        secondary_result,
                    )
                    require_ok(secondary_result, "explicit secondary display screenshot")
                    secondary_snapshot = grouped_structured_result(secondary_result)
                    if not isinstance(secondary_snapshot, Mapping):
                        raise RuntimeError(
                            "explicit secondary screenshot did not return structuredContent"
                        )
                    secondary_capture = require_capture(secondary_snapshot)
                    require_capture_scope(
                        secondary_capture, "display", "explicit secondary screenshot"
                    )
                    require_display_id(
                        secondary_capture, secondary_id, "explicit secondary screenshot"
                    )
                    require_positive_capture(secondary_capture, "explicit secondary screenshot")

                rejected_all_result = client.tools_call(
                    24, "capture_desktop", {"capture_all_displays": True}
                )
                write_json(artifact_dir / "all-displays-rejection-result.json", rejected_all_result)
                if not rejected_all_result.get("isError"):
                    raise RuntimeError(
                        "capture_desktop must reject capture_all_displays so the agent stays on a "
                        f"single screen, got: {rejected_all_result!r}"
                    )

                target_display_id = primary_id
                target_display = target_window.get("display")
                if isinstance(target_display, Mapping) and isinstance(
                    target_display.get("display_id"), str
                ):
                    target_display_id = target_display["display_id"]
                target_result = client.tools_call(
                    25, "capture_desktop", {"display_id": target_display_id}
                )
                write_json(artifact_dir / "target-display-screenshot-result.json", target_result)
                require_ok(target_result, "target display screenshot")
                target_snapshot = grouped_structured_result(target_result)
                if not isinstance(target_snapshot, Mapping):
                    raise RuntimeError("target display screenshot did not return structuredContent")
                target_snapshot_id = target_snapshot.get("snapshot_id")
                if not isinstance(target_snapshot_id, str) or not target_snapshot_id:
                    raise RuntimeError(
                        f"target display screenshot did not return snapshot_id: {target_snapshot!r}"
                    )
                target_capture = require_capture(target_snapshot)
                require_capture_scope(target_capture, "display", "target display screenshot")
                require_display_id(target_capture, target_display_id, "target display screenshot")
                require_positive_capture(target_capture, "target display screenshot")

                click_point = screenshot_point_for_desktop_point(
                    target_capture,
                    window_button_point(target_window),
                )
                write_json(artifact_dir / "target-display-click-point.json", click_point)
                click_result = client.tools_call(
                    26,
                    "desktop_pointer",
                    {"operation": "click", "snapshot_id": target_snapshot_id, **click_point},
                )
                write_json(artifact_dir / "target-display-click-result.json", click_result)
                require_ok(click_result, "display screenshot coordinate click")

                stdout, stderr = target_dialog.communicate(timeout=8)
                if target_dialog.returncode != 0:
                    raise RuntimeError(
                        f"target dialog exited with {target_dialog.returncode}\n"
                        f"stdout={stdout!r}\nstderr={stderr!r}"
                    )
                if stdout.strip() != "display-screenshot-ok":
                    raise RuntimeError(
                        f"expected target dialog to submit display-screenshot-ok, got {stdout.strip()!r}"
                    )
                write_json(
                    artifact_dir / "target-dialog-exit.json",
                    {"returncode": target_dialog.returncode, "stdout": stdout.strip()},
                )
            finally:
                client.close()
                terminate_processes_for_temp_socket(service_socket_path)
        finally:
            terminate_process(target_dialog)
            if target_dialog.stderr is not None and target_dialog.poll() is not None:
                stderr = ""
                with suppress(ValueError):
                    stderr = target_dialog.stderr.read()
                if target_dialog.stderr.closed:
                    stderr = ""
                if stderr.strip():
                    (artifact_dir / "target.stderr.log").write_text(stderr, encoding="utf-8")
            with suppress(ProcessLookupError):
                os.kill(target_dialog.pid, signal.SIGTERM)

    print(f"Display screenshot smoke completed successfully; artifacts: {artifact_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
