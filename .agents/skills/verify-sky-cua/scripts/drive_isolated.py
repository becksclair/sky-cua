#!/usr/bin/env python3
"""Drive the isolated sky-cua instance through its MCP surface and record evidence.

Journeys (``--journey``, default ``all`` = smoke + semantic + keyboard):

- ``smoke``: tools/list surface and the doctor tool.
- ``semantic``: launch a zenity entry dialog into the private desktop with
  ``desktop_launch_app``, observe it, write a unique token with
  ``desktop_set_value`` on the entry, read it back from a fresh observation,
  activate OK with ``desktop_action``, and confirm the side effect (zenity wrote
  the token to a file and exited 0) plus window disappearance.
- ``keyboard``: the same dialog, but the token goes in through
  ``desktop_keyboard`` type_text and the dialog is accepted with press_key Return.

Every app journey also proves non-interference: the launched app's environment
points at the private display and bus, and no host X display shows the window.

Run through ``verify.sh drive``; it supplies the hermetic isolated environment.
Evidence lands in ``--run-dir``: ``steps.jsonl`` (every MCP request/response),
``images/`` (AppShot images copied out of the service's temporary store), the
zenity output files, and ``summary.json``. Exit status is 0 only when every
check passes.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import secrets
import shutil
import subprocess
import sys
import time
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

JOURNEYS = ("smoke", "semantic", "keyboard")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--client", required=True, type=Path)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--display", required=True)
    parser.add_argument("--repo-scripts", required=True, type=Path)
    parser.add_argument("--journey", default="all", choices=["all", *JOURNEYS])
    parser.add_argument("--timeout", type=float, default=30.0)
    return parser.parse_args()


def walk(node: Any, key: str = "") -> Iterator[tuple[str, Any]]:
    """Yield (key, value) for every scalar in a JSON tree."""
    if isinstance(node, dict):
        for child_key, child in node.items():
            yield from walk(child, str(child_key))
    elif isinstance(node, list):
        for child in node:
            yield from walk(child, key)
    else:
        yield key, node


def first(node: Any, wanted: str) -> Any:
    for key, value in walk(node):
        if key == wanted and value not in (None, ""):
            return value
    return None


def result_of(response: dict[str, Any]) -> dict[str, Any]:
    """Successful grouped payloads live under structuredContent.result."""
    return (response.get("structuredContent") or {}).get("result") or {}


def elements_of(observation: dict[str, Any]) -> list[dict[str, Any]]:
    return (result_of(observation).get("semantic_projection") or {}).get("elements") or []


def find_element(
    observation: dict[str, Any], predicate: Callable[[dict[str, Any]], bool]
) -> dict[str, Any] | None:
    return next((element for element in elements_of(observation) if predicate(element)), None)


def is_entry(element: dict[str, Any]) -> bool:
    return "set_value" in (element.get("semantic_actions") or []) and "text" in str(
        element.get("role")
    )


def is_ok_button(element: dict[str, Any]) -> bool:
    return element.get("role") == "button" and element.get("name") == "OK"


def environ_of(pid: int) -> dict[str, str]:
    raw = Path(f"/proc/{pid}/environ").read_bytes()
    pairs = (item.split(b"=", 1) for item in raw.split(b"\0") if b"=" in item)
    return {k.decode(errors="replace"): v.decode(errors="replace") for k, v in pairs}


def child_pids(pid: int) -> list[int]:
    out = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True, check=False)
    return [int(line) for line in out.stdout.split()]


def host_display_report(marker: str, own_display: str) -> dict[str, str]:
    """Best-effort: is a window titled ``marker`` visible on any host X display?"""
    # Map each X server's display to its -auth file from its cmdline (readable even
    # when the server or compositor is setcap'd and its environ is not).
    auth_by_display: dict[str, str] = {}
    for cmdline in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            argv = cmdline.read_bytes().split(b"\0")
        except OSError:
            continue
        words = [word.decode(errors="replace") for word in argv if word]
        if not words or Path(words[0]).name not in {"Xwayland", "Xorg", "X"}:
            continue
        display = next((word for word in words[1:] if word.startswith(":")), None)
        if display and "-auth" in words[:-1]:
            auth_by_display[display] = words[words.index("-auth") + 1]
    report: dict[str, str] = {}
    for sock in sorted(Path("/tmp/.X11-unix").glob("X*")):
        display = f":{sock.name[1:]}"
        if display == own_display:
            continue
        env = {k: v for k, v in os.environ.items() if k != "WAYLAND_DISPLAY"}
        env["DISPLAY"] = display
        if display in auth_by_display:
            env["XAUTHORITY"] = auth_by_display[display]
        probe = subprocess.run(
            ["xdotool", "search", "--name", marker],
            capture_output=True,
            text=True,
            env=env,
            check=False,
            timeout=10,
        )
        if probe.stdout.strip():
            report[display] = "present"
        elif probe.returncode == 1 and not probe.stderr.strip():
            report[display] = "absent"
        else:
            report[display] = f"unqueryable: {probe.stderr.strip()[:120]}"
    return report


class Recorder:
    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.images = run_dir / "images"
        self.images.mkdir(parents=True, exist_ok=True)
        self.steps_path = run_dir / "steps.jsonl"
        runtime_store = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
        self.appshot_store = runtime_store / "sky-cua/appshots"
        self.capture_store = runtime_store / "sky-cua/captures"
        self.checks: list[dict[str, Any]] = []
        self.seq = 0
        self.journey = ""

    def log_call(self, name: str, arguments: dict[str, Any], response: dict[str, Any]) -> None:
        self.seq += 1
        saved: list[str] = []
        prefix = f"{self.seq:02d}-{self.journey or 'run'}-{name}"
        for index, block in enumerate(response.get("content") or []):
            if isinstance(block, dict) and block.get("type") == "image" and block.get("data"):
                ext = {"image/png": "png", "image/jpeg": "jpg", "image/webp": "webp"}.get(
                    str(block.get("mimeType")), "bin"
                )
                path = self.images / f"{prefix}-inline{index}.{ext}"
                path.write_bytes(base64.b64decode(str(block["data"])))
                saved.append(path.name)
        # AppShot images are temporary host-private artifacts named by filename;
        # copy them out before they expire. The full-resolution source capture
        # lives in captures/<snapshot_id>.png.
        for key, value in walk(response.get("structuredContent")):
            if not isinstance(value, str) or "/" in value:
                continue
            if key == "filename":
                source = self.appshot_store / value
            elif key == "snapshot_id":
                source = self.capture_store / f"{value}.png"
            else:
                continue
            target = self.images / f"{prefix}-{source.name}"
            if source.is_file() and not target.exists():
                shutil.copy2(source, target)
                saved.append(target.name)
        slim = {
            **response,
            "content": [
                {**b, "data": f"<{len(str(b.get('data', '')))} b64 chars>"}
                if isinstance(b, dict) and b.get("type") == "image"
                else b
                for b in (response.get("content") or [])
            ],
        }
        entry = {
            "seq": self.seq,
            "journey": self.journey,
            "tool": name,
            "arguments": arguments,
            "images": saved,
            "response": slim,
        }
        with self.steps_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, sort_keys=True) + "\n")

    def check(self, name: str, passed: bool, detail: Any = None) -> bool:
        label = f"{self.journey}: {name}" if self.journey else name
        self.checks.append(
            {"journey": self.journey, "check": name, "passed": bool(passed), "detail": detail}
        )
        mark = "PASS" if passed else "FAIL"
        print(f"[{mark}] {label}" + (f": {detail}" if detail is not None else ""), flush=True)
        return passed


class Driver:
    def __init__(self, args: argparse.Namespace, client: Any, rec: Recorder) -> None:
        self.args = args
        self.client = client
        self.rec = rec
        self.request_id = 10
        self.live_pids: list[int] = []

    def call(self, name: str, arguments: dict[str, Any], *, log: bool = True) -> dict[str, Any]:
        self.request_id += 1
        response = self.client.tools_call(self.request_id, name, arguments)
        if log:
            self.rec.log_call(name, arguments, response)
        return response

    def windows_with(self, marker: str) -> tuple[bool, dict[str, Any]]:
        response = self.call(
            "list_resources", {"surface": "desktop", "resource": "windows"}, log=False
        )
        return marker in json.dumps(result_of(response)), response

    # ---------------------------------------------------------------- journeys

    def smoke(self) -> None:
        tools = self.client.tools_list()
        (self.rec.run_dir / "tools-list.json").write_text(
            json.dumps(tools, indent=2, sort_keys=True)
        )
        names = {tool["name"] for tool in tools}
        self.rec.check(
            "tools/list advertises the desktop tools",
            {
                "desktop_launch_app",
                "observe",
                "desktop_keyboard",
                "desktop_set_value",
                "desktop_action",
                "doctor",
            }
            <= names,
            sorted(names),
        )
        self.rec.check(
            "browser and phone tools are absent (SKY_CUA_SURFACES=desktop)",
            not any(name.startswith(("browser_", "phone_")) for name in names),
        )
        doctor = self.call("doctor", {})
        self.rec.check("doctor returned without isError", not doctor.get("isError"))
        self.rec.check(
            "doctor reports session_kind x11", first(result_of(doctor), "session_kind") == "x11"
        )

    def open_dialog(self) -> tuple[str, dict[str, Any], Path, Path] | None:
        """Launch a zenity entry dialog into the private desktop; return its observation."""
        marker = f"sky-cua-verify-{self.rec.journey}-{secrets.token_hex(3)}"
        out_file = self.rec.run_dir / f"{self.rec.journey}-zenity-output.txt"
        rc_file = self.rec.run_dir / f"{self.rec.journey}-zenity-exit.txt"
        script = (
            f"zenity --entry --title='{marker}' --text='sky-cua verification' "
            f"--width=420 > '{out_file}'; echo $? > '{rc_file}'"
        )
        launched = self.call("desktop_launch_app", {"command": "sh", "args": ["-c", script]})
        sh_pid = first(result_of(launched), "pid")
        if not self.rec.check("desktop_launch_app returned a pid", isinstance(sh_pid, int), sh_pid):
            return None
        self.live_pids.append(sh_pid)

        deadline = time.monotonic() + self.args.timeout
        found, windows = False, {}
        while time.monotonic() < deadline:
            found, windows = self.windows_with(marker)
            if found:
                break
            time.sleep(0.5)
        self.rec.log_call("list_resources", {"surface": "desktop", "resource": "windows"}, windows)
        if not self.rec.check("dialog window listed on the private desktop", found, marker):
            return None

        zenity = child_pids(sh_pid)
        if self.rec.check("zenity process found under the launched shell", bool(zenity), zenity):
            self.live_pids.extend(zenity)
            env = environ_of(zenity[0])
            user_bus = f"unix:path={os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000')}/bus"
            self.rec.check(
                "app DISPLAY is the private display",
                env.get("DISPLAY") == self.args.display,
                env.get("DISPLAY"),
            )
            self.rec.check(
                "app has no WAYLAND_DISPLAY",
                "WAYLAND_DISPLAY" not in env,
                env.get("WAYLAND_DISPLAY"),
            )
            self.rec.check(
                "app session bus is private, not the user's",
                env.get("DBUS_SESSION_BUS_ADDRESS") not in (None, user_bus),
                env.get("DBUS_SESSION_BUS_ADDRESS"),
            )
        hosts = host_display_report(marker, self.args.display)
        self.rec.check(
            "window absent from every queryable host X display",
            "present" not in hosts.values(),
            hosts,
        )

        observation = self.call("observe", {"surface": "desktop", "window_title": marker})
        entry = find_element(observation, is_entry)
        self.rec.check(
            "observe returned an appshot_id, snapshot_id, and the entry element",
            bool(
                first(result_of(observation), "appshot_id")
                and first(result_of(observation), "snapshot_id")
                and entry
            ),
            entry
            and {k: entry.get(k) for k in ("element_index", "role", "value", "semantic_actions")},
        )
        if not entry:
            return None
        return marker, observation, out_file, rc_file

    def confirm_side_effect(self, marker: str, token: str, out_file: Path, rc_file: Path) -> None:
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline and not rc_file.exists():
            time.sleep(0.25)
        written = out_file.read_text().strip() if out_file.exists() else None
        exit_code = rc_file.read_text().strip() if rc_file.exists() else None
        self.rec.check("side effect: zenity printed exactly the token", written == token, written)
        self.rec.check("side effect: zenity exited 0 (OK accepted)", exit_code == "0", exit_code)
        still_listed, windows = self.windows_with(marker)
        self.rec.log_call("list_resources", {"surface": "desktop", "resource": "windows"}, windows)
        self.rec.check("dialog window is gone afterwards", not still_listed)

    def readback(self, marker: str, token: str, initial: Any) -> dict[str, Any]:
        after = self.call("observe", {"surface": "desktop", "window_title": marker})
        entry = find_element(after, is_entry)
        final = entry.get("value") if entry else None
        self.rec.check(
            "fresh structured readback: entry value == token and != initial",
            final == token and final != initial,
            {"initial": initial, "final": final},
        )
        return after

    def semantic(self) -> None:
        opened = self.open_dialog()
        if not opened:
            return
        marker, observation, out_file, rc_file = opened
        token = f"semantic-{secrets.token_hex(4)}"
        result = result_of(observation)
        entry = find_element(observation, is_entry) or {}
        set_value = self.call(
            "desktop_set_value",
            {
                "appshot_id": result["appshot_id"],
                "snapshot_id": first(result, "snapshot_id"),
                "element_index": entry["element_index"],
                "value": token,
            },
        )
        self.rec.check(
            "desktop_set_value succeeded",
            not set_value.get("isError"),
            result_of(set_value).get("message"),
        )
        after = self.readback(marker, token, entry.get("value"))
        ok = find_element(after, is_ok_button)
        if not self.rec.check("OK button present in the fresh observation", ok is not None):
            return
        assert ok is not None
        activated = self.call(
            "desktop_action",
            {
                "operation": "activate",
                "appshot_id": result_of(after)["appshot_id"],
                "snapshot_id": first(result_of(after), "snapshot_id"),
                "element_index": ok["element_index"],
            },
        )
        self.rec.check(
            "desktop_action activate OK succeeded",
            not activated.get("isError"),
            result_of(activated).get("message"),
        )
        self.confirm_side_effect(marker, token, out_file, rc_file)

    def keyboard(self) -> None:
        opened = self.open_dialog()
        if not opened:
            return
        marker, observation, out_file, rc_file = opened
        token = f"keyboard-{secrets.token_hex(4)}"
        entry = find_element(observation, is_entry) or {}
        typed = self.call(
            "desktop_keyboard",
            {
                "operation": "type_text",
                "text": token,
                "appshot_id": result_of(observation)["appshot_id"],
            },
        )
        self.rec.check(
            "desktop_keyboard type_text returned success",
            not typed.get("isError"),
            result_of(typed).get("message"),
        )
        after = self.readback(marker, token, entry.get("value"))
        pressed = self.call(
            "desktop_keyboard",
            {
                "operation": "press_key",
                "key": "Return",
                "appshot_id": result_of(after)["appshot_id"],
            },
        )
        self.rec.check(
            "desktop_keyboard press_key returned success",
            not pressed.get("isError"),
            result_of(pressed).get("message"),
        )
        self.confirm_side_effect(marker, token, out_file, rc_file)

    def reap(self) -> None:
        """Kill only what this driver launched, by pid; never by name."""
        for pid in reversed(self.live_pids):
            if Path(f"/proc/{pid}").exists():
                subprocess.run(["kill", str(pid)], check=False, capture_output=True)
        self.live_pids.clear()


def finish(rec: Recorder) -> int:
    passed = bool(rec.checks) and all(check["passed"] for check in rec.checks)
    journeys: dict[str, bool] = {}
    for check in rec.checks:
        journeys[check["journey"]] = journeys.get(check["journey"], True) and check["passed"]
    summary = {"passed": passed, "journeys": journeys, "checks": rec.checks}
    (rec.run_dir / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True))
    verdicts = ", ".join(f"{name}={'PASS' if ok else 'FAIL'}" for name, ok in journeys.items())
    print(f"summary: {verdicts} -> {rec.run_dir / 'summary.json'}", flush=True)
    return 0 if passed else 1


def main() -> int:
    args = parse_args()
    sys.path.insert(0, str(args.repo_scripts))
    from _mcp_stdio import McpClient  # repo harness: Content-Length framed MCP stdio

    args.run_dir.mkdir(parents=True, exist_ok=True)
    rec = Recorder(args.run_dir)
    client = McpClient(
        [str(args.client), "mcp"],
        cwd=args.repo_scripts.parent,
        read_timeout=60.0,
        client_name="verify-sky-cua",
        gate_deploy=False,
    )
    driver = Driver(args, client, rec)
    selected = JOURNEYS if args.journey == "all" else (args.journey,)
    try:
        client.initialize()
        for journey in selected:
            rec.journey = journey
            try:
                getattr(driver, journey)()
            except Exception as error:  # record the failure as evidence and continue
                rec.check("journey raised", False, f"{type(error).__name__}: {error}")
            finally:
                driver.reap()
        return finish(rec)
    finally:
        client.close()


if __name__ == "__main__":
    raise SystemExit(main())
