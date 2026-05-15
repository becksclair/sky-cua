#!/usr/bin/env python3
"""X11 live smoke for the agent cursor overlay host.

Target app: a tiny Tk window on a private or current X11 display.
Artifacts: artifacts/codex-e2e/agent-cursor-x11-overlay/<timestamp>/
Proof: the X11 shaped-window backend draws the Chrome cursor asset into an
X11 root capture, hides it on request, re-shows before click-through, and
remains click-through to the target window underneath.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
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
ARTIFACT_ROOT = REPO_ROOT / "artifacts" / "codex-e2e" / "agent-cursor-x11-overlay"
OVERLAY_HOST_BIN = REPO_ROOT / "target" / "debug" / "sky-cua-overlay-host"

TARGET_TITLE = "sky-cua overlay click-through target"
TARGET_GEOMETRY = "420x300+120+90"
TARGET_CLICK_POINT = (330.0, 240.0)

TK_FIXTURE_CODE = r"""
import json
import os
import tkinter as tk
from pathlib import Path

state_path = Path(os.environ["SKY_CUA_X11_OVERLAY_STATE"])
root = tk.Tk()
root.title("sky-cua overlay click-through target")
root.geometry("420x300+120+90")
root.configure(bg="#245a8d")
label = tk.Label(
    root,
    text="click-through target",
    bg="#245a8d",
    fg="white",
    font=("Sans", 24),
)
label.pack(expand=True, fill="both")
root.update_idletasks()


def write_state(*, clicked=False, event=None):
    payload = {
        "ready": True,
        "clicked": clicked,
        "geometry": root.geometry(),
        "title": root.title(),
    }
    if event is not None:
        payload["event"] = {
            "x_root": event.x_root,
            "y_root": event.y_root,
            "x": event.x,
            "y": event.y,
        }
    state_path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")


def on_click(event):
    write_state(clicked=True, event=event)
    root.after(100, root.destroy)


