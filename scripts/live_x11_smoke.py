#!/usr/bin/env python3
"""Isolated pure-X11 smoke harness for sky-cua.

This spins up a nested Xvfb display, launches the MCP client against that
display, and proves that the X11 capture/input fallback lane works without
leaning on the host Wayland session.
"""

from __future__ import annotations

import json
import os
import shutil
import stat
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
    pick_x11_click_target,
    require_ok,
    require_x11_action_region_hints,
    run_pointer_fixture,
    wait_for_stable_pointer_fixture,
    wait_for_state,
    wait_for_x11_window_titles,
    x11_click_arguments,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
X11_POINTER_WIDTH = 1280
X11_POINTER_HEIGHT = 900


def require_installed(binary: str) -> None:
    if shutil.which(binary) is None:
        raise RuntimeError(f"required binary is not installed: {binary}")


def read_stream_line(stream: Any, timeout: float) -> str:
    if stream is None:
        raise RuntimeError("subprocess stream was not available")
    deadline = time.time() + timeout
    while time.time() < deadline:
        line = stream.readline()
        if line:
            return line
        time.sleep(0.05)
    raise RuntimeError("timed out waiting for Xvfb to announce a display number")


def start_xvfb() -> tuple[subprocess.Popen[str], str]:
    require_installed("Xvfb")
    process = subprocess.Popen(
        [
            "Xvfb",
            "-screen",
            "0",
            f"{X11_POINTER_WIDTH}x{X11_POINTER_HEIGHT}x24",
            "-nolisten",
            "tcp",
            "-displayfd",
            "1",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        cwd=REPO_ROOT,
    )

    try:
        display_suffix = read_stream_line(process.stdout, timeout=8).strip()
    except Exception as exc:
        stderr = process.stderr.read() if process.stderr is not None else ""
        process.kill()
        raise RuntimeError(f"Xvfb failed to announce a display.\nstderr={stderr}") from exc

    if not display_suffix.isdigit():
        stderr = process.stderr.read() if process.stderr is not None else ""
        process.kill()
        raise RuntimeError(
            f"Xvfb announced a non-numeric display {display_suffix!r}.\nstderr={stderr}"
        )

    display = f":{display_suffix}"
    return process, display


def wait_for_x11_display(display: str, *, deadline: float) -> None:
    env = {"DISPLAY": display}
    while time.time() < deadline:
        ready = subprocess.run(
            ["xdpyinfo"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
            env={**os.environ, **env},
        )
        if ready.returncode == 0:
            return
        time.sleep(0.1)
    raise RuntimeError(f"timed out waiting for nested X11 display {display} to become ready")


def terminate_process(process: subprocess.Popen[str], *, name: str) -> None:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    stderr = process.stderr.read() if process.stderr is not None else ""
    if stderr.strip():
        print(f"{name} stderr: {stderr.strip()}", file=sys.stderr)


def make_runtime_dir(base_dir: Path) -> Path:
    runtime_dir = base_dir / "runtime"
    runtime_dir.mkdir(parents=True, exist_ok=True)
    runtime_dir.chmod(stat.S_IRWXU)
    return runtime_dir


def assert_x11_snapshot(snapshot: dict[str, Any], *, label: str) -> dict[str, Any]:
    environment = snapshot.get("environment") or {}
    if environment.get("session_kind") != "x11":
        raise RuntimeError(
            f"{label} snapshot did not report a pure X11 session.\n"
            f"environment={json.dumps(environment, indent=2, sort_keys=True)}"
        )
    if environment.get("capture_backend") != "x11":
        raise RuntimeError(
            f"{label} snapshot did not use the X11 capture backend.\n"
            f"environment={json.dumps(environment, indent=2, sort_keys=True)}"
        )
    if environment.get("input_backend") != "x_test":
        raise RuntimeError(
            f"{label} snapshot did not use the XTest input backend.\n"
            f"environment={json.dumps(environment, indent=2, sort_keys=True)}"
        )

    capture = snapshot.get("capture") or {}
    if capture.get("image_backend") != "x11":
        raise RuntimeError(
            f"{label} snapshot did not report X11 as the actual image backend.\n"
            f"capture={json.dumps(capture, indent=2, sort_keys=True)}"
        )
    screenshot_path = capture.get("screenshot_path")
    if not screenshot_path or not Path(screenshot_path).exists():
        raise RuntimeError(
            f"{label} snapshot did not produce a real X11 screenshot path.\n"
            f"capture={json.dumps(capture, indent=2, sort_keys=True)}"
        )
    pixel_size = capture.get("pixel_size") or {}
    if int(pixel_size.get("width", 0) or 0) <= 0 or int(pixel_size.get("height", 0) or 0) <= 0:
        raise RuntimeError(
            f"{label} snapshot did not include pixel dimensions.\n"
            f"capture={json.dumps(capture, indent=2, sort_keys=True)}"
        )

    return capture


def wait_for_listed_app(client: McpClient, title_hint: str, *, deadline: float) -> dict[str, Any]:
    lowered = title_hint.lower()
    request_id = 80
    last_apps: list[dict[str, Any]] = []

    while time.time() < deadline:
        apps = client.tools_call(request_id, "list_apps", {})["structuredContent"]["apps"]
        request_id += 1
        last_apps = apps
        matching = next(
            (
                app
                for app in apps
                if lowered in ((app.get("window_title") or "").lower())
                or lowered in ((app.get("name") or "").lower())
            ),
            None,
        )
        if matching is not None:
            return matching
        time.sleep(0.35)

    raise RuntimeError(
        f"timed out waiting for an X11 app with title containing {title_hint!r}.\n"
        f"last list_apps sample={json.dumps(last_apps, indent=2, sort_keys=True)}"
    )


def x11_fallback_probe(client: McpClient, extra_env: dict[str, str]) -> None:
    require_installed("xmessage")
    title = "sky-cua pure x11 xmessage probe"
    probe = subprocess.Popen(
        ["xmessage", "-title", title, "-buttons", "OK:0", "-center", "sky-cua x11 fallback"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        cwd=REPO_ROOT,
        env={**os.environ, **extra_env},
    )
    try:
        app = wait_for_listed_app(client, title, deadline=time.time() + 12)
        snapshot = client.tools_call(
            90,
            "get_app_state",
            {"app_id": app["app_id"]},
        )["structuredContent"]

        capture = assert_x11_snapshot(snapshot, label="xmessage")
        elements = snapshot.get("elements", [])
        bounds = elements[0].get("bounds") or {}
        if bounds.get("width", 0) <= 0 or bounds.get("height", 0) <= 0:
            raise RuntimeError(
                "X11 fallback root element did not expose usable bounds.\n"
                f"element={json.dumps(elements[0], indent=2, sort_keys=True)}"
            )
        if shutil.which("xwininfo") is not None:
            require_x11_action_region_hints(snapshot, "X11")
        descendant_region = pick_x11_click_target(snapshot)
        require_ok(
            client.tools_call(
                91,
                "click",
                x11_click_arguments(snapshot, descendant_region),
            ),
            "x11 descendant-region click",
        )
        if descendant_region.get("role") in {"x11_leaf_region", "x11_action_region"}:
            try:
                probe.wait(timeout=4)
            except subprocess.TimeoutExpired as exc:
                raise RuntimeError(
                    "Clicking the recovered pure-X11 descendant region did not dismiss xmessage.\n"
                    f"target={json.dumps(descendant_region, indent=2, sort_keys=True)}"
                ) from exc
        else:
            print(
                "X11 fallback root click was sent, but dismissal is only required "
                "when child action-region hints are available."
            )

        print("X11 list_apps/get_app_state fallback probe passed.")
        print(f"X11 focused app: {snapshot.get('focused_app')}")
        print(f"X11 fallback capture: {capture}")
        print(f"X11 fallback element count: {len(elements)}")
        print(
            "X11 fallback click smoke passed."
            if descendant_region.get("role") in {"x11_leaf_region", "x11_action_region"}
            else "X11 root fallback transport probe passed."
        )
    finally:
        terminate_process(probe, name="xmessage probe")


def x11_selector_probe(client: McpClient, extra_env: dict[str, str]) -> None:
    require_installed("xmessage")
    alpha_title = "sky-cua pure x11 selector alpha"
    beta_title = "sky-cua pure x11 selector beta"
    alpha = subprocess.Popen(
        ["xmessage", "-title", alpha_title, "-buttons", "OK:0", "-center", "alpha body"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        cwd=REPO_ROOT,
        env={**os.environ, **extra_env},
    )
    beta = subprocess.Popen(
        ["xmessage", "-title", beta_title, "-buttons", "OK:0", "-center", "beta body"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        cwd=REPO_ROOT,
        env={**os.environ, **extra_env},
    )
    try:
        request_id = 95
        wait_for_x11_window_titles(
            [alpha_title, beta_title],
            deadline=time.time() + 12,
            extra_env=extra_env,
        )

        snapshot = client.tools_call(
            request_id,
            "get_app_state",
            {
                "desktop_file_id": "xmessage.desktop",
                "window_title": beta_title,
            },
        )["structuredContent"]
        focused = snapshot.get("focused_app") or {}
        if focused.get("window_title") != beta_title:
            raise RuntimeError(
                "pure-X11 selector probe did not choose the exact title-matched window.\n"
                f"focused={json.dumps(focused, indent=2, sort_keys=True)}"
            )
        print("Pure X11 selector probe passed.")
    finally:
        terminate_process(alpha, name="xmessage selector alpha")
        terminate_process(beta, name="xmessage selector beta")


def x11_pointer_smoke(client: McpClient, extra_env: dict[str, str]) -> None:
    with tempfile.TemporaryDirectory(prefix="sky-cua-x11-pointer-") as tmpdir:
        state_path = Path(tmpdir) / "state.json"
        fixture_env = {
            **extra_env,
            "SKY_CUA_POINTER_FULLSCREEN": "0",
            "SKY_CUA_POINTER_WIDTH": str(X11_POINTER_WIDTH),
            "SKY_CUA_POINTER_HEIGHT": str(X11_POINTER_HEIGHT),
        }
        fixture = run_pointer_fixture(state_path, extra_env=fixture_env)
        try:
            state = wait_for_stable_pointer_fixture(state_path, deadline=time.time() + 20)
            print(f"X11 pointer fixture ready: {json.dumps(state['points'], sort_keys=True)}")

            pointer_state = client.tools_call(100, "get_app_state", {})["structuredContent"]
            capture = assert_x11_snapshot(pointer_state, label="pointer fixture")
            print(f"X11 pointer snapshot capture: {capture}")

            text_point = state["points"]["text_entry"]
            require_ok(
                client.tools_call(101, "click", {"x": text_point["x"], "y": text_point["y"]}),
                "x11 text-entry focus click",
            )
            require_ok(
                client.tools_call(
                    102,
                    "type_text",
                    {"x": text_point["x"], "y": text_point["y"], "text": "typed-x11"},
                ),
                "x11 type_text",
            )
            require_ok(
                client.tools_call(
                    103,
                    "press_key",
                    {"x": text_point["x"], "y": text_point["y"], "key": "Enter"},
                ),
                "x11 press_key",
            )
            wait_for_state(
                state_path,
                lambda current: current.get("submitted_text") == "typed-x11",
                deadline=time.time() + 8,
                description="typed X11 text submission",
            )
            print("X11 type_text + press_key smoke passed.")

            click_point = state["points"]["click_button"]
            require_ok(
                client.tools_call(104, "click", {"x": click_point["x"], "y": click_point["y"]}),
                "x11 physical click",
            )
            wait_for_state(
                state_path,
                lambda current: bool(current.get("clicked")),
                deadline=time.time() + 8,
                description="X11 click acknowledgement",
            )
            print("X11 physical click smoke passed.")

            secondary_point = state["points"]["secondary"]
            require_ok(
                client.tools_call(
                    105,
                    "perform_secondary_action",
                    {"x": secondary_point["x"], "y": secondary_point["y"]},
                ),
                "x11 physical secondary click",
            )
            wait_for_state(
                state_path,
                lambda current: bool(current.get("secondary_clicked")),
                deadline=time.time() + 8,
                description="X11 secondary click acknowledgement",
            )
            print("X11 physical secondary-click smoke passed.")

            drag_from = state["points"]["drag_from"]
            drag_to = state["points"]["drag_to"]
            require_ok(
                client.tools_call(
                    106,
                    "drag",
                    {
                        "x": drag_from["x"],
                        "y": drag_from["y"],
                        "to_x": drag_to["x"],
                        "to_y": drag_to["y"],
                    },
                ),
                "x11 physical drag",
            )
            wait_for_state(
                state_path,
                lambda current: bool(current.get("drag_completed")),
                deadline=time.time() + 8,
                description="X11 drag acknowledgement",
            )
            print("X11 physical drag smoke passed.")

            scroll_point = state["points"]["scroll"]
            starting_scrolls = int((load_state(state_path) or {}).get("scroll_events", 0))
            require_ok(
                client.tools_call(
                    107,
                    "scroll",
                    {"x": scroll_point["x"], "y": scroll_point["y"], "delta_y": -180.0},
                ),
                "x11 physical scroll",
            )
            wait_for_state(
                state_path,
                lambda current: int(current.get("scroll_events", 0)) > starting_scrolls,
                deadline=time.time() + 8,
                description="X11 scroll acknowledgement",
            )
            print("X11 physical scroll smoke passed.")
        finally:
            terminate_process(fixture, name="X11 pointer fixture")


def main() -> int:
    for binary in ("xdpyinfo", "xdotool", "xprop", "xwininfo"):
        require_installed(binary)

    with tempfile.TemporaryDirectory(prefix="sky-cua-x11-runtime-") as tmpdir:
        base_dir = Path(tmpdir)
        runtime_dir = make_runtime_dir(base_dir)
        xvfb, display = start_xvfb()
        try:
            wait_for_x11_display(display, deadline=time.time() + 8)
            print(f"Nested X11 display ready on {display}.")

            x11_env = {
                "DISPLAY": display,
                "XDG_SESSION_TYPE": "x11",
                "XDG_RUNTIME_DIR": str(runtime_dir),
                "WAYLAND_DISPLAY": "",
                "GDK_BACKEND": "x11",
                "QT_QPA_PLATFORM": "xcb",
            }

            client = McpClient([str(CLIENT), "mcp"], extra_env=x11_env)
            try:
                client.initialize()
                tools = {tool["name"] for tool in client.tools_list()}
                required_tools = {
                    "list_apps",
                    "get_app_state",
                    "click",
                    "perform_secondary_action",
                    "scroll",
                    "drag",
                    "type_text",
                    "press_key",
                    "set_value",
                }
                missing = sorted(required_tools - tools)
                if missing:
                    raise RuntimeError(f"MCP server did not advertise required tools: {missing}")

                x11_fallback_probe(client, x11_env)
                x11_selector_probe(client, x11_env)
                x11_pointer_smoke(client, x11_env)

                apps = client.tools_call(110, "list_apps", {})["structuredContent"]["apps"]
                print(f"Nested X11 list_apps returned {len(apps)} apps.")
                print("\nPure X11 smoke completed successfully.")
            finally:
                client.close()
        finally:
            terminate_process(xvfb, name="Xvfb")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
