#!/usr/bin/env python3
"""Visible Wayland pointer smoke for the real desktop session.

This runs the fullscreen GTK pointer fixture on the current Wayland display and
drives click, drag, and scroll actions through the Computer Use MCP server. It
does not start Xvfb or any nested display server.
"""

from __future__ import annotations

import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time
from contextlib import suppress
from pathlib import Path
from typing import Any

from live_desktop_smoke import (  # type: ignore[import-not-found]
    CLIENT,
    McpClient,
    load_state,
    require_ok,
    run_pointer_fixture,
    wait_for_stable_pointer_fixture,
    wait_for_state,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
EIS_INPUT_USED = "PortalEisInputUsed"
EIS_INPUT_FALLBACK = "PortalEisInputFallback"


def require_real_wayland_session() -> None:
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR", "").strip()
    wayland_display = os.environ.get("WAYLAND_DISPLAY", "").strip()
    if not runtime_dir or not wayland_display:
        raise RuntimeError("XDG_RUNTIME_DIR and WAYLAND_DISPLAY must point at the real session")
    socket_path = Path(runtime_dir) / wayland_display
    if not socket_path.is_socket():
        raise RuntimeError(f"Wayland socket does not exist: {socket_path}")


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True), encoding="utf-8")


def hide_agent_cursor(service_socket_path: Path) -> dict[str, Any] | None:
    if not service_socket_path.exists():
        return None
    payload = {
        "type": "hide_agent_cursor",
        "reason": "live_wayland_pointer_smoke cleanup",
    }
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(2.0)
        client.connect(str(service_socket_path))
        client.sendall(json.dumps(payload).encode("utf-8") + b"\n")
        chunks: list[bytes] = []
        while True:
            chunk = client.recv(4096)
            if not chunk:
                break
            chunks.append(chunk)
            if b"\n" in chunk:
                break
    if not chunks:
        return None
    return json.loads(b"".join(chunks).split(b"\n", 1)[0])


def terminate_processes_for_temp_socket(service_socket_path: Path) -> None:
    overlay_socket_path = service_socket_path.parent / "agent-cursor.sock"
    current_pid = os.getpid()
    targets: set[int] = set()
    for proc_dir in Path("/proc").iterdir():
        if not proc_dir.name.isdecimal():
            continue
        pid = int(proc_dir.name)
        if pid == current_pid:
            continue
        try:
            environ = (proc_dir / "environ").read_bytes()
            cmdline = (proc_dir / "cmdline").read_bytes()
        except OSError:
            continue
        if (
            b"sky-cua-service" in cmdline
            and f"SKY_CUA_SERVICE_SOCKET_PATH={service_socket_path}".encode() in environ
        ) or (b"sky-cua-overlay-host" in cmdline and str(overlay_socket_path).encode() in cmdline):
            targets.add(pid)

    for pid in targets:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            continue
    deadline = time.time() + 2.0
    remaining = set(targets)
    while remaining and time.time() < deadline:
        for pid in list(remaining):
            if not Path(f"/proc/{pid}").exists():
                remaining.remove(pid)
        if remaining:
            time.sleep(0.05)
    for pid in remaining:
        with suppress(ProcessLookupError):
            os.kill(pid, signal.SIGKILL)


def action_diagnostic_codes(result: dict[str, Any]) -> set[str]:
    structured = result.get("structuredContent") or {}
    diagnostics = structured.get("diagnostics") or []
    return {
        code
        for entry in diagnostics
        if isinstance(entry, dict) and isinstance(code := entry.get("code"), str)
    }


def require_gnome_eis_input_used(result: dict[str, Any], action: str, *, is_gnome: bool) -> None:
    if not is_gnome:
        return
    codes = action_diagnostic_codes(result)
    if EIS_INPUT_FALLBACK in codes or EIS_INPUT_USED not in codes:
        msg = (
            f"{action} did not use GNOME RemoteDesktop EIS input. "
            f"diagnostic_codes={sorted(codes)} "
            f"result={json.dumps(result, indent=2, sort_keys=True)}"
        )
        if os.environ.get("SKY_CUA_REQUIRE_EIS") == "1":
            raise RuntimeError(msg)
        print(f"WARNING: {msg}", file=sys.stderr)


