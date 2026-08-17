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

from _fixture_acknowledgement import (  # type: ignore[import-not-found]
    wait_for_press_key_acknowledgement,
    wait_for_type_text_acknowledgement,
)
from live_desktop_smoke import (  # type: ignore[import-not-found]
    CLIENT,
    McpClient,
    find_scroll_region,
    isolated_daemon_env,
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


def terminate_fixture(fixture: subprocess.Popen[str]) -> tuple[str, str]:
    if fixture.poll() is None:
        fixture.terminate()
    try:
        return fixture.communicate(timeout=2)
    except subprocess.TimeoutExpired:
        fixture.kill()
        return fixture.communicate(timeout=2)


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
    structured = grouped_structured_result(result)
    diagnostics = structured.get("diagnostics") or []
    return {
        code
        for entry in diagnostics
        if isinstance(entry, dict) and isinstance(code := entry.get("code"), str)
    }


def grouped_structured_result(result: dict[str, Any]) -> dict[str, Any]:
    structured = result.get("structuredContent") or {}
    if not isinstance(structured, dict):
        return {}
    nested = structured.get("result")
    return nested if isinstance(nested, dict) else structured


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


def _observe_desktop(client: McpClient, call_id: int) -> str:
    """Call observe on the desktop and return the appshot_id."""
    result = client.tools_call(call_id, "observe", {"surface": "desktop"})
    structured = grouped_structured_result(result)
    appshot_id = structured.get("appshot_id")
    if not isinstance(appshot_id, str) or not appshot_id:
        raise RuntimeError(
            f"desktop observe did not return appshot_id: "
            f"{json.dumps(result, indent=2, sort_keys=True)}"
        )
    return appshot_id


def _drag_call(
    client: McpClient,
    call_id: int,
    point_from: dict[str, float],
    point_to: dict[str, float],
    *,
    duration_ms: int,
    appshot_id: str | None = None,
) -> dict[str, Any]:
    args: dict[str, Any] = {
        "operation": "drag",
        "x": point_from["x"],
        "y": point_from["y"],
        "to_x": point_to["x"],
        "to_y": point_to["y"],
        "duration_ms": duration_ms,
    }
    if appshot_id is not None:
        args["appshot_id"] = appshot_id
    return client.tools_call(call_id, "desktop_pointer", args)


def drive_extended_controls(
    client: McpClient,
    state_path: Path,
    points: dict[str, dict[str, float]],
    artifact_dir: Path,
    *,
    appshot_id: str,
) -> None:
    """Exercise the richer control surface (sliders, DnD, spin, combo, switch, XY pad).

    These are the targets that harden dragging and drag-and-drop. The slider, DnD
    and XY-pad proofs pass a duration_ms so the backend emits an interpolated,
    paced drag — the behavior a single teleport cannot satisfy.
    """
    # Horizontal slider: drag the thumb rightward; the value must track.
    result = _drag_call(
        client,
        150,
        points["slider_h_from"],
        points["slider_h_to"],
        duration_ms=600,
        appshot_id=appshot_id,
    )
    write_json(artifact_dir / "slider-h-result.json", result)
    require_ok(result, "visible Wayland horizontal slider drag")
    wait_for_state(
        state_path,
        lambda current: float(current.get("slider_h_value", 0.0)) >= 50.0,
        deadline=time.time() + 8,
        description="horizontal slider value tracked the drag",
    )
    print("Horizontal slider drag passed.")

    # Vertical slider: drag the thumb downward (top = min on a GTK vertical scale).
    result = _drag_call(
        client,
        151,
        points["slider_v_from"],
        points["slider_v_to"],
        duration_ms=600,
        appshot_id=appshot_id,
    )
    write_json(artifact_dir / "slider-v-result.json", result)
    require_ok(result, "visible Wayland vertical slider drag")
    wait_for_state(
        state_path,
        lambda current: float(current.get("slider_v_value", 0.0)) >= 40.0,
        deadline=time.time() + 8,
        description="vertical slider value tracked the drag",
    )
    print("Vertical slider drag passed.")

    # Drag-and-drop: the keystone. GTK only arms a drag gesture once motion
    # crosses its threshold under the button grab, which only the interpolated
    # backend drag produces.
    result = _drag_call(
        client,
        152,
        points["dnd_source"],
        points["dnd_target"],
        duration_ms=600,
        appshot_id=appshot_id,
    )
    write_json(artifact_dir / "dnd-result.json", result)
    require_ok(result, "visible Wayland drag-and-drop")
    wait_for_state(
        state_path,
        lambda current: (
            bool(current.get("dnd_dropped")) and current.get("dnd_payload") == "sky-cua-chip"
        ),
        deadline=time.time() + 8,
        description="drag-and-drop delivered the chip to the drop zone",
    )
    print("Drag-and-drop passed.")

    # Spin button: click the up-stepper three times.
    for index in range(3):
        result = client.tools_call(
            153 + index,
            "desktop_pointer",
            {
                "operation": "click",
                "appshot_id": appshot_id,
                "x": points["spin_up_button"]["x"],
                "y": points["spin_up_button"]["y"],
            },
        )
        require_ok(result, f"visible Wayland spin increment {index + 1}")
    wait_for_state(
        state_path,
        lambda current: int(current.get("spin_value", 0)) >= 3,
        deadline=time.time() + 8,
        description="spin button reached three increments",
    )
    print("Spin button increments passed.")

    # Combo box: click to open, then keyboard-select the next item. This is the
    # only extended-control step that needs the keyboard, so honor a profile that
    # opted out of keyboard input (e.g. wayland-pointer-scaled sets this env);
    # the pointer-driven drag controls above still run.
    if os.environ.get("SKY_CUA_POINTER_SKIP_KEYBOARD") == "1":
        print("Skipping combo selection (keyboard-driven) for pointer-only profile.")
    else:
        result = client.tools_call(
            156,
            "desktop_pointer",
            {
                "operation": "click",
                "appshot_id": appshot_id,
                "x": points["combo"]["x"],
                "y": points["combo"]["y"],
            },
        )
        require_ok(result, "visible Wayland combo open")
        for call_id, key in ((157, "Down"), (158, "Return")):
            key_result = client.tools_call(
                call_id,
                "desktop_keyboard",
                {"operation": "press_key", "key": key, "appshot_id": appshot_id},
            )
            require_ok(key_result, f"visible Wayland combo {key}")
        wait_for_state(
            state_path,
            lambda current: int(current.get("combo_index", 0)) >= 1,
            deadline=time.time() + 8,
            description="combo box selected a non-default item",
        )
        print("Combo selection passed.")

    # Switch: prefer a knob drag (exercises smooth motion); fall back to a click
    # if the synthetic knob drag does not toggle.
    result = _drag_call(
        client,
        159,
        points["switch_off"],
        points["switch_on"],
        duration_ms=400,
        appshot_id=appshot_id,
    )
    write_json(artifact_dir / "switch-result.json", result)
    require_ok(result, "visible Wayland switch drag")
    try:
        wait_for_state(
            state_path,
            lambda current: bool(current.get("switch_active")),
            deadline=time.time() + 4,
            description="switch toggled on via knob drag",
        )
        print("Switch knob drag passed.")
    except RuntimeError:
        click_result = client.tools_call(
            160,
            "desktop_pointer",
            {
                "operation": "click",
                "appshot_id": appshot_id,
                "x": points["switch"]["x"],
                "y": points["switch"]["y"],
            },
        )
        require_ok(click_result, "visible Wayland switch click fallback")
        wait_for_state(
            state_path,
            lambda current: bool(current.get("switch_active")),
            deadline=time.time() + 8,
            description="switch toggled on via click fallback",
        )
        print("Switch click fallback passed.")

    # 2D drag pad: free-form drag; the path must reach the bottom-right quadrant.
    result = _drag_call(
        client,
        161,
        points["xy_pad_from"],
        points["xy_pad_to"],
        duration_ms=500,
        appshot_id=appshot_id,
    )
    write_json(artifact_dir / "xy-pad-result.json", result)
    require_ok(result, "visible Wayland 2D drag pad")
    wait_for_state(
        state_path,
        lambda current: (
            bool(current.get("xy_pad_dragged"))
            and float(current.get("xy_pad_x", 0.0)) > 0.5
            and float(current.get("xy_pad_y", 0.0)) > 0.5
        ),
        deadline=time.time() + 8,
        description="2D drag pad tracked the path to the destination quadrant",
    )
    print("2D drag pad passed.")


def main() -> int:
    require_real_wayland_session()

    artifact_root = REPO_ROOT / "artifacts" / "gui-desktop-smoke" / "wayland-pointer"
    artifact_dir = artifact_root / time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    artifact_dir.mkdir(parents=True, exist_ok=True)

    step_delay = float(os.environ.get("SKY_CUA_VISIBLE_STEP_DELAY_SECONDS", "2.5"))
    final_hold = float(os.environ.get("SKY_CUA_VISIBLE_FINAL_HOLD_SECONDS", "20"))
    fixture_ready_timeout = float(
        os.environ.get("SKY_CUA_POINTER_FIXTURE_READY_TIMEOUT_SECONDS", "45")
    )
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
        # The smoke owns an isolated daemon; keep it off the machine-wide
        # phone-direct TCP endpoint so a lingering daemon cannot break the run.
        client_env = isolated_daemon_env(client_env)
        fixture = run_pointer_fixture(state_path, extra_env=fixture_env)
        try:
            try:
                state = wait_for_stable_pointer_fixture(
                    state_path,
                    deadline=time.time() + fixture_ready_timeout,
                )
            except RuntimeError as first_error:
                stdout, stderr = terminate_fixture(fixture)
                write_json(
                    artifact_dir / "fixture-startup-retry.json",
                    {
                        "error": str(first_error),
                        "returncode": fixture.returncode,
                        "stdout": stdout,
                        "stderr": stderr,
                    },
                )
                state_path.unlink(missing_ok=True)
                fixture = run_pointer_fixture(state_path, extra_env=fixture_env)
                state = wait_for_stable_pointer_fixture(
                    state_path,
                    deadline=time.time() + fixture_ready_timeout,
                )
            write_json(artifact_dir / "initial-state.json", state)
            print(f"Visible Wayland pointer fixture ready; artifacts: {artifact_dir}")
            print(f"points={json.dumps(state['points'], sort_keys=True)}")

            client = McpClient([str(CLIENT), "mcp"], extra_env=client_env)
            try:
                client.initialize()
                tools = {tool["name"] for tool in client.tools_list()}
                required_tools = {
                    "desktop_keyboard",
                    "desktop_pointer",
                    "desktop_scroll",
                    "list_resources",
                    "observe",
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
                    "list_resources",
                    {"surface": "desktop", "resource": "windows"},
                )
                write_json(artifact_dir / "pre-activate-windows.json", list_result)
                windows = grouped_structured_result(list_result).get("windows") or []
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
                if fixture_window.get(
                    "backend"
                ) == "kwin" and "WindowFocusVerified" not in action_diagnostic_codes(
                    activate_result
                ):
                    raise RuntimeError(
                        "KWin activate_window did not report the focus-verified contract. "
                        f"result={json.dumps(activate_result, indent=2)}"
                    )

                # Verify the fixture is actually focused after activation.
                focused_result = client.tools_call(
                    101,
                    "list_resources",
                    {"surface": "desktop", "resource": "focused_window"},
                )
                write_json(artifact_dir / "focused-window-after-activate.json", focused_result)
                focused_window = grouped_structured_result(focused_result).get("window")
                # All backends, including KWin (scripted active-window readback),
                # must report the fixture as focused after verified activation.
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
                appshot_id = _observe_desktop(client, 110)
                click_result = client.tools_call(
                    102,
                    "desktop_pointer",
                    {
                        "operation": "click",
                        "appshot_id": appshot_id,
                        "x": click_point["x"],
                        "y": click_point["y"],
                    },
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
                    "desktop_pointer",
                    {
                        "operation": "secondary_click",
                        "appshot_id": appshot_id,
                        "x": secondary_point["x"],
                        "y": secondary_point["y"],
                    },
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
                    "desktop_pointer",
                    {
                        "operation": "drag",
                        "appshot_id": appshot_id,
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
                before_scroll = load_state(state_path) or {}
                starting_scrolls = int(before_scroll.get("scroll_events", 0))
                observe_result = client.tools_call(
                    105,
                    "observe",
                    {
                        "surface": "desktop",
                        "detail": "full",
                        "element_query": "Scroll region",
                        "element_limit": 20,
                    },
                )
                write_json(artifact_dir / "pre-scroll-observe.json", observe_result)
                require_ok(observe_result, "visible Wayland pre-scroll observation")
                observed = grouped_structured_result(observe_result)
                # Flatten semantic_projection elements to top level for find_scroll_region.
                sem = observed.get("semantic_projection") or {}
                if "elements" not in observed and isinstance(sem.get("elements"), list):
                    observed = {**observed, "elements": sem["elements"]}
                scroll_appshot_id = observed.get("appshot_id") or ""
                snapshot_id = observed.get("snapshot_id") or (
                    observed.get("action_snapshot") or {}
                ).get("snapshot_id")
                if not isinstance(snapshot_id, str) or not snapshot_id:
                    raise RuntimeError(
                        "pre-scroll observation did not return a snapshot_id. "
                        f"result={json.dumps(observe_result, indent=2)}"
                    )
                scroll_region = find_scroll_region(observed)
                scroll_result = client.tools_call(
                    106,
                    "desktop_scroll",
                    {
                        "direction": "down",
                        "pages": 1,
                        "appshot_id": scroll_appshot_id,
                        "snapshot_id": snapshot_id,
                        "element_index": scroll_region["element_index"],
                    },
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

                # Extended control surface (sliders, drag-and-drop, spin, combo,
                # switch, 2D pad). Gated on the fixture version so older fixtures
                # don't fail this lane. These run before the keyboard-skip gate
                # because they are pointer-driven drag/click hardening.
                controls_state = load_state(state_path) or {}
                if int(controls_state.get("fixture_controls_version", 0)) >= 2:
                    appshot_id = _observe_desktop(client, 200)
                    drive_extended_controls(
                        client,
                        state_path,
                        state["points"],
                        artifact_dir,
                        appshot_id=appshot_id,
                    )
                    print("Extended control surface passed.")
                else:
                    print("Fixture predates extended controls; skipping slider/DnD proofs.")

                if os.environ.get("SKY_CUA_POINTER_SKIP_KEYBOARD") == "1":
                    print("Skipping keyboard proof for this pointer-focused profile.")
                    print(f"Holding final fixture state for {final_hold:.1f}s...")
                    time.sleep(final_hold)
                    return 0

                print(f"Waiting {step_delay:.1f}s before text entry...")
                time.sleep(step_delay)
                text_point = state["points"]["text_entry"]
                text_value = "cosmic-text-smoke"
                focus_result = client.tools_call(
                    107,
                    "desktop_pointer",
                    {
                        "operation": "click",
                        "appshot_id": appshot_id,
                        "x": text_point["x"],
                        "y": text_point["y"],
                    },
                )
                write_json(artifact_dir / "text-focus-result.json", focus_result)
                require_ok(focus_result, "visible Wayland text-entry focus click")
                require_gnome_eis_input_used(
                    focus_result,
                    "visible Wayland text-entry focus click",
                    is_gnome=is_gnome,
                )
                focused_entry_state = wait_for_state(
                    state_path,
                    lambda current: bool(current.get("entry_focused")),
                    deadline=time.time() + 8,
                    description="visible Wayland text-entry focus",
                )
                write_json(artifact_dir / "text-focused-state.json", focused_entry_state)

                type_result = client.tools_call(
                    108,
                    "desktop_keyboard",
                    {"operation": "type_text", "text": text_value, "appshot_id": appshot_id},
                )
                write_json(artifact_dir / "type-result.json", type_result)
                require_ok(type_result, "visible Wayland type_text")
                require_gnome_eis_input_used(
                    type_result, "visible Wayland type_text", is_gnome=is_gnome
                )
                wait_for_type_text_acknowledgement(
                    client,
                    108,
                    state_path,
                    text_value=text_value,
                    appshot_id=appshot_id,
                    artifact_dir=artifact_dir,
                )

                key_result = client.tools_call(
                    109,
                    "desktop_keyboard",
                    {"operation": "press_key", "key": "Enter", "appshot_id": appshot_id},
                )
                write_json(artifact_dir / "press-key-result.json", key_result)
                require_ok(key_result, "visible Wayland press_key")
                require_gnome_eis_input_used(
                    key_result, "visible Wayland press_key", is_gnome=is_gnome
                )
                final_state = wait_for_press_key_acknowledgement(
                    client,
                    109,
                    state_path,
                    text_value=text_value,
                    appshot_id=appshot_id,
                    artifact_dir=artifact_dir,
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
