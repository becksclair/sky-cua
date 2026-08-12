#!/usr/bin/env python3
"""Live desktop smoke harness for sky-cua.

This script exercises the MCP client end to end against real windows on a live
Linux desktop. It is intentionally operator-facing rather than a default CI
check because it opens real windows and may trigger portal approval prompts.
"""

from __future__ import annotations

import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

from _mcp_stdio import McpClient

REPO_ROOT = Path(__file__).resolve().parents[1]
CLIENT = REPO_ROOT / "bin" / "sky-cua-client"
POINTER_FIXTURE = REPO_ROOT / "scripts" / "gtk_pointer_smoke_fixture.py"
ZENITY_TITLE = "sky-cua zenity smoke"
POINTER_TITLE = "sky-cua pointer smoke"


def wait_for_app_snapshot(client: McpClient, title_hint: str, deadline: float) -> dict[str, Any]:
    return wait_for_app_snapshot_result(client, title_hint, deadline=deadline)["structuredContent"]


def wait_for_window_snapshot(
    client: McpClient, selector: dict[str, Any], *, deadline: float
) -> dict[str, Any]:
    request_id = 10
    last_result: dict[str, Any] | None = None
    while time.time() < deadline:
        result = client.tools_call(
            request_id,
            "observe",
            {"surface": "desktop", **selector},
        )
        request_id += 1
        normalized = normalized_grouped_result(result)
        if not normalized.get("isError"):
            return normalized["structuredContent"]
        last_result = normalized
        time.sleep(0.5)
    raise RuntimeError(
        f"timed out observing desktop selector {selector!r}.\n"
        f"last_result={json.dumps(last_result, indent=2, sort_keys=True)}"
    )


def grouped_structured_result(result: dict[str, Any]) -> dict[str, Any]:
    structured = result.get("structuredContent") or {}
    if not isinstance(structured, dict):
        return {}
    nested = structured.get("result")
    payload = nested if isinstance(nested, dict) else structured
    return normalized_appshot(payload)


def normalized_appshot(payload: dict[str, Any]) -> dict[str, Any]:
    """Expose canonical AppShot projections without discarding their fences."""
    semantic_projection = payload.get("semantic_projection")
    action_snapshot = payload.get("action_snapshot")
    if not isinstance(semantic_projection, dict) or not isinstance(action_snapshot, dict):
        return payload

    normalized = dict(payload)
    for key in ("accessibility", "elements", "focused_app"):
        if key in semantic_projection:
            normalized[key] = semantic_projection[key]
    snapshot_id = action_snapshot.get("snapshot_id")
    if isinstance(snapshot_id, str) and snapshot_id:
        normalized["snapshot_id"] = snapshot_id
    return normalized


def appshot_action_fences(snapshot: dict[str, Any]) -> dict[str, str]:
    appshot_id = snapshot.get("appshot_id")
    snapshot_id = snapshot.get("snapshot_id")
    if not isinstance(appshot_id, str) or not appshot_id:
        raise RuntimeError(f"desktop observe did not return appshot_id: {snapshot!r}")
    if not isinstance(snapshot_id, str) or not snapshot_id:
        raise RuntimeError(f"desktop observe did not return action snapshot_id: {snapshot!r}")
    return {"appshot_id": appshot_id, "snapshot_id": snapshot_id}


def normalized_grouped_result(result: dict[str, Any]) -> dict[str, Any]:
    normalized = dict(result)
    structured = result.get("structuredContent") or {}
    normalized["structuredContent"] = grouped_structured_result(result)
    if isinstance(structured, dict):
        tool = structured.get("tool")
        branch = structured.get("branch")
        prefix = f"{tool}/{branch}. " if isinstance(tool, str) and isinstance(branch, str) else None
        content = normalized.get("content")
        if prefix and isinstance(content, list):
            normalized_content: list[Any] = []
            for block in content:
                if isinstance(block, dict) and isinstance(block.get("text"), str):
                    normalized_block = dict(block)
                    text = normalized_block["text"]
                    if text.startswith(prefix):
                        normalized_block["text"] = text.removeprefix(prefix)
                    normalized_content.append(normalized_block)
                else:
                    normalized_content.append(block)
            normalized["content"] = normalized_content
    return normalized