def main() -> int:
    require_real_wayland_session()

    artifact_root = REPO_ROOT / "artifacts" / "gui-desktop-smoke" / "wayland-pointer"
    artifact_dir = artifact_root / time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    artifact_dir.mkdir(parents=True, exist_ok=True)

    step_delay = float(os.environ.get("SKY_CUA_VISIBLE_STEP_DELAY_SECONDS", "2.5"))
    final_hold = float(os.environ.get("SKY_CUA_VISIBLE_FINAL_HOLD_SECONDS", "20"))
    is_gnome = "gnome" in os.environ.get("XDG_CURRENT_DESKTOP", "").lower()

    fixture_env = {
        "GDK_BACKEND": "wayland",
        "SKY_CUA_POINTER_FULLSCREEN": os.environ.get("SKY_CUA_POINTER_FULLSCREEN", "1"),
    }
    default_agent_cursor = "0" if is_gnome else "1"
    client_env = {
        "GDK_BACKEND": "wayland",
        "DISPLAY": "",
        "SKY_CUA_AGENT_CURSOR": os.environ.get("SKY_CUA_AGENT_CURSOR", default_agent_cursor),
        "SKY_CUA_SCREENSHOT_CURSOR": os.environ.get("SKY_CUA_SCREENSHOT_CURSOR", "1"),
    }

    with tempfile.TemporaryDirectory(prefix="sky-cua-wayland-pointer-") as tmpdir:
        tmpdir_path = Path(tmpdir)
        state_path = tmpdir_path / "state.json"
        service_socket_path = tmpdir_path / "service.sock"
        client_env["SKY_CUA_SERVICE_SOCKET_PATH"] = str(service_socket_path)
        fixture = run_pointer_fixture(state_path, extra_env=fixture_env)
        try:
            state = wait_for_stable_pointer_fixture(state_path, deadline=time.time() + 20)
            write_json(artifact_dir / "initial-state.json", state)
            print(f"Visible Wayland pointer fixture ready; artifacts: {artifact_dir}")
            print(f"points={json.dumps(state['points'], sort_keys=True)}")

            client = McpClient([str(CLIENT), "mcp"], extra_env=client_env)
            try:
                client.initialize()
                tools = {tool["name"] for tool in client.tools_list()}
                required_tools = {
                    "click",
                    "drag",
                    "scroll",
                    "perform_secondary_action",
                    "type_text",
                    "press_key",
                }
                missing = sorted(required_tools - tools)
                if missing:
                    raise RuntimeError(f"MCP server did not advertise required tools: {missing}")

                # Activate the fixture window explicitly on both GNOME and Plasma
                # to ensure it has focus before EIS input is sent.
                if "activate_window" not in tools:
                    raise RuntimeError("MCP server did not advertise activate_window")

                # Discover the fixture window by exact title + PID when the
                # backend exposes PID. COSMIC's toplevel protocol does not.
                list_result = client.tools_call(
                    99,
                    "list_windows",
                    {},
                )
                write_json(artifact_dir / "pre-activate-windows.json", list_result)
                windows = (list_result.get("structuredContent") or {}).get("windows") or []
                title_matches = [w for w in windows if w.get("title") == "sky-cua pointer smoke"]
                fixture_window = next(
                    (w for w in title_matches if w.get("pid") == fixture.pid),
                    title_matches[0] if len(title_matches) == 1 else None,
                )
                if fixture_window is None:
                    raise RuntimeError(
                        "Did not find a unique fixture window with title "
                        f"'sky-cua pointer smoke' and pid {fixture.pid}. "
                        f"windows={json.dumps(windows, indent=2)}"
                    )

                activate_result = client.tools_call(
                    100,
                    "activate_window",
                    {"window_id": fixture_window["window_id"]},
                )
                write_json(artifact_dir / "activate-window-result.json", activate_result)
                require_ok(activate_result, "pointer fixture window activation")

                # Verify the fixture is actually focused after activation.
                focused_result = client.tools_call(
                    101,
                    "focused_window",
                    {},
                )
                write_json(artifact_dir / "focused-window-after-activate.json", focused_result)
                focused_window = (focused_result.get("structuredContent") or {}).get("window")
                if focused_window is None or focused_window.get("title") != "sky-cua pointer smoke":
                    raise RuntimeError(
                        "Fixture window did not gain focus after activation. "
                        f"focused={json.dumps(focused_window, indent=2)}"
                    )

                state = wait_for_stable_pointer_fixture(state_path, deadline=time.time() + 8)
                write_json(artifact_dir / "post-activate-state.json", state)
                print(f"post_activate_points={json.dumps(state['points'], sort_keys=True)}")

                print(f"Waiting {step_delay:.1f}s before click...")
                time.sleep(step_delay)
                click_point = state["points"]["click_button"]
                click_result = client.tools_call(
                    102,
                    "click",
                    {"x": click_point["x"], "y": click_point["y"]},
                )
                write_json(artifact_dir / "click-result.json", click_result)
                require_ok(click_result, "visible Wayland physical click")
                if state_path.exists():
                    write_json(
                        artifact_dir / "post-click-state.json",
                        json.loads(state_path.read_text()),
                    )
                require_gnome_eis_input_used(
                    click_result, "visible Wayland physical click", is_gnome=is_gnome
                )
                wait_for_state(
                    state_path,
                    lambda current: (
                        bool(current.get("clicked"))
                        or (
                            bool(current.get("button_press_seen"))
                            and bool(current.get("button_release_seen"))
                        )
                    ),
                    deadline=time.time() + 8,
                    description="visible Wayland click acknowledgement",
                )
                print("Visible Wayland click passed.")

                print(f"Waiting {step_delay:.1f}s before secondary click...")
                time.sleep(step_delay)
                secondary_point = state["points"]["secondary"]
                secondary_result = client.tools_call(
                    103,
                    "perform_secondary_action",
                    {"x": secondary_point["x"], "y": secondary_point["y"]},
                )
                write_json(artifact_dir / "secondary-result.json", secondary_result)
                require_ok(secondary_result, "visible Wayland secondary click")
                require_gnome_eis_input_used(
                    secondary_result, "visible Wayland secondary click", is_gnome=is_gnome
                )
                wait_for_state(
                    state_path,
                    lambda current: bool(current.get("secondary_clicked")),
                    deadline=time.time() + 8,
                    description="visible Wayland secondary-click acknowledgement",
                )
                print("Visible Wayland secondary click passed.")

                print(f"Waiting {step_delay:.1f}s before drag...")
                time.sleep(step_delay)
                drag_from = state["points"]["drag_from"]
                drag_to = state["points"]["drag_to"]
                drag_result = client.tools_call(
                    104,
                    "drag",
                    {
                        "x": drag_from["x"],
                        "y": drag_from["y"],
                        "to_x": drag_to["x"],
                        "to_y": drag_to["y"],
                    },
                )
                write_json(artifact_dir / "drag-result.json", drag_result)
                require_ok(drag_result, "visible Wayland physical drag")
                require_gnome_eis_input_used(
                    drag_result, "visible Wayland physical drag", is_gnome=is_gnome
                )
                wait_for_state(
                    state_path,
                    lambda current: bool(current.get("drag_completed")),
                    deadline=time.time() + 8,
                    description="visible Wayland drag acknowledgement",
                )
                print("Visible Wayland drag passed.")

                print(f"Waiting {step_delay:.1f}s before scroll...")
                time.sleep(step_delay)
                scroll_point = state["points"].get("scroll_safe", state["points"]["scroll"])
                before_scroll = load_state(state_path) or {}
                starting_scrolls = int(before_scroll.get("scroll_events", 0))
                scroll_result = client.tools_call(
                    105,
                    "scroll",
                    {"x": scroll_point["x"], "y": scroll_point["y"], "delta_y": -180.0},
                )
                write_json(artifact_dir / "scroll-result.json", scroll_result)
                require_ok(scroll_result, "visible Wayland physical scroll")
                require_gnome_eis_input_used(
                    scroll_result, "visible Wayland physical scroll", is_gnome=is_gnome
                )
                final_state = wait_for_state(
                    state_path,
                    lambda current: int(current.get("scroll_events", 0)) > starting_scrolls,
                    deadline=time.time() + 8,
                    description="visible Wayland scroll acknowledgement",
                )
                write_json(artifact_dir / "final-state.json", final_state)
                print("Visible Wayland scroll passed.")

                print(f"Waiting {step_delay:.1f}s before text entry...")
                time.sleep(step_delay)
                text_point = state["points"]["text_entry"]
                text_value = "cosmic-text-smoke"
                focus_result = client.tools_call(
                    106,
                    "click",
                    {"x": text_point["x"], "y": text_point["y"]},
                )
                write_json(artifact_dir / "text-focus-result.json", focus_result)
                require_ok(focus_result, "visible Wayland text-entry focus click")
                require_gnome_eis_input_used(
                    focus_result,
                    "visible Wayland text-entry focus click",
                    is_gnome=is_gnome,
                )
                time.sleep(0.35)

                type_result = client.tools_call(
                    107,
                    "type_text",
                    {"text": text_value},
                )
                write_json(artifact_dir / "type-result.json", type_result)
                require_ok(type_result, "visible Wayland type_text")
                require_gnome_eis_input_used(
                    type_result, "visible Wayland type_text", is_gnome=is_gnome
                )
                wait_for_state(
                    state_path,
                    lambda current: current.get("entry_text") == text_value,
                    deadline=time.time() + 8,
                    description="visible Wayland type_text acknowledgement",
                )

                key_result = client.tools_call(
                    108,
                    "press_key",
                    {"key": "Enter"},
                )
                write_json(artifact_dir / "press-key-result.json", key_result)
                require_ok(key_result, "visible Wayland press_key")
                require_gnome_eis_input_used(
                    key_result, "visible Wayland press_key", is_gnome=is_gnome
                )
                final_state = wait_for_state(
                    state_path,
                    lambda current: current.get("submitted_text") == text_value,
                    deadline=time.time() + 8,
                    description="visible Wayland press_key acknowledgement",
                )
                write_json(artifact_dir / "final-state.json", final_state)
                print("Visible Wayland type_text + press_key passed.")
                print(f"Holding final fixture state for {final_hold:.1f}s...")
                time.sleep(final_hold)
            finally:
                try:
                    hide_result = hide_agent_cursor(service_socket_path)
                    if hide_result is not None:
                        write_json(artifact_dir / "hide-agent-cursor-result.json", hide_result)
                except Exception as error:
                    write_json(
                        artifact_dir / "hide-agent-cursor-error.json",
                        {"error": f"{type(error).__name__}: {error}"},
                    )
                client.close()
                terminate_processes_for_temp_socket(service_socket_path)
        finally:
            if fixture.poll() is None:
                fixture.terminate()
                try:
                    fixture.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    fixture.kill()
            stderr = fixture.stderr.read() if fixture.stderr is not None else ""
            if stderr.strip():
                (artifact_dir / "fixture.stderr.log").write_text(stderr, encoding="utf-8")
                print("Pointer fixture stderr:", stderr.strip(), file=sys.stderr)

    print(f"Visible Wayland pointer smoke completed successfully; artifacts: {artifact_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
