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
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
CLIENT = REPO_ROOT / "bin" / "sky-cua-client"
POINTER_FIXTURE = REPO_ROOT / "scripts" / "gtk_pointer_smoke_fixture.py"
ZENITY_TITLE = "sky-cua zenity smoke"
POINTER_TITLE = "sky-cua pointer smoke"


@dataclass
class McpResponse:
    raw: dict[str, Any]

    @property
    def result(self) -> dict[str, Any]:
        if "result" not in self.raw:
            raise RuntimeError(
                "MCP call did not return a result payload.\n"
                f"response={json.dumps(self.raw, indent=2, sort_keys=True)}"
            )
        return self.raw["result"]


class McpClient:
    def __init__(self, argv: list[str], *, extra_env: dict[str, str] | None = None) -> None:
        env = dict(os.environ)
        env.setdefault("SKY_CUA_REPO_ROOT", str(REPO_ROOT))
        if extra_env:
            env.update(extra_env)
        self.proc = subprocess.Popen(
            argv,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=False,
            cwd=REPO_ROOT,
            env=env,
        )

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def initialize(self) -> None:
        self.call_raw(
            1,
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "live-desktop-smoke", "version": "0.2.0"},
            },
        )
        self.notify("notifications/initialized", {})

    def tools_list(self) -> list[dict[str, Any]]:
        response = self.call_raw(2, "tools/list", {})
        return response.result["tools"]

    def tools_call(self, request_id: int, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        response = self.call_raw(
            request_id,
            "tools/call",
            {"name": name, "arguments": arguments},
        )
        return response.result

    def notify(self, method: str, params: dict[str, Any]) -> None:
        payload = {"jsonrpc": "2.0", "method": method, "params": params}
        self._write_message(payload)

    def call_raw(self, request_id: int, method: str, params: dict[str, Any]) -> McpResponse:
        payload = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        self._write_message(payload)
        return McpResponse(self._read_message())

    def _write_message(self, payload: dict[str, Any]) -> None:
        encoded = json.dumps(payload).encode("utf-8")
        message = f"Content-Length: {len(encoded)}\r\n\r\n".encode("ascii") + encoded
        assert self.proc.stdin is not None
        self.proc.stdin.write(message)
        self.proc.stdin.flush()

    def _read_message(self) -> dict[str, Any]:
        assert self.proc.stdout is not None
        headers = {}
        while True:
            line = self.proc.stdout.readline()
            if not line:
                stderr = b""
                if self.proc.stderr is not None:
                    stderr = self.proc.stderr.read() or b""
                raise RuntimeError(
                    f"MCP client exited unexpectedly.\nstderr:\n{stderr.decode(errors='replace')}"
                )
            if line == b"\r\n":
                break
            name, _, value = line.decode("ascii").partition(":")
            headers[name.strip().lower()] = value.strip()
        length = int(headers["content-length"])
        body = self.proc.stdout.read(length)
        return json.loads(body.decode("utf-8"))


def wait_for_app_snapshot(client: McpClient, title_hint: str, deadline: float) -> dict[str, Any]:
    return wait_for_app_snapshot_result(client, title_hint, deadline=deadline)["structuredContent"]


def wait_for_app_snapshot_result(
    client: McpClient, title_hint: str, *, deadline: float
) -> dict[str, Any]:
    request_id = 10
    lowered = title_hint.lower()
    while time.time() < deadline:
        apps_result = client.tools_call(request_id, "list_apps", {})
        request_id += 1
        apps = apps_result["structuredContent"]["apps"]
        matching_app = next(
            (app for app in apps if lowered in ((app.get("window_title") or "").lower())),
            None,
        )
        if matching_app is not None:
            result = client.tools_call(
                request_id,
                "get_app_state",
                {"app_id": matching_app["app_id"]},
            )
            request_id += 1
            return result
        time.sleep(0.5)
    raise RuntimeError(f"timed out waiting for an app with title containing {title_hint!r}")


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


def find_button(snapshot: dict[str, Any], label: str) -> dict[str, Any]:
    lowered = label.lower()
    for element in snapshot["elements"]:
        name = (element.get("name") or "").strip().lower()
        if name == lowered:
            return element
    raise RuntimeError(f"did not find a button named {label!r}")


def run_zenity_input(
    title: str,
    *,
    initial_text: str | None = None,
    extra_env: dict[str, str] | None = None,
) -> subprocess.Popen[str]:
    env = dict(os.environ)
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
    while time.time() < deadline:
        state = load_state(state_path)
        if state is not None and predicate(state):
            return state
        time.sleep(0.15)
    raise RuntimeError(f"timed out waiting for fixture state: {description}")


def wait_for_stable_pointer_fixture(state_path: Path, *, deadline: float) -> dict[str, Any]:
    candidate: dict[str, Any] | None = None
    while time.time() < deadline:
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
    raise RuntimeError("timed out waiting for stable fullscreen pointer-fixture geometry")


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
            "snapshot_id": snapshot["snapshot_id"],
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
    capture = snapshot.get("capture") or {}
    image_backend = capture.get("image_backend")
    if image_backend != "portal_pipe_wire":
        raise RuntimeError(
            f"{label} snapshot did not keep PipeWire as the actual image backend.\n"
            f"capture={json.dumps(capture, indent=2, sort_keys=True)}\n"
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
        snapshot = wait_for_app_snapshot(client, ZENITY_TITLE, deadline=time.time() + 30)
        require_no_portal_approval_pending(snapshot, "initial zenity snapshot")
        require_no_pipewire_failure(snapshot, "initial zenity")
        require_live_wayland_image_backend(snapshot, "initial zenity")
        editable = find_editable(snapshot)
        ok_button = find_button(snapshot, "OK")
        require_editable_readback(
            editable,
            "stale-smoke",
            snapshot=snapshot,
            label="initial zenity snapshot",
        )

        print(f"Focused app: {snapshot.get('focused_app')}")
        print(f"Editable element index: {editable['element_index']}")
        print(f"OK button index: {ok_button['element_index']}")
        print(f"Capture: {snapshot.get('capture')}")

        require_ok(
            client.tools_call(
                20,
                "set_value",
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "element_index": editable["element_index"],
                    "value": "smoke-value",
                },
            ),
            "set_value",
        )
        updated_snapshot = wait_for_app_snapshot(
            client, ZENITY_TITLE, deadline=time.time() + 10
        )
        updated_editable = find_editable(updated_snapshot)
        require_editable_readback(
            updated_editable,
            "smoke-value",
            snapshot=updated_snapshot,
            label="post-set_value zenity snapshot",
        )
        ok_button = find_button(updated_snapshot, "OK")
        require_ok(
            client.tools_call(
                21,
                "click",
                {
                    "snapshot_id": updated_snapshot["snapshot_id"],
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
                "click",
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "element_index": editable["element_index"],
                },
            ),
            "focus click before type_text",
        )
        require_ok(
            client.tools_call(
                31,
                "type_text",
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "element_index": editable["element_index"],
                    "text": "typed-smoke",
                },
            ),
            "type_text",
        )
        typed_snapshot = wait_for_app_snapshot(
            client, f"{ZENITY_TITLE} enter", deadline=time.time() + 10
        )
        typed_editable = find_editable(typed_snapshot)
        require_editable_readback(
            typed_editable,
            "typed-smoke",
            snapshot=typed_snapshot,
            label="post-type_text zenity snapshot",
        )
        require_ok(
            client.tools_call(
                32,
                "press_key",
                {
                    "snapshot_id": typed_snapshot["snapshot_id"],
                    "element_index": typed_editable["element_index"],
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


def physical_pointer_smoke(client: McpClient) -> None:
    with tempfile.TemporaryDirectory(prefix="sky-cua-pointer-smoke-") as tmpdir:
        state_path = Path(tmpdir) / "state.json"
        fixture = run_pointer_fixture(state_path)
        try:
            state = wait_for_stable_pointer_fixture(state_path, deadline=time.time() + 20)
            print(f"Pointer fixture ready: {json.dumps(state['points'], sort_keys=True)}")
            apps = client.tools_call(39, "list_apps", {})["structuredContent"]["apps"]
            fixture_visible = any(
                POINTER_TITLE.lower() in ((app.get("window_title") or "").lower()) for app in apps
            )
            print(f"Pointer fixture visible in AT-SPI app list: {fixture_visible}")

            click_point = state["points"]["click_button"]
            require_ok(
                client.tools_call(
                    40,
                    "click",
                    {"x": click_point["x"], "y": click_point["y"]},
                ),
                "physical click",
            )
            wait_for_state(
                state_path,
                lambda current: bool(current.get("clicked")),
                deadline=time.time() + 8,
                description="physical click acknowledgement",
            )
            print("physical click smoke passed.")

            secondary_point = state["points"]["secondary"]
            require_ok(
                client.tools_call(
                    41,
                    "perform_secondary_action",
                    {"x": secondary_point["x"], "y": secondary_point["y"]},
                ),
                "physical secondary click",
            )
            wait_for_state(
                state_path,
                lambda current: bool(current.get("secondary_clicked")),
                deadline=time.time() + 8,
                description="secondary click acknowledgement",
            )
            print("physical secondary-click smoke passed.")

            drag_from = state["points"]["drag_from"]
            drag_to = state["points"]["drag_to"]
            require_ok(
                client.tools_call(
                    42,
                    "drag",
                    {
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
            print("physical drag smoke passed.")

            scroll_point = state["points"]["scroll"]
            pointer_state = load_state(state_path) or {}
            starting_scrolls = int(pointer_state.get("scroll_events", 0))
            require_ok(
                client.tools_call(
                    43,
                    "scroll",
                    {"x": scroll_point["x"], "y": scroll_point["y"], "delta_y": -180.0},
                ),
                "physical scroll",
            )
            wait_for_state(
                state_path,
                lambda current: int(current.get("scroll_events", 0)) > starting_scrolls,
                deadline=time.time() + 8,
                description="scroll acknowledgement",
            )
            print("physical scroll smoke passed.")
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


def xwayland_visibility_probe(client: McpClient) -> None:
    if shutil.which("xmessage") is None:
        print("Skipping XWayland visibility probe: xmessage is not installed.")
        return
    if shutil.which("xdpyinfo") is None:
        print("Skipping XWayland visibility probe: xdpyinfo is not installed.")
        return

    display_check = subprocess.run(
        ["xdpyinfo"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if display_check.returncode != 0:
        print("Skipping XWayland visibility probe: no usable X11 display is available.")
        return

    title = "sky-cua xmessage probe"
    probe = subprocess.Popen(
        ["xmessage", "-title", title, "-buttons", "OK:0", "-center", "sky-cua x11 body"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        deadline = time.time() + 12
        request_id = 60
        seen: dict[str, Any] | None = None
        last_sample: list[dict[str, Any]] = []
        lowered = title.lower()

        while time.time() < deadline:
            apps = client.tools_call(request_id, "list_apps", {})["structuredContent"]["apps"]
            request_id += 1
            last_sample = [
                {
                    "name": app.get("name"),
                    "window_title": app.get("window_title"),
                    "desktop_file_id": app.get("desktop_file_id"),
                }
                for app in apps
            ]
            matching = [app for app in apps if lowered in ((app.get("window_title") or "").lower())]
            seen = max(
                matching,
                key=lambda app: (
                    bool(app.get("is_focused_candidate")),
                    str(app.get("app_id") or "").startswith("x11:"),
                    app.get("desktop_file_id") == "xmessage.desktop",
                ),
                default=None,
            )
            if seen is not None:
                break
            time.sleep(0.5)

        if seen is None:
            raise RuntimeError(
                "XWayland xmessage probe did not appear in list_apps.\n"
                f"Sample:\n{json.dumps(last_sample, indent=2, sort_keys=True)}"
            )

        snapshot = client.tools_call(
            request_id,
            "get_app_state",
            {"app_id": seen["app_id"]},
        )["structuredContent"]
        request_id += 1
        focused_snapshot = client.tools_call(request_id, "get_app_state", {})["structuredContent"]
        print("XWayland xmessage probe visible in AT-SPI app list: True")
        print(f"XWayland focused app: {snapshot.get('focused_app')}")
        print(f"XWayland element count: {len(snapshot.get('elements', []))}")
        focused_app = snapshot.get("focused_app") or {}
        if focused_app.get("app_id") != seen.get("app_id"):
            raise RuntimeError(
                "XWayland get_app_state did not stay on the selected window.\n"
                f"selected={json.dumps(seen, indent=2, sort_keys=True)}\n"
                f"focused={json.dumps(focused_app, indent=2, sort_keys=True)}"
            )
        default_focused_app = focused_snapshot.get("focused_app") or {}
        if default_focused_app.get("app_id") != seen.get("app_id"):
            raise RuntimeError(
                "XWayland get_app_state without a selector did not follow the focused X11 window.\n"
                f"selected={json.dumps(seen, indent=2, sort_keys=True)}\n"
                f"default_focused={json.dumps(default_focused_app, indent=2, sort_keys=True)}"
            )
        snapshot = focused_snapshot
        if shutil.which("xwininfo") is not None:
            require_x11_action_region_hints(snapshot, "XWayland")
        descendant_region = pick_x11_click_target(snapshot)
        click_result = client.tools_call(
            request_id,
            "click",
            x11_click_arguments(snapshot, descendant_region),
        )
        require_ok(click_result, "XWayland descendant-region click")
        click_message = (click_result.get("structuredContent") or {}).get("message")
        if click_message:
            print(f"XWayland click result: {click_message}")
        request_id += 1
        if descendant_region.get("role") in {"x11_leaf_region", "x11_action_region"}:
            try:
                probe.wait(timeout=4)
            except subprocess.TimeoutExpired as exc:
                raise RuntimeError(
                    "Clicking the recovered XWayland descendant region did not dismiss xmessage.\n"
                    f"target={json.dumps(descendant_region, indent=2, sort_keys=True)}"
                ) from exc
        else:
            print(
                "XWayland fallback root click was sent, but dismissal is only required "
                "when child action-region hints are available."
            )
        print(
            "XWayland fallback click smoke passed."
            if descendant_region.get("role") in {"x11_leaf_region", "x11_action_region"}
            else "XWayland root fallback transport probe passed."
        )
        if shutil.which("xwininfo") is not None:
            elements = snapshot.get("elements", [])
            bounds = elements[0].get("bounds") or {}
            if bounds.get("width", 0) <= 0 or bounds.get("height", 0) <= 0:
                raise RuntimeError(
                    "XWayland fallback root element did not include usable bounds.\n"
                    f"element={json.dumps(elements[0], indent=2, sort_keys=True)}"
                )

            selector_alpha_title = "sky-cua selector alpha"
            selector_beta_title = "sky-cua selector beta"
            alpha = subprocess.Popen(
                [
                    "xmessage",
                    "-title",
                    selector_alpha_title,
                    "-buttons",
                    "OK:0",
                    "-center",
                    "sky-cua selector alpha body",
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            beta = subprocess.Popen(
                [
                    "xmessage",
                    "-title",
                    selector_beta_title,
                    "-buttons",
                    "OK:0",
                    "-center",
                    "sky-cua selector beta body",
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            try:
                wait_for_x11_window_titles(
                    [selector_alpha_title, selector_beta_title],
                    deadline=time.time() + 12,
                )

                selector_snapshot = client.tools_call(
                    request_id,
                    "get_app_state",
                    {
                        "desktop_file_id": "xmessage.desktop",
                        "window_title": selector_beta_title,
                    },
                )["structuredContent"]
                selector_focused = selector_snapshot.get("focused_app") or {}
                if selector_focused.get("window_title") != selector_beta_title:
                    raise RuntimeError(
                        "XWayland selector did not choose the exact title-matched xmessage window.\n"
                        f"focused={json.dumps(selector_focused, indent=2, sort_keys=True)}"
                    )
                print("XWayland selector probe passed.")
            finally:
                for process in (alpha, beta):
                    if process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            process.kill()
    finally:
        if probe.poll() is None:
            probe.terminate()
            try:
                probe.wait(timeout=5)
            except subprocess.TimeoutExpired:
                probe.kill()


def main() -> int:
    print("Starting live desktop smoke.")
    print("If KDE shows a portal approval prompt, approve it so the test can continue.\n")

    client = McpClient([str(CLIENT), "mcp"])
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

        semantic_text_smoke(client)
        physical_pointer_smoke(client)
        xwayland_visibility_probe(client)

        apps = client.tools_call(50, "list_apps", {})["structuredContent"]["apps"]
        print(f"list_apps returned {len(apps)} apps.")
        print("\nLive KDE smoke completed successfully.")
        return 0
    finally:
        client.close()


if __name__ == "__main__":
    raise SystemExit(main())