root.bind_all("<Button-1>", on_click)
write_state()
root.mainloop()
"""


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


def main() -> int:
    parser = argparse.ArgumentParser(description="Live smoke for the X11 agent cursor overlay.")
    display_group = parser.add_mutually_exclusive_group()
    display_group.add_argument(
        "--current-display",
        action="store_true",
        help=(
            "Run against the current DISPLAY. Requires a real X11 desktop session "
            "and refuses Wayland/XWayland."
        ),
    )
    args = parser.parse_args()

    try:
        current_display = require_current_x11_display() if args.current_display else None
    except RuntimeError as exc:
        raise SystemExit(str(exc)) from None
    require_installed("xdpyinfo")
    require_installed("xdotool")
    require_installed("import")
    build_overlay_host()

    artifact_dir = ARTIFACT_ROOT / datetime.now(UTC).strftime("%Y%m%dT%H%M%S%fZ")
    artifact_dir.mkdir(parents=True, exist_ok=True)
    display_mode = display_mode_for_args(args)
    summary: dict[str, Any] = {"mode": display_mode, "artifact_dir": str(artifact_dir)}

    with tempfile.TemporaryDirectory(prefix="sky-cua-x11-overlay-") as tmpdir:
        base_dir = Path(tmpdir)
        runtime_dir = make_runtime_dir(base_dir)
        state_path = base_dir / "target-state.json"
        overlay: OverlayHostProcess | None = None
        target: subprocess.Popen[str] | None = None
        try:
            if current_display is None:
                raise RuntimeError("--current-display did not resolve a display")
            display = current_display
            wait_for_x11_display(display, deadline=time.time() + 8)
            env = x11_overlay_env(display, runtime_dir)
            target = start_target_window(env, state_path)
            target_state = wait_for_target_state(state_path, deadline=time.time() + 8)

            before_path = artifact_dir / "before.png"
            visible_path = artifact_dir / "visible.png"
            reshown_path = artifact_dir / "reshown.png"
            hidden_path = artifact_dir / "hidden.png"
            capture_root(before_path, env)

            overlay = start_overlay_host(env)
            set_reply = overlay.send(cursor_message(TARGET_CLICK_POINT, sequence=1))
            require_x11_overlay_reply(set_reply)
            time.sleep(0.2)
            capture_root(visible_path, env)
            visible_probe = probe_marker(before_path, visible_path, TARGET_CLICK_POINT)

            hide_reply = overlay.send({"version": 1, "kind": "hide", "reason": "X11 capture guard"})
            require_system_cursor_reply(hide_reply, hidden=False, context="hide")
            time.sleep(0.1)
            capture_root(hidden_path, env)
            hidden_probe = probe_marker(before_path, hidden_path, TARGET_CLICK_POINT)

            show_reply = overlay.send(show_cursor_message(hide_reply))
            require_visible_cursor_reply(show_reply, context="show")
            require_system_cursor_reply(show_reply, hidden=True, context="show")
            time.sleep(0.2)
            capture_root(reshown_path, env)
            reshown_probe = probe_marker(before_path, reshown_path, TARGET_CLICK_POINT)
            overlay_visible_for_click = reshown_probe.found
            click_target(env, TARGET_CLICK_POINT)
            clicked_state = wait_for_target_click(state_path, deadline=time.time() + 5)

            summary.update(
                {
                    "ok": visible_probe.found
                    and not hidden_probe.found
                    and overlay_visible_for_click
                    and clicked_state is not None,
                    "display": display,
                    "target": target_state,
                    "click_state": clicked_state,
                    "requested_point": {
                        "x": TARGET_CLICK_POINT[0],
                        "y": TARGET_CLICK_POINT[1],
                    },
                    "visible_overlay_captured": visible_probe.found,
                    "hidden_overlay_captured": hidden_probe.found,
                    "reshown_overlay_captured": reshown_probe.found,
                    "overlay_visible_for_click": overlay_visible_for_click,
                    "click_through_proved": clicked_state is not None and overlay_visible_for_click,
                    "backend": backend_from_reply(set_reply),
                    "system_cursor_hide_supported": capability_bool(
                        set_reply, "system_cursor_hide_supported"
                    ),
                    "system_cursor_hidden_after_set": capability_bool(
                        set_reply, "system_cursor_hidden"
                    ),
                    "system_cursor_hidden_after_hide": capability_bool(
                        hide_reply, "system_cursor_hidden"
                    ),
                    "system_cursor_hidden_after_show": capability_bool(
                        show_reply, "system_cursor_hidden"
                    ),
                    "set_cursor": set_reply,
                    "hide": hide_reply,
                    "show": show_reply,
                    "visible_marker_probe": marker_probe_json(visible_probe),
                    "hidden_marker_probe": marker_probe_json(hidden_probe),
                    "reshown_marker_probe": marker_probe_json(reshown_probe),
                    "before_screenshot": str(
                        copy_artifact(before_path, artifact_dir, "root-before")
                    ),
                    "visible_screenshot": str(
                        copy_artifact(visible_path, artifact_dir, "root-visible")
                    ),
                    "hidden_screenshot": str(
                        copy_artifact(hidden_path, artifact_dir, "root-hidden")
                    ),
                    "reshown_screenshot": str(
                        copy_artifact(reshown_path, artifact_dir, "root-reshown")
                    ),
                }
            )
        finally:
            if overlay is not None:
                overlay.close()
            if target is not None:
                terminate_process(target, name="Tk click-through target")

    write_summary(artifact_dir, summary)
    return 0 if summary.get("ok") is True else 1


def require_installed(binary: str) -> None:
    if shutil.which(binary) is None:
        raise RuntimeError(f"required binary is not installed: {binary}")


def make_runtime_dir(base_dir: Path) -> Path:
    runtime_dir = base_dir / "runtime"
    runtime_dir.mkdir(parents=True, exist_ok=True)
    runtime_dir.chmod(0o700)
    return runtime_dir


def wait_for_x11_display(display: str, *, deadline: float) -> None:
    env = {**os.environ, "DISPLAY": display}
    while time.time() < deadline:
        ready = subprocess.run(
            ["xdpyinfo"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
            env=env,
        )
        if ready.returncode == 0:
            return
        time.sleep(0.1)
    raise RuntimeError(f"timed out waiting for X11 display {display} to become ready")


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


def require_current_x11_display() -> str:
    display = os.environ.get("DISPLAY", "").strip()
    if not display:
        raise RuntimeError("--current-display requires DISPLAY to be set")
    session_type = os.environ.get("XDG_SESSION_TYPE", "").strip().lower()
    if session_type != "x11":
        raise RuntimeError(
            "--current-display is only accepted in a real X11 session; "
            f"XDG_SESSION_TYPE={session_type or '<unset>'}"
        )
    if os.environ.get("WAYLAND_DISPLAY"):
        raise RuntimeError(
            "--current-display refused a Wayland/XWayland session because "
            f"WAYLAND_DISPLAY={os.environ['WAYLAND_DISPLAY']!r}"
        )
    return display


def display_mode_for_args(args: argparse.Namespace) -> str:
    if args.current_display:
        return "current-x11-display"
    return "current-x11-display"


def build_overlay_host() -> None:
    prebuilt = overlay_host_path_from_env()
    if os.environ.get("SKY_CUA_USE_PREBUILT_RUNTIMES") == "1" and prebuilt:
        if not prebuilt.exists():
            raise RuntimeError(f"prebuilt overlay host binary does not exist: {prebuilt}")
        return
    subprocess.run(
        ["cargo", "build", "--package", "sky-cua-overlay-host"],
        cwd=REPO_ROOT,
        check=True,
    )


def overlay_host_path_from_env() -> Path | None:
    value = os.environ.get("SKY_CUA_OVERLAY_HOST_PATH") or os.environ.get(
        "SKY_CUA_DEBUG_OVERLAY_HOST_PATH"
    )
    return Path(value) if value else None


def overlay_host_binary() -> Path:
    return overlay_host_path_from_env() or OVERLAY_HOST_BIN


def x11_overlay_env(display: str, runtime_dir: Path) -> dict[str, str]:
    env = dict(os.environ)
    env["DISPLAY"] = display
    env["XDG_SESSION_TYPE"] = "x11"
    env["XDG_RUNTIME_DIR"] = str(runtime_dir)
    env["SKY_CUA_OVERLAY_BACKEND"] = "x11"
    env["GDK_BACKEND"] = "x11"
    env["QT_QPA_PLATFORM"] = "xcb"
    env.pop("WAYLAND_DISPLAY", None)
    return env


def start_target_window(env: Mapping[str, str], state_path: Path) -> subprocess.Popen[str]:
    target_env = {**env, "SKY_CUA_X11_OVERLAY_STATE": str(state_path)}
    return subprocess.Popen(
        [sys.executable, "-c", TK_FIXTURE_CODE],
        cwd=REPO_ROOT,
        env=target_env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )


def wait_for_target_state(state_path: Path, *, deadline: float) -> dict[str, Any]:
    while time.time() < deadline:
        state = read_target_state(state_path)
        if state.get("ready") is True:
            return state
        time.sleep(0.05)
    raise RuntimeError(f"timed out waiting for target window state at {state_path}")


def wait_for_target_click(state_path: Path, *, deadline: float) -> dict[str, Any] | None:
    while time.time() < deadline:
        state = read_target_state(state_path)
        if state.get("clicked") is True:
            return state
        time.sleep(0.05)
    return None


def read_target_state(state_path: Path) -> dict[str, Any]:
    if not state_path.exists():
        return {}
    data = json.loads(state_path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise RuntimeError(f"target state was not an object: {data!r}")
    return data


def capture_root(path: Path, env: Mapping[str, str]) -> None:
    subprocess.run(
        ["import", "-window", "root", str(path)],
        cwd=REPO_ROOT,
        env=dict(env),
        check=True,
    )


def start_overlay_host(env: Mapping[str, str]) -> OverlayHostProcess:
    process = subprocess.Popen(
        [str(overlay_host_binary()), "serve"],
        cwd=REPO_ROOT,
        env=dict(env),
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return OverlayHostProcess(process)


def cursor_message(point: tuple[float, float], *, sequence: int) -> dict[str, Any]:
    return {
        "version": 1,
        "kind": "set_cursor",
        "state": {
            "visible": True,
            "sequence": sequence,
            "model_point": {
                "x": point[0],
                "y": point[1],
                "coordinate_space": "stream_pixels",
            },
            "source_action": "click",
            "updated_at_ms": 0,
        },
    }


def show_cursor_message(reply: Mapping[str, Any]) -> dict[str, Any]:
    state = reply.get("state")
    if not isinstance(state, Mapping):
        raise RuntimeError(
            "overlay host reply did not include cursor state for show: "
            + json.dumps(reply, indent=2, sort_keys=True)
        )
    cloned_state = json.loads(json.dumps(state))
    if not isinstance(cloned_state, dict):
        raise RuntimeError(f"overlay host state was not an object: {state!r}")
    cloned_state["visible"] = True
    return {"version": 1, "kind": "show", "state": cloned_state}


def require_x11_overlay_reply(reply: Mapping[str, Any]) -> None:
    capabilities = reply.get("capabilities")
    if not reply.get("ok") or not isinstance(capabilities, dict):
        raise RuntimeError(
            "overlay host did not return usable capabilities: "
            + json.dumps(reply, indent=2, sort_keys=True)
        )
    expected = {
        "backend": "x11_shaped_window",
        "visible_overlay": True,
        "click_through": True,
        "system_cursor_hide_supported": True,
        "system_cursor_hidden": True,
    }
    for key, value in expected.items():
        if capabilities.get(key) != value:
            raise RuntimeError(
                f"overlay host capability {key!r} was {capabilities.get(key)!r}, "
                f"expected {value!r}.\nreply={json.dumps(reply, indent=2, sort_keys=True)}"
            )


def require_visible_cursor_reply(reply: Mapping[str, Any], *, context: str) -> None:
    if not reply.get("ok"):
        raise RuntimeError(
            f"overlay host rejected {context} request: "
            + json.dumps(reply, indent=2, sort_keys=True)
        )
    state = reply.get("state")
    if not isinstance(state, Mapping) or state.get("visible") is not True:
        raise RuntimeError(
            f"overlay host {context} reply did not confirm a visible cursor: "
            + json.dumps(reply, indent=2, sort_keys=True)
        )


def require_system_cursor_reply(reply: Mapping[str, Any], *, hidden: bool, context: str) -> None:
    if not reply.get("ok"):
        raise RuntimeError(
            f"overlay host rejected {context} request: "
            + json.dumps(reply, indent=2, sort_keys=True)
        )
    capabilities = reply.get("capabilities")
    if not isinstance(capabilities, Mapping):
        raise RuntimeError(
            f"overlay host {context} reply did not include capabilities: "
            + json.dumps(reply, indent=2, sort_keys=True)
        )
    expected = {
        "system_cursor_hide_supported": True,
        "system_cursor_hidden": hidden,
    }
    for key, value in expected.items():
        if capabilities.get(key) != value:
            raise RuntimeError(
                f"overlay host {context} capability {key!r} was "
                f"{capabilities.get(key)!r}, expected {value!r}.\n"
                f"reply={json.dumps(reply, indent=2, sort_keys=True)}"
            )


def backend_from_reply(reply: Mapping[str, Any]) -> str | None:
    capabilities = reply.get("capabilities")
    if isinstance(capabilities, dict):
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


def click_target(env: Mapping[str, str], point: tuple[float, float]) -> None:
    subprocess.run(
        [
            "xdotool",
            "mousemove",
            str(round(point[0])),
            str(round(point[1])),
            "click",
            "1",
        ],
        cwd=REPO_ROOT,
        env=dict(env),
        check=True,
    )


def marker_probe_json(probe: MarkerProbe) -> dict[str, Any]:
    return {
        "found": probe.found,
        "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
        "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
        "checked_box": list(probe.checked_box),
    }


if __name__ == "__main__":
    raise SystemExit(main())
