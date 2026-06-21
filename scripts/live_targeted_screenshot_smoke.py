#!/usr/bin/env python3
"""Live targeted screenshot smoke for the real desktop session."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import tempfile
import time
from collections.abc import Mapping
from contextlib import suppress
from pathlib import Path
from typing import Any

from PIL import Image

from _mcp_stdio import stop_service_processes_for_socket
from live_desktop_smoke import (  # type: ignore[import-not-found]
    CLIENT,
    McpClient,
    require_ok,
    run_zenity_input,
    wait_for_app_snapshot,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
TARGET_TITLE = "sky-cua targeted screenshot target"
OCCLUDER_TITLE = "sky-cua targeted screenshot occluder"


def require_real_graphical_session() -> str:
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR", "").strip()
    wayland_display = os.environ.get("WAYLAND_DISPLAY", "").strip()
    if runtime_dir and wayland_display:
        socket_path = Path(runtime_dir) / wayland_display
        if socket_path.is_socket():
            return "wayland"
    if os.environ.get("DISPLAY", "").strip():
        return "x11"
    raise RuntimeError(
        "XDG_RUNTIME_DIR/WAYLAND_DISPLAY or DISPLAY must point at the real graphical session"
    )


def require_real_wayland_session() -> None:
    if require_real_graphical_session() != "wayland":
        raise RuntimeError("XDG_RUNTIME_DIR and WAYLAND_DISPLAY must point at the real session")


def gtk_session_env(session_backend: str) -> dict[str, str]:
    if session_backend == "wayland":
        return {"GDK_BACKEND": "wayland", "DISPLAY": ""}
    if session_backend == "x11":
        return {"GDK_BACKEND": "x11", "WAYLAND_DISPLAY": ""}
    raise RuntimeError(f"unsupported graphical session backend: {session_backend}")


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True), encoding="utf-8")


def diagnostic_codes(result: Mapping[str, Any]) -> set[str]:
    structured = result.get("structuredContent")
    if not isinstance(structured, Mapping):
        return set()
    diagnostics = structured.get("diagnostics")
    if not isinstance(diagnostics, list):
        return set()
    return {
        code
        for entry in diagnostics
        if isinstance(entry, Mapping) and isinstance(code := entry.get("code"), str)
    }


def require_doctor_display_topology(result: Mapping[str, Any]) -> Mapping[str, Any]:
    structured = result.get("structuredContent")
    if not isinstance(structured, Mapping):
        raise RuntimeError("doctor did not return structuredContent")
    topology = structured.get("display_topology")
    if not isinstance(topology, Mapping):
        raise RuntimeError(
            "doctor did not include display_topology; screenshot smoke cannot prove display discovery diagnostics"
        )
    display_count = topology.get("display_count")
    if not isinstance(display_count, int):
        raise RuntimeError(f"doctor display_topology missing display_count: {topology!r}")
    if display_count == 0:
        probes = topology.get("probes")
        if not isinstance(probes, list) or not probes:
            raise RuntimeError(
                "doctor display_topology reported no displays without provider probe diagnostics"
            )
    return topology


def require_number(mapping: Mapping[str, Any], key: str) -> float:
    value = mapping.get(key)
    if not isinstance(value, int | float):
        raise RuntimeError(f"expected numeric field {key}")
    return float(value)


def require_mapping(mapping: Mapping[str, Any], key: str) -> Mapping[str, Any]:
    value = mapping.get(key)
    if not isinstance(value, Mapping):
        raise RuntimeError(f"expected object field {key}")
    return value


def require_capture(snapshot: Mapping[str, Any]) -> Mapping[str, Any]:
    capture = snapshot.get("capture")
    if not isinstance(capture, Mapping):
        raise RuntimeError("screenshot did not include capture metadata")
    image_path = capture.get("inspection_image_path")
    if not isinstance(image_path, str) or not image_path:
        raise RuntimeError("screenshot capture did not include inspection_image_path")
    if not Path(image_path).exists():
        raise RuntimeError(f"screenshot path does not exist: {image_path}")
    return capture


def require_crop_metadata(
    capture: Mapping[str, Any],
    reference_capture: Mapping[str, Any],
) -> None:
    pixel_size = require_mapping(capture, "pixel_size")
    reference_pixel_size = require_mapping(reference_capture, "pixel_size")
    width = require_number(pixel_size, "width")
    height = require_number(pixel_size, "height")
    reference_width = require_number(reference_pixel_size, "width")
    reference_height = require_number(reference_pixel_size, "height")
    if width <= 0 or height <= 0 or reference_width <= 0 or reference_height <= 0:
        raise RuntimeError(f"targeted screenshot dimensions must be positive: {capture!r}")
    if width >= reference_width and height >= reference_height:
        raise RuntimeError(
            "targeted screenshot was not cropped smaller than an untargeted screenshot.\n"
            f"targeted={json.dumps(capture, indent=2, sort_keys=True)}\n"
            f"untargeted={json.dumps(reference_capture, indent=2, sort_keys=True)}"
        )
    logical_rect = require_mapping(capture, "logical_rect")
    if require_number(logical_rect, "width") <= 0 or require_number(logical_rect, "height") <= 0:
        raise RuntimeError(f"targeted screenshot logical_rect is not usable: {logical_rect!r}")


def require_screenshot_file_matches_capture(capture: Mapping[str, Any]) -> None:
    image_path = capture["inspection_image_path"]
    if not isinstance(image_path, str):
        raise RuntimeError("inspection_image_path is not a string")
    pixel_size = require_mapping(capture, "pixel_size")
    expected = (int(require_number(pixel_size, "width")), int(require_number(pixel_size, "height")))
    with Image.open(image_path) as image:
        actual = image.size
    if actual != expected:
        raise RuntimeError(
            f"targeted screenshot image dimensions {actual} did not match capture pixel_size {expected}"
        )


def zenity_ok_button_point(capture: Mapping[str, Any]) -> dict[str, float]:
    pixel_size = require_mapping(capture, "pixel_size")
    pixel_width = require_number(pixel_size, "width")
    pixel_height = require_number(pixel_size, "height")
    if pixel_width <= 0 or pixel_height <= 0:
        raise RuntimeError(f"targeted screenshot pixel_size is not usable: {pixel_size!r}")
    return {"x": pixel_width * 0.76, "y": pixel_height * 0.89}


def find_window_by_title_and_pid(
    client: McpClient,
    title: str,
    pid: int,
    artifact_dir: Path,
    *,
    request_id: int,
) -> Mapping[str, Any]:
    result = client.tools_call(request_id, "list_windows", {})
    write_json(artifact_dir / f"windows-{request_id}.json", result)
    windows = (result.get("structuredContent") or {}).get("windows") or []
    title_matches = [w for w in windows if isinstance(w, Mapping) and w.get("title") == title]
    window = next(
        (w for w in title_matches if w.get("pid") == pid),
        title_matches[0] if len(title_matches) == 1 else None,
    )
    if window is None:
        raise RuntimeError(
            f"did not find unique window title={title!r} pid={pid}.\n"
            f"windows={json.dumps(windows, indent=2, sort_keys=True)}"
        )
    if not isinstance(window.get("window_id"), str):
        raise RuntimeError(f"window did not expose window_id: {window!r}")
    return window


def terminate_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()


def terminate_processes_for_temp_socket(service_socket_path: Path) -> None:
    stop_service_processes_for_socket(service_socket_path)
    overlay_socket_path = service_socket_path.parent / "agent-cursor.sock"
    for proc_dir in Path("/proc").iterdir():
        if not proc_dir.name.isdecimal():
            continue
        pid = int(proc_dir.name)
        if pid == os.getpid():
            continue
        try:
            cmdline = (proc_dir / "cmdline").read_bytes()
        except OSError:
            continue
        if b"sky-cua-overlay-host" in cmdline and str(overlay_socket_path).encode() in cmdline:
            with suppress(ProcessLookupError):
                os.kill(pid, signal.SIGTERM)


def main() -> int:
    session_backend = require_real_graphical_session()
    gtk_env = gtk_session_env(session_backend)

    artifact_root = REPO_ROOT / "artifacts" / "gui-desktop-smoke" / "targeted-screenshot"
    artifact_dir = artifact_root / time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    artifact_dir.mkdir(parents=True, exist_ok=True)

    client_env = {
        "SKY_CUA_AGENT_CURSOR": "0",
        "SKY_CUA_SCREENSHOT_CURSOR": "0",
    }
    client_env.update(gtk_env)

    with tempfile.TemporaryDirectory(prefix="sky-cua-targeted-screenshot-") as tmpdir:
        service_socket_path = Path(tmpdir) / "service.sock"
        client_env["SKY_CUA_SERVICE_SOCKET_PATH"] = str(service_socket_path)
        target_dialog = run_zenity_input(
            TARGET_TITLE,
            initial_text="targeted-screenshot-ok",
            extra_env=gtk_env,
        )
        occluder: subprocess.Popen[str] | None = None
        try:
            client = McpClient([str(CLIENT), "mcp"], extra_env=client_env)
            try:
                client.initialize()
                tools = {tool["name"] for tool in client.tools_list()}
                missing = {
                    "click",
                    "focused_window",
                    "get_app_state",
                    "list_windows",
                    "screenshot",
                } - tools
                if missing:
                    raise RuntimeError(
                        f"MCP server did not advertise required tools: {sorted(missing)}"
                    )

                doctor_result = client.tools_call(19, "doctor", {})
                write_json(artifact_dir / "doctor-display-topology.json", doctor_result)
                require_ok(doctor_result, "doctor display topology")
                require_doctor_display_topology(doctor_result)

                app_snapshot = wait_for_app_snapshot(
                    client, TARGET_TITLE, deadline=time.time() + 30
                )
                write_json(artifact_dir / "target-app-state.json", app_snapshot)

                target_window = find_window_by_title_and_pid(
                    client,
                    TARGET_TITLE,
                    target_dialog.pid,
                    artifact_dir,
                    request_id=20,
                )

                occluder = run_zenity_input(
                    OCCLUDER_TITLE,
                    initial_text="screenshot target should raise the target dialog",
                    extra_env=gtk_env,
                )
                time.sleep(0.8)

                untargeted_result = client.tools_call(21, "screenshot", {})
                write_json(artifact_dir / "untargeted-screenshot-result.json", untargeted_result)
                require_ok(untargeted_result, "untargeted screenshot reference")
                untargeted_snapshot = untargeted_result.get("structuredContent")
                if not isinstance(untargeted_snapshot, Mapping):
                    raise RuntimeError("untargeted screenshot did not return structuredContent")
                reference_capture = require_capture(untargeted_snapshot)

                target = {"window_id": target_window["window_id"]}
                screenshot_result = client.tools_call(22, "screenshot", target)
                write_json(artifact_dir / "targeted-screenshot-result.json", screenshot_result)
                require_ok(screenshot_result, "targeted screenshot")
                snapshot = screenshot_result.get("structuredContent")
                if not isinstance(snapshot, Mapping):
                    raise RuntimeError("targeted screenshot did not return structuredContent")
                snapshot_id = snapshot.get("snapshot_id")
                if not isinstance(snapshot_id, str) or not snapshot_id:
                    raise RuntimeError(
                        f"targeted screenshot did not return snapshot_id: {snapshot!r}"
                    )

                if target_window.get(
                    "backend"
                ) == "kwin" and "WindowFocusVerified" not in diagnostic_codes(screenshot_result):
                    raise RuntimeError(
                        "KWin targeted screenshot did not report WindowFocusVerified.\n"
                        f"result={json.dumps(screenshot_result, indent=2, sort_keys=True)}"
                    )

                focused_result = client.tools_call(23, "focused_window", {})
                write_json(artifact_dir / "focused-window-after-screenshot.json", focused_result)
                focused_window = (focused_result.get("structuredContent") or {}).get("window")
                if (
                    not isinstance(focused_window, Mapping)
                    or focused_window.get("title") != TARGET_TITLE
                ):
                    raise RuntimeError(
                        "targeted screenshot did not leave the target dialog focused.\n"
                        f"focused={json.dumps(focused_window, indent=2, sort_keys=True)}"
                    )

                capture = require_capture(snapshot)
                require_crop_metadata(capture, reference_capture)
                require_screenshot_file_matches_capture(capture)
                write_json(artifact_dir / "targeted-capture.json", capture)

                screenshot_point = zenity_ok_button_point(capture)
                write_json(artifact_dir / "targeted-click-point.json", screenshot_point)
                click_result = client.tools_call(
                    24,
                    "click",
                    {"snapshot_id": snapshot_id, **screenshot_point},
                )
                write_json(artifact_dir / "targeted-click-result.json", click_result)
                require_ok(click_result, "targeted screenshot coordinate click")

                stdout, stderr = target_dialog.communicate(timeout=8)
                if target_dialog.returncode != 0:
                    raise RuntimeError(
                        f"target dialog exited with {target_dialog.returncode}\n"
                        f"stdout={stdout!r}\nstderr={stderr!r}"
                    )
                if stdout.strip() != "targeted-screenshot-ok":
                    raise RuntimeError(
                        f"expected target dialog to submit targeted-screenshot-ok, got {stdout.strip()!r}"
                    )
                write_json(
                    artifact_dir / "target-dialog-exit.json",
                    {"returncode": target_dialog.returncode, "stdout": stdout.strip()},
                )
            finally:
                client.close()
                terminate_processes_for_temp_socket(service_socket_path)
        finally:
            if occluder is not None:
                terminate_process(occluder)
                stderr = occluder.stderr.read() if occluder.stderr is not None else ""
                if stderr.strip():
                    (artifact_dir / "occluder.stderr.log").write_text(stderr, encoding="utf-8")
            terminate_process(target_dialog)
            if target_dialog.stderr is not None and target_dialog.poll() is not None:
                stderr = ""
                with suppress(ValueError):
                    stderr = target_dialog.stderr.read()
                if target_dialog.stderr.closed:
                    stderr = ""
                if stderr.strip():
                    (artifact_dir / "target.stderr.log").write_text(stderr, encoding="utf-8")

    print(f"Targeted screenshot smoke completed successfully; artifacts: {artifact_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