def wait_for_app_snapshot_result(
    client: McpClient, title_hint: str, *, deadline: float
) -> dict[str, Any]:
    request_id = 10
    last_result: dict[str, Any] | None = None
    while time.time() < deadline:
        result = client.tools_call(
            request_id,
            "observe",
            {"surface": "desktop", "window_title": title_hint},
        )
        request_id += 1
        normalized = normalized_grouped_result(result)
        if not normalized.get("isError"):
            return normalized
        last_result = normalized
        time.sleep(0.5)
    raise RuntimeError(
        f"timed out observing a desktop window with title containing {title_hint!r}.\n"
        f"last_result={json.dumps(last_result, indent=2, sort_keys=True)}"
    )


def find_editable(snapshot: dict[str, Any]) -> dict[str, Any]:
    for element in snapshot["elements"]:
        if "set_value" in element.get("semantic_actions", []):
            return element
    raise RuntimeError("did not find an editable element in the focused snapshot")


def require_editable_readback(
    element: dict[str, Any],
    expected: str,
    *,
    snapshot: dict[str, Any],
    label: str,
) -> None:
    text = element.get("text") or {}
    value = element.get("value")
    content = text.get("content")
    if value != expected or content != expected:
        raise RuntimeError(
            f"{label} did not expose expected editable readback {expected!r}.\n"
            f"element={json.dumps(element, indent=2, sort_keys=True)}\n"
            f"diagnostics={json.dumps(snapshot.get('diagnostics', []), indent=2, sort_keys=True)}"
        )
    if not element.get("supports_editable_text"):
        raise RuntimeError(
            f"{label} editable element did not advertise supports_editable_text.\n"
            f"element={json.dumps(element, indent=2, sort_keys=True)}"
        )


def wait_for_editable_readback(
    client: McpClient,
    title_hint: str,
    expected: str,
    *,
    deadline: float,
) -> tuple[dict[str, Any], dict[str, Any]]:
    last_snapshot: dict[str, Any] | None = None
    last_element: dict[str, Any] | None = None
    while time.time() < deadline:
        snapshot = wait_for_app_snapshot(client, title_hint, deadline=deadline)
        last_snapshot = snapshot
        try:
            element = find_editable(snapshot)
        except RuntimeError:
            time.sleep(0.1)
            continue
        last_element = element
        text = element.get("text") or {}
        if element.get("value") == expected and text.get("content") == expected:
            require_editable_readback(element, expected, snapshot=snapshot, label=title_hint)
            return snapshot, element
        time.sleep(0.1)
    raise RuntimeError(
        f"timed out waiting for {title_hint!r} editable readback {expected!r}.\n"
        f"element={json.dumps(last_element, indent=2, sort_keys=True)}\n"
        f"diagnostics={json.dumps((last_snapshot or {}).get('diagnostics', []), indent=2, sort_keys=True)}"
    )


def find_button(snapshot: dict[str, Any], label: str) -> dict[str, Any]:
    lowered = label.lower()
    for element in snapshot["elements"]:
        name = (element.get("name") or "").strip().lower()
        if name == lowered:
            return element
    raise RuntimeError(f"did not find a button named {label!r}")


def find_scroll_region(snapshot: dict[str, Any]) -> dict[str, Any]:
    candidates = [
        element
        for element in snapshot.get("elements", [])
        if (element.get("name") or "") == "Scroll region"
        and element.get("role") in {"panel", "scroll pane", "scroll_pane", "scrollbar"}
        and isinstance(element.get("element_index"), int)
    ]
    if candidates:
        return min(
            candidates,
            key=lambda element: (
                0 if element.get("role") in {"scroll pane", "scroll_pane"} else 1,
                float((element.get("bounds") or {}).get("height", float("inf"))),
                int(element["element_index"]),
            ),
        )
    raise RuntimeError(
        "did not find a unique scroll-region target element.\n"
        f"elements={json.dumps(snapshot.get('elements', []), indent=2, sort_keys=True)}"
    )


def run_zenity_input(
    title: str,
    *,
    initial_text: str | None = None,
    extra_env: dict[str, str] | None = None,
) -> subprocess.Popen[str]:
    env = dict(os.environ)
    env["NO_AT_BRIDGE"] = "0"
    if extra_env:
        env.update(extra_env)
    command = [
        "zenity",
        "--entry",
        f"--title={title}",
        "--text=sky-cua live smoke",
    ]
    if initial_text is not None:
        command.append(f"--entry-text={initial_text}")
    return subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )


