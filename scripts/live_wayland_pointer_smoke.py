#!/usr/bin/env python3
"""Visible Wayland pointer smoke for the real desktop session.

This runs the fullscreen GTK pointer fixture on the current Wayland display and
drives click, drag, and scroll actions through the Computer Use MCP server. It
does not start Xvfb or any nested display server.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
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


def main() -> int:
    require_real_wayland_session()

    artifact_root = REPO_ROOT / "artifacts" / "gui-desktop-smoke" / "wayland-pointer"
    artifact_dir = artifact_root / time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    artifact_dir.mkdir(parents=True, exist_ok=True)

    step_delay = float(os.environ.get("SKY_CUA_VISIBLE_STEP_DELAY_SECONDS", "2.5"))
    final_hold = float(os.environ.get("SKY_CUA_VISIBLE_FINAL_HOLD_SECONDS", "20"))

    fixture_env = {
        "GDK_BACKEND": "wayland",
        "SKY_CUA_POINTER_FULLSCREEN": "1",
    }
    client_env = {
        "GDK_BACKEND": "wayland",
        "DISPLAY": "",
        "SKY_CUA_AGENT_CURSOR": os.environ.get("SKY_CUA_AGENT_CURSOR", "1"),
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

                print(f"Waiting {step_delay:.1f}s before click...")
                time.sleep(step_delay)
                click_point = state["points"]["click_button"]
                click_result = client.tools_call(
                    100,
                    "click",
                    {"x": click_point["x"], "y": click_point["y"]},
                )
                write_json(artifact_dir / "click-result.json", click_result)
                require_ok(click_result, "visible Wayland physical click")
                wait_for_state(
                    state_path,
                    lambda current: bool(current.get("clicked")),
                    deadline=time.time() + 8,
                    description="visible Wayland click acknowledgement",
                )
                print("Visible Wayland click passed.")

                print(f"Waiting {step_delay:.1f}s before drag...")
                time.sleep(step_delay)
                drag_from = state["points"]["drag_from"]
                drag_to = state["points"]["drag_to"]
                drag_result = client.tools_call(
                    101,
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
                    102,
                    "scroll",
                    {"x": scroll_point["x"], "y": scroll_point["y"], "delta_y": -180.0},
                )
                write_json(artifact_dir / "scroll-result.json", scroll_result)
                require_ok(scroll_result, "visible Wayland physical scroll")
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
                    103,
                    "click",
                    {"x": text_point["x"], "y": text_point["y"]},
                )
                write_json(artifact_dir / "text-focus-result.json", focus_result)
                require_ok(focus_result, "visible Wayland text-entry focus click")
                time.sleep(0.35)

                type_result = client.tools_call(
                    104,
                    "type_text",
                    {"text": text_value},
                )
                write_json(artifact_dir / "type-result.json", type_result)
                require_ok(type_result, "visible Wayland type_text")
                wait_for_state(
                    state_path,
                    lambda current: current.get("entry_text") == text_value,
                    deadline=time.time() + 8,
                    description="visible Wayland type_text acknowledgement",
                )

                key_result = client.tools_call(
                    105,
                    "press_key",
                    {"key": "Enter"},
                )
                write_json(artifact_dir / "press-key-result.json", key_result)
                require_ok(key_result, "visible Wayland press_key")
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
                client.close()
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