def run_pointer_fixture(
    state_path: Path,
    *,
    extra_env: dict[str, str] | None = None,
) -> subprocess.Popen[str]:
    env = dict(os.environ)
    env["NO_AT_BRIDGE"] = "0"
    if extra_env:
        env.update(extra_env)
    return subprocess.Popen(
        [sys.executable, str(POINTER_FIXTURE), str(state_path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        cwd=REPO_ROOT,
        env=env,
    )


def load_state(state_path: Path) -> dict[str, Any] | None:
    if not state_path.exists():
        return None
    try:
        return json.loads(state_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None


def wait_for_state(
    state_path: Path,
    predicate: Callable[[dict[str, Any]], bool],
    *,
    deadline: float,
    description: str,
) -> dict[str, Any]:
    monotonic_deadline = time.monotonic() + max(0.0, deadline - time.time())
    sleep_seconds = 0.05
    last_state: dict[str, Any] | None = None
    while time.monotonic() < monotonic_deadline:
        state = load_state(state_path)
        last_state = state
        if state is not None and predicate(state):
            return state
        time.sleep(sleep_seconds)
        sleep_seconds = min(sleep_seconds * 1.5, 0.5)
    raise RuntimeError(
        f"timed out waiting for fixture state: {description}\n"
        f"last_state={json.dumps(last_state, indent=2, sort_keys=True)}"
    )


def wait_for_stable_pointer_fixture(state_path: Path, *, deadline: float) -> dict[str, Any]:
    monotonic_deadline = time.monotonic() + max(0.0, deadline - time.time())
    candidate: dict[str, Any] | None = None
    while time.monotonic() < monotonic_deadline:
        state = load_state(state_path)
        if state is None:
            time.sleep(0.15)
            continue
        width = int(state.get("window_size", {}).get("width", 0) or 0)
        height = int(state.get("window_size", {}).get("height", 0) or 0)
        if not state.get("ready") or width < 1000 or height < 700:
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
    raise RuntimeError(
        "timed out waiting for stable fullscreen pointer-fixture geometry\n"
        f"last_state={json.dumps(candidate, indent=2, sort_keys=True)}"
    )


def wait_for_x11_window_titles(
    titles: list[str],
    *,
    deadline: float,
    extra_env: dict[str, str] | None = None,
) -> None:
    env = dict(os.environ)
    if extra_env:
        env.update(extra_env)

    last_tree = ""
    lowered_titles = [title.lower() for title in titles]
    while time.time() < deadline:
        tree = subprocess.run(
            ["xwininfo", "-root", "-tree"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
            env=env,
        )
        if tree.returncode == 0:
            last_tree = tree.stdout
            lowered_tree = tree.stdout.lower()
            if all(title in lowered_tree for title in lowered_titles):
                return
        time.sleep(0.35)

    raise RuntimeError(
        "timed out waiting for X11 window titles to appear in xwininfo.\n"
        f"titles={json.dumps(titles, indent=2)}\n"
        f"last_tree_tail={last_tree[-4000:]}"
    )


def activate_x11_window(title: str, *, pid: int | None = None, deadline: float) -> None:
    if shutil.which("xdotool") is None:
        raise RuntimeError("xdotool is required to activate the X11 pointer fixture")

    last_search = ""
    while time.time() < deadline:
        search_command = ["xdotool", "search", "--onlyvisible"]
        if pid is not None:
            search_command.extend(["--pid", str(pid)])
        search_command.extend(["--name", title])
        search = subprocess.run(
            search_command,
            capture_output=True,
            text=True,
            check=False,
        )
        last_search = search.stderr.strip() or search.stdout.strip()
        window_ids = [line.strip() for line in search.stdout.splitlines() if line.strip()]
        if search.returncode == 0 and window_ids:
            activate = subprocess.run(
                ["xdotool", "windowactivate", "--sync", window_ids[-1]],
                capture_output=True,
                text=True,
                check=False,
                timeout=3,
            )
            if activate.returncode == 0:
                return
            last_search = activate.stderr.strip() or activate.stdout.strip()
        time.sleep(0.2)

    raise RuntimeError(f"timed out activating X11 window {title!r} with xdotool: {last_search}")


def pick_x11_click_target(snapshot: dict[str, Any]) -> dict[str, Any]:
    elements = snapshot.get("elements", [])
    parent_indices = {
        element.get("parent_index")
        for element in elements
        if element.get("parent_index") is not None
    }
    leaf_regions = [
        element
        for element in elements
        if element.get("role") in {"x11_leaf_region", "x11_action_region"}
        and element.get("element_index") not in parent_indices
        and element.get("bounds")
    ]
    if not leaf_regions:
        root_fallback = next(
            (
                element
                for element in elements
                if element.get("parent_index") is None
                and element.get("bounds")
                and "native_window_fallback" in (element.get("state_flags") or [])
            ),
            None,
        )
        if root_fallback is None:
            raise RuntimeError(
                "did not find a leaf x11_region element or actionable root fallback in the fallback snapshot.\n"
                f"elements={json.dumps(elements, indent=2, sort_keys=True)}"
            )
        return root_fallback

    def sort_key(element: dict[str, Any]) -> tuple[float, float, int]:
        bounds = element.get("bounds") or {}
        y = float(bounds.get("y", 0.0))
        height = float(bounds.get("height", 0.0))
        width = float(bounds.get("width", 0.0))
        center_y = y + (height / 2.0)
        area = width * height
        return (center_y, area, int(element.get("element_index", -1)))

    return max(leaf_regions, key=sort_key)


def x11_click_arguments(snapshot: dict[str, Any], target: dict[str, Any]) -> dict[str, Any]:
    if target.get("role") in {"x11_leaf_region", "x11_action_region"}:
        return {
            **appshot_action_fences(snapshot),
            "element_index": target["element_index"],
        }

    bounds = target.get("bounds") or {}
    width = float(bounds.get("width", 0.0))
    height = float(bounds.get("height", 0.0))
    if width <= 0 or height <= 0:
        raise RuntimeError(
            "X11 fallback root element did not include usable click bounds.\n"
            f"target={json.dumps(target, indent=2, sort_keys=True)}"
        )
    return {
        **appshot_action_fences(snapshot),
        "x": float(bounds.get("x", 0.0)) + (width / 2.0),
        "y": float(bounds.get("y", 0.0)) + (height * 0.76),
    }


def require_x11_action_region_hints(snapshot: dict[str, Any], label: str) -> None:
    elements = snapshot.get("elements", [])
    if len(elements) <= 1:
        raise RuntimeError(
            f"{label} fallback snapshot did not recover any child X11 regions beyond the root window.\n"
            f"elements={json.dumps(elements, indent=2, sort_keys=True)}"
        )
    if not any(element.get("role") == "x11_action_region" for element in elements):
        raise RuntimeError(
            f"{label} fallback snapshot did not surface any x11_action_region hints.\n"
            f"elements={json.dumps(elements, indent=2, sort_keys=True)}"
        )


def require_ok(result: dict[str, Any], action: str) -> None:
    if result.get("isError"):
        structured = result.get("structuredContent") or {}
        if structured.get("code") == "PortalApprovalPending":
            raise RuntimeError(
                f"{action} is waiting on portal approval. Approve the KDE portal dialog, then retry.\n"
                f"result={json.dumps(result, indent=2, sort_keys=True)}"
            )
        raise RuntimeError(f"{action} failed: {json.dumps(result, indent=2, sort_keys=True)}")


def require_no_pipewire_failure(snapshot: dict[str, Any], label: str) -> None:
    diagnostics = snapshot.get("diagnostics", [])
    if any(entry.get("code") == "PipeWireStreamFailed" for entry in diagnostics):
        raise RuntimeError(
            f"{label} snapshot fell back from PipeWire unexpectedly: "
            f"{json.dumps(diagnostics, indent=2, sort_keys=True)}"
        )


def require_live_wayland_image_backend(snapshot: dict[str, Any], label: str) -> None:
    image_backend = snapshot.get("image_backend")
    if image_backend != "portal_pipe_wire":
        raise RuntimeError(
            f"{label} snapshot did not keep PipeWire as the actual image backend.\n"
            f"capture_backend={snapshot.get('capture_backend')!r}\n"
            f"image_backend={image_backend!r}\n"
            f"diagnostics={json.dumps(snapshot.get('diagnostics', []), indent=2, sort_keys=True)}"
        )


def require_no_portal_approval_pending(snapshot: dict[str, Any], label: str) -> None:
    diagnostics = snapshot.get("diagnostics", [])
    pending = next(
        (entry for entry in diagnostics if entry.get("code") == "PortalApprovalPending"),
        None,
    )
    if pending is not None:
        raise RuntimeError(
            f"{label} is still waiting on portal approval. Approve the KDE dialog and re-run.\n"
            f"{json.dumps(pending, indent=2, sort_keys=True)}"
        )


def semantic_text_smoke(client: McpClient) -> None:
    dialog = run_zenity_input(ZENITY_TITLE, initial_text="stale-smoke")
    try:
        snapshot, editable = wait_for_editable_readback(
            client, ZENITY_TITLE, "stale-smoke", deadline=time.time() + 30
        )
        require_no_portal_approval_pending(snapshot, "initial zenity snapshot")
        require_no_pipewire_failure(snapshot, "initial zenity")
        require_live_wayland_image_backend(snapshot, "initial zenity")
        ok_button = find_button(snapshot, "OK")

        print(f"Focused app: {snapshot.get('focused_app')}")
        print(f"Editable element index: {editable['element_index']}")
        print(f"OK button index: {ok_button['element_index']}")
        print(f"Capture: {snapshot.get('capture')}")

        require_ok(
            client.tools_call(
                20,
                "desktop_set_value",
                {
                    **appshot_action_fences(snapshot),
                    "element_index": editable["element_index"],
                    "value": "smoke-value",
                },
            ),
            "set_value",
        )
        updated_snapshot, _updated_editable = wait_for_editable_readback(
            client, ZENITY_TITLE, "smoke-value", deadline=time.time() + 10
        )
        ok_button = find_button(updated_snapshot, "OK")
        require_ok(
            client.tools_call(
                21,
                "desktop_pointer",
                {
                    "operation": "click",
                    **appshot_action_fences(updated_snapshot),
                    "element_index": ok_button["element_index"],
                },
            ),
            "click",
        )

        stdout, stderr = dialog.communicate(timeout=10)
        if dialog.returncode != 0:
            raise RuntimeError(
                f"zenity entry box exited with {dialog.returncode}\nstdout={stdout!r}\nstderr={stderr!r}"
            )
        if stdout.strip() != "smoke-value":
            raise RuntimeError(f"expected zenity to return 'smoke-value', got {stdout.strip()!r}")
        print("set_value + semantic click smoke passed.")
    finally:
        if dialog.poll() is None:
            dialog.terminate()

    enter_dialog = run_zenity_input(f"{ZENITY_TITLE} enter")
    try:
        snapshot = wait_for_app_snapshot(client, f"{ZENITY_TITLE} enter", deadline=time.time() + 30)
        require_no_portal_approval_pending(snapshot, "enter zenity snapshot")
        require_no_pipewire_failure(snapshot, "enter zenity")
        editable = find_editable(snapshot)

        require_ok(
            client.tools_call(
                30,
                "desktop_pointer",
                {
                    "operation": "click",
                    **appshot_action_fences(snapshot),
                    "element_index": editable["element_index"],
                },
            ),
            "focus click before type_text",
        )
        require_ok(
            client.tools_call(
                31,
                "desktop_keyboard",
                {
                    "operation": "type_text",
                    **appshot_action_fences(snapshot),
                    "text": "typed-smoke",
                },
            ),
            "type_text",
        )
        typed_snapshot, _typed_editable = wait_for_editable_readback(
            client,
            f"{ZENITY_TITLE} enter",
            "typed-smoke",
            deadline=time.time() + 10,
        )
        require_ok(
            client.tools_call(
                32,
                "desktop_keyboard",
                {
                    "operation": "press_key",
                    **appshot_action_fences(typed_snapshot),
                    "key": "Enter",
                },
            ),
            "press_key",
        )

        stdout, stderr = enter_dialog.communicate(timeout=10)
        if enter_dialog.returncode != 0:
            raise RuntimeError(
                f"zenity enter box exited with {enter_dialog.returncode}\nstdout={stdout!r}\nstderr={stderr!r}"
            )
        if stdout.strip() != "typed-smoke":
            raise RuntimeError(f"expected zenity to return 'typed-smoke', got {stdout.strip()!r}")
        print("type_text + press_key smoke passed.")
    finally:
        if enter_dialog.poll() is None:
            enter_dialog.terminate()


def service_scroll_at(
    socket_path: Path,
    appshot_id: str,
    x: float,
    y: float,
) -> dict[str, Any]:
    request = {
        "type": "scroll",
        "context": {
            "session_id": "live-xpra-pointer-smoke",
            "turn_id": f"scroll-{time.time_ns()}",
            "appshot_id": appshot_id,
            "deadline_ms": 10_000,
        },
        "direction": "down",
        "pixels": 120,
        "x": x,
        "y": y,
        "post_action_sleep_ms": 0,
    }
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
        stream.settimeout(15.0)
        stream.connect(str(socket_path))
        stream.sendall(json.dumps(request).encode("utf-8") + b"\n")
        response = b""
        while b"\n" not in response:
            chunk = stream.recv(65_536)
            if not chunk:
                break
            response += chunk
    raw = response.partition(b"\n")[0].strip()
    if not raw:
        raise RuntimeError(f"empty CUA scroll response from {socket_path}")
    parsed = json.loads(raw)
    if parsed.get("type") != "scroll" or not parsed.get("ok"):
        raise RuntimeError(f"CUA scroll failed: {json.dumps(parsed, indent=2, sort_keys=True)}")
    return parsed


def physical_pointer_smoke(
    client: McpClient,
    *,
    xwayland: bool = False,
    raw_scroll_socket: Path | None = None,
) -> None:
    backend_label = "XWayland" if xwayland else "Wayland"
    if xwayland:
        if shutil.which("xdpyinfo") is None:
            print("Skipping XWayland pointer smoke: xdpyinfo is not installed.")
            return
        display_check = subprocess.run(
            ["xdpyinfo"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if display_check.returncode != 0:
            print("Skipping XWayland pointer smoke: no usable X11 display is available.")
            return

    with tempfile.TemporaryDirectory(
        prefix=f"sky-cua-pointer-smoke-{'xwayland' if xwayland else 'wayland'}-"
    ) as tmpdir:
        state_path = Path(tmpdir) / "state.json"
        fixture = run_pointer_fixture(
            state_path,
            extra_env={"GDK_BACKEND": "x11"} if xwayland else None,
        )
        try:
            if xwayland:
                activate_x11_window(
                    POINTER_TITLE,
                    pid=fixture.pid,
                    deadline=time.time() + 10,
                )
            state = wait_for_stable_pointer_fixture(state_path, deadline=time.time() + 20)
            print(
                f"{backend_label} pointer fixture ready: "
                f"{json.dumps(state['points'], sort_keys=True)}"
            )
            apps = grouped_structured_result(
                client.tools_call(39, "list_resources", {"surface": "desktop", "resource": "apps"})
            ).get("apps", [])
            matching_apps = [
                app
                for app in apps
                if POINTER_TITLE.lower() in ((app.get("window_title") or "").lower())
            ]
            fixture_visible = bool(matching_apps)
            print(f"{backend_label} pointer fixture visible in app list: {fixture_visible}")

            selector: dict[str, Any] = {"window_title": POINTER_TITLE}
            if xwayland:
                windows = grouped_structured_result(
                    client.tools_call(
                        39,
                        "list_resources",
                        {"surface": "desktop", "resource": "windows"},
                    )
                ).get("windows", [])
                x11_window = next(
                    (
                        window
                        for window in windows
                        if window.get("title") == POINTER_TITLE
                        and window.get("backend") == "x11"
                        and isinstance(window.get("window_id"), str)
                    ),
                    None,
                )
                if x11_window is None:
                    raise RuntimeError(
                        "GTK pointer fixture launched with GDK_BACKEND=x11 but no exact "
                        "XWayland window handle was exposed.\n"
                        f"windows={json.dumps(windows, indent=2, sort_keys=True)}"
                    )
                selector = {"window_id": x11_window["window_id"]}

            def action_point(point: dict[str, Any], appshot: dict[str, Any]) -> dict[str, float]:
                if not xwayland:
                    return {"x": float(point["x"]), "y": float(point["y"])}
                bounds = appshot.get("bounds") or {}
                window_size = state.get("window_size") or {}
                width = float(window_size.get("width") or 0)
                height = float(window_size.get("height") or 0)
                if (
                    width <= 0
                    or height <= 0
                    or float(bounds.get("width") or 0) <= 0
                    or float(bounds.get("height") or 0) <= 0
                ):
                    raise RuntimeError(
                        "XWayland pointer smoke could not map fixture physical pixels "
                        "to AppShot desktop-logical bounds.\n"
                        f"window_size={window_size!r} bounds={bounds!r}"
                    )
                return {
                    "x": float(bounds.get("x") or 0)
                    + float(point["x"]) * float(bounds["width"]) / width,
                    "y": float(bounds.get("y") or 0)
                    + float(point["y"]) * float(bounds["height"]) / height,
                }

            fixture_appshot = wait_for_window_snapshot(client, selector, deadline=time.time() + 10)
            if xwayland:
                print(
                    "XWayland AppShot bounds: "
                    f"{json.dumps(fixture_appshot.get('bounds'), sort_keys=True)}"
                )
            click_point = action_point(state["points"]["click_button"], fixture_appshot)
            require_ok(
                client.tools_call(
                    40,
                    "desktop_pointer",
                    {
                        "operation": "click",
                        "appshot_id": fixture_appshot["appshot_id"],
                        "x": click_point["x"],
                        "y": click_point["y"],
                    },
                ),
                "physical click",
            )
            wait_for_state(
                state_path,
                lambda current: bool(current.get("clicked")),
                deadline=time.time() + 8,
                description="physical click acknowledgement",
            )
            print(f"{backend_label} physical click smoke passed.")

            fixture_appshot = wait_for_window_snapshot(client, selector, deadline=time.time() + 10)
            secondary_point = action_point(state["points"]["secondary"], fixture_appshot)
            require_ok(
                client.tools_call(
                    41,
                    "desktop_pointer",
                    {
                        "operation": "secondary_click",
                        "appshot_id": fixture_appshot["appshot_id"],
                        "x": secondary_point["x"],
                        "y": secondary_point["y"],
                    },
                ),
                "physical secondary click",
            )
            wait_for_state(
                state_path,
                lambda current: bool(current.get("secondary_clicked")),
                deadline=time.time() + 8,
                description="secondary click acknowledgement",
            )
            print(f"{backend_label} physical secondary-click smoke passed.")

            fixture_appshot = wait_for_window_snapshot(client, selector, deadline=time.time() + 10)
            drag_from = action_point(state["points"]["drag_from"], fixture_appshot)
            drag_to = action_point(state["points"]["drag_to"], fixture_appshot)
            require_ok(
                client.tools_call(
                    42,
                    "desktop_pointer",
                    {
                        "operation": "drag",
                        "appshot_id": fixture_appshot["appshot_id"],
                        "x": drag_from["x"],
                        "y": drag_from["y"],
                        "to_x": drag_to["x"],
                        "to_y": drag_to["y"],
                    },
                ),
                "physical drag",
            )
            wait_for_state(
                state_path,
                lambda current: bool(current.get("drag_completed")),
                deadline=time.time() + 8,
                description="drag acknowledgement",
            )
            print(f"{backend_label} physical drag smoke passed.")

            observe_result = client.tools_call(
                43,
                "observe",
                {
                    "surface": "desktop",
                    **selector,
                    "detail": "full",
                    "element_query": "Scroll region",
                    "element_limit": 20,
                },
            )
            require_ok(observe_result, "pre-scroll observation")
            fixture_snapshot = grouped_structured_result(observe_result)
            pointer_state = load_state(state_path) or {}
            starting_scrolls = int(pointer_state.get("scroll_events", 0))
            scroll_region: dict[str, Any] | None = None
            if fixture_snapshot.get("elements"):
                scroll_region = find_scroll_region(fixture_snapshot)
                print(f"{backend_label} scroll target: {json.dumps(scroll_region, sort_keys=True)}")
                scroll_result = client.tools_call(
                    44,
                    "desktop_scroll",
                    {
                        "direction": "down",
                        "pages": 1,
                        **appshot_action_fences(fixture_snapshot),
                        "element_index": scroll_region["element_index"],
                    },
                )
                require_ok(scroll_result, "physical scroll")
                print(
                    f"{backend_label} scroll result: "
                    f"{json.dumps(grouped_structured_result(scroll_result), sort_keys=True)}"
                )
            elif raw_scroll_socket is not None:
                raw_scroll_point = action_point(state["points"]["scroll"], fixture_snapshot)
                scroll_result = service_scroll_at(
                    raw_scroll_socket,
                    str(fixture_snapshot["appshot_id"]),
                    raw_scroll_point["x"],
                    raw_scroll_point["y"],
                )
                print(
                    f"{backend_label} raw CUA scroll result: "
                    f"{json.dumps(scroll_result, sort_keys=True)}"
                )
            else:
                raise RuntimeError(
                    "pointer fixture exposed no accessibility elements and no raw CUA "
                    "scroll socket was supplied"
                )
            wait_for_state(
                state_path,
                lambda current: int(current.get("scroll_events", 0)) > starting_scrolls,
                deadline=time.time() + 8,
                description="scroll acknowledgement",
            )
            time.sleep(0.35)
            settled_scrolls = int((load_state(state_path) or {}).get("scroll_events", 0))
            if settled_scrolls != starting_scrolls + 1:
                raise RuntimeError(
                    f"{backend_label} one-page scroll produced "
                    f"{settled_scrolls - starting_scrolls} fixture scroll events"
                )
            print(f"{backend_label} physical scroll smoke passed.")

            if scroll_region is not None and xwayland:
                horizontal_observe_result = client.tools_call(
                    45,
                    "observe",
                    {
                        "surface": "desktop",
                        **selector,
                        "detail": "full",
                        "element_query": "Scroll region",
                        "element_limit": 20,
                    },
                )
                require_ok(horizontal_observe_result, "pre-horizontal-scroll observation")
                horizontal_snapshot = grouped_structured_result(horizontal_observe_result)
                horizontal_scroll_region = find_scroll_region(horizontal_snapshot)
                starting_horizontal_scrolls = int(
                    (load_state(state_path) or {}).get("horizontal_scroll_events", 0)
                )
                horizontal_scroll_result = client.tools_call(
                    46,
                    "desktop_scroll",
                    {
                        "direction": "right",
                        "pages": 1,
                        **appshot_action_fences(horizontal_snapshot),
                        "element_index": horizontal_scroll_region["element_index"],
                    },
                )
                require_ok(horizontal_scroll_result, "physical horizontal scroll")
                print(
                    f"{backend_label} horizontal scroll result: "
                    f"{json.dumps(grouped_structured_result(horizontal_scroll_result), sort_keys=True)}"
                )
                wait_for_state(
                    state_path,
                    lambda current: (
                        int(current.get("horizontal_scroll_events", 0))
                        > starting_horizontal_scrolls
                    ),
                    deadline=time.time() + 8,
                    description="horizontal scroll acknowledgement",
                )
                time.sleep(0.35)
                settled_horizontal_scrolls = int(
                    (load_state(state_path) or {}).get("horizontal_scroll_events", 0)
                )
                if settled_horizontal_scrolls != starting_horizontal_scrolls + 1:
                    raise RuntimeError(
                        f"{backend_label} one-page horizontal scroll produced "
                        f"{settled_horizontal_scrolls - starting_horizontal_scrolls} "
                        "fixture scroll events"
                    )
                print(f"{backend_label} physical horizontal scroll smoke passed.")
        finally:
            if fixture.poll() is None:
                fixture.terminate()
                try:
                    fixture.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    fixture.kill()
            stderr = fixture.stderr.read() if fixture.stderr is not None else ""
            if stderr.strip():
                print("Pointer fixture stderr:", stderr.strip(), file=sys.stderr)


def main() -> int:
    print("Starting live desktop smoke.")
    print("If KDE shows a portal approval prompt, approve it so the test can continue.\n")

    client = McpClient([str(CLIENT), "mcp"], read_timeout=35)
    try:
        client.initialize()
        tools = {tool["name"] for tool in client.tools_list()}
        required_tools = {
            "desktop_keyboard",
            "desktop_pointer",
            "desktop_scroll",
            "desktop_set_value",
            "list_resources",
            "observe",
        }
        missing = sorted(required_tools - tools)
        if missing:
            raise RuntimeError(f"MCP server did not advertise required tools: {missing}")

        runtime_dir = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
        service_socket_path = Path(
            os.environ.get(
                "SKY_CUA_SERVICE_SOCKET_PATH",
                str(runtime_dir / "sky-cua" / "service.sock"),
            )
        )
        semantic_text_smoke(client)
        physical_pointer_smoke(client, raw_scroll_socket=service_socket_path)
        physical_pointer_smoke(
            client,
            xwayland=True,
            raw_scroll_socket=service_socket_path,
        )

        apps = grouped_structured_result(
            client.tools_call(50, "list_resources", {"surface": "desktop", "resource": "apps"})
        ).get("apps", [])
        print(f"list_resources desktop/apps returned {len(apps)} apps.")
        print("\nLive desktop smoke completed successfully.")
        return 0
    finally:
        client.close()


if __name__ == "__main__":
    raise SystemExit(main())
