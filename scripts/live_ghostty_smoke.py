#!/usr/bin/env python3
"""Operator-run native Wayland Ghostty workflow smoke.

This launches Ghostty as a real GTK/Wayland app, targets it through MCP, and
proves keyboard-driven control by creating project-summary artifacts on disk.
The important detail is that keyboard actions stay element-scoped so the
backend can use AT-SPI focus; Ghostty's reported AT-SPI extents are not
trustworthy enough for a pre-click driven by physical coordinates.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

from live_desktop_smoke import (  # type: ignore[import-not-found]
    CLIENT,
    McpClient,
    require_no_portal_approval_pending,
    require_ok,
    wait_for_app_snapshot_result,
)

GHOSTTY_TITLE = "sky-cua ghostty smoke"
SUMMARY_NAME = "project-summary.md"
COUNT_NAME = "project-summary.lines"
EXPECTED_TEXT = (
    "# sky-cua\n"
    "\n"
    "- Rust-native split client/service Codex Computer Use plugin.\n"
    "- Wayland-first capture, input, and AT-SPI semantics on Linux.\n"
    "- Live-proven KDE Wayland, X11 fallback, and app-specific smoke workflows.\n"
)
SUMMARY_COMMAND = (
    "printf '%s\\n' '# sky-cua' '' "
    "'- Rust-native split client/service Codex Computer Use plugin.' "
    "'- Wayland-first capture, input, and AT-SPI semantics on Linux.' "
    "'- Live-proven KDE Wayland, X11 fallback, and app-specific smoke workflows.' "
    f"> {SUMMARY_NAME}"
)
COUNT_COMMAND = f"wc -l {SUMMARY_NAME} > {COUNT_NAME}"


def require_installed(binary: str) -> None:
    if shutil.which(binary) is None:
        raise RuntimeError(f"required binary is not installed: {binary}")


def launch_ghostty(workdir: Path) -> subprocess.Popen[str]:
    return subprocess.Popen(
        ["ghostty", f"--title={GHOSTTY_TITLE}", "-e", "bash", "--noprofile", "--norc"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        cwd=workdir,
        env=dict(os.environ),
    )


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
        print(f"{name} stderr: {stderr.strip()}")


def find_ghostty_target(snapshot: dict[str, Any]) -> dict[str, Any]:
    title_lower = GHOSTTY_TITLE.lower()
    candidates = [
        element
        for element in snapshot.get("elements", [])
        if element.get("backend_ref")
        and element.get("bounds")
        and "focus" in (element.get("semantic_actions") or [])
    ]
    named_widget = next(
        (
            element
            for element in candidates
            if element.get("role") == "widget"
            and title_lower in ((element.get("name") or "").lower())
        ),
        None,
    )
    if named_widget is not None:
        return named_widget

    if not candidates:
        raise RuntimeError(
            "did not find a plausible Ghostty focus target.\n"
            f"elements={json.dumps(snapshot.get('elements', []), indent=2, sort_keys=True)}"
        )

    return max(
        candidates,
        key=lambda element: float(element["bounds"]["width"]) * float(element["bounds"]["height"]),
    )


def wait_for_file_text(path: Path, expected: str, *, deadline: float) -> str | None:
    last = None
    while time.time() < deadline:
        if path.exists():
            last = path.read_text(encoding="utf-8")
            if last == expected:
                return last
        time.sleep(0.25)
    return last


def wait_for_file_contains(path: Path, needle: str, *, deadline: float) -> str | None:
    last = None
    while time.time() < deadline:
        if path.exists():
            last = path.read_text(encoding="utf-8")
            if needle in last:
                return last
        time.sleep(0.25)
    return last


def main() -> int:
    print("Starting live Ghostty Wayland smoke.")
    print("If KDE shows a portal approval prompt, approve it so the test can continue.\n")

    require_installed("ghostty")

    with tempfile.TemporaryDirectory(prefix="sky-cua-ghostty-smoke-") as tmpdir:
        tmpdir_path = Path(tmpdir)
        socket_path = tmpdir_path / "service.sock"
        summary_path = tmpdir_path / SUMMARY_NAME
        count_path = tmpdir_path / COUNT_NAME

        ghostty = launch_ghostty(tmpdir_path)
        client = McpClient(
            [str(CLIENT), "mcp"],
            extra_env={"SKY_CUA_SERVICE_SOCKET_PATH": str(socket_path)},
        )
        try:
            client.initialize()
            tools = {tool["name"] for tool in client.tools_list()}
            missing = sorted({"list_apps", "get_app_state", "type_text", "press_key"} - tools)
            if missing:
                raise RuntimeError(f"MCP server did not advertise required tools: {missing}")

            result = wait_for_app_snapshot_result(
                client,
                GHOSTTY_TITLE,
                deadline=time.time() + 30,
            )
            if result.get("isError"):
                structured = result.get("structuredContent") or {}
                code = structured.get("code")
                if code == "PortalApprovalPending":
                    raise RuntimeError(
                        "Ghostty smoke is still waiting on portal approval. "
                        "Approve the KDE dialog and re-run.\n"
                        f"result={json.dumps(result, indent=2, sort_keys=True)}"
                    )
                raise RuntimeError(
                    "Ghostty smoke failed before a snapshot was produced.\n"
                    f"result={json.dumps(result, indent=2, sort_keys=True)}"
                )

            snapshot = result["structuredContent"]
            require_no_portal_approval_pending(snapshot, "Ghostty snapshot")
            focused_app = snapshot.get("focused_app") or {}
            if focused_app.get("desktop_file_id") != "ghostty.desktop":
                raise RuntimeError(
                    "Ghostty smoke did not focus the expected desktop app.\n"
                    f"focused_app={json.dumps(focused_app, indent=2, sort_keys=True)}"
                )
            if str(focused_app.get("app_id") or "").startswith("x11:"):
                raise RuntimeError(
                    "Ghostty smoke surfaced as X11/XWayland instead of native Wayland.\n"
                    f"focused_app={json.dumps(focused_app, indent=2, sort_keys=True)}"
                )
            if GHOSTTY_TITLE.lower() not in ((focused_app.get("window_title") or "").lower()):
                raise RuntimeError(
                    "Ghostty smoke did not focus the expected titled window.\n"
                    f"focused_app={json.dumps(focused_app, indent=2, sort_keys=True)}"
                )

            target = find_ghostty_target(snapshot)

            type_summary = client.tools_call(
                30,
                "type_text",
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "element_index": target["element_index"],
                    "text": SUMMARY_COMMAND,
                },
            )
            require_ok(type_summary, "Ghostty summary type_text")
            run_summary = client.tools_call(
                31,
                "press_key",
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "element_index": target["element_index"],
                    "key": "Enter",
                },
            )
            require_ok(run_summary, "Ghostty summary enter")

            final_text = wait_for_file_text(summary_path, EXPECTED_TEXT, deadline=time.time() + 10)
            if final_text != EXPECTED_TEXT:
                raise RuntimeError(
                    "Ghostty smoke wrote the wrong summary bytes.\n"
                    f"expected={EXPECTED_TEXT!r}\n"
                    f"actual={final_text!r}"
                )

            type_count = client.tools_call(
                32,
                "type_text",
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "element_index": target["element_index"],
                    "text": COUNT_COMMAND,
                },
            )
            require_ok(type_count, "Ghostty count type_text")
            run_count = client.tools_call(
                33,
                "press_key",
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "element_index": target["element_index"],
                    "key": "Enter",
                },
            )
            require_ok(run_count, "Ghostty count enter")

            count_text = wait_for_file_contains(count_path, SUMMARY_NAME, deadline=time.time() + 10)
            if count_text is None:
                raise RuntimeError(
                    f"Ghostty smoke did not produce the line-count artifact.\npath={count_path}"
                )
            if not count_text.strip().startswith("5 "):
                raise RuntimeError(
                    f"Ghostty smoke produced an unexpected line count.\nactual={count_text!r}"
                )

            print(f"Focused app: {focused_app}")
            print(
                "Target element: "
                + json.dumps(
                    {
                        "element_index": target["element_index"],
                        "role": target.get("role"),
                        "name": target.get("name"),
                        "bounds": target.get("bounds"),
                    },
                    sort_keys=True,
                )
            )
            print(
                f"type_text result: {json.dumps(type_summary.get('structuredContent'), sort_keys=True)}"
            )
            print(
                f"press_key result: {json.dumps(run_summary.get('structuredContent'), sort_keys=True)}"
            )
            print(f"summary bytes: {final_text!r}")
            print(f"count file: {count_text!r}")
            print("\nLive Ghostty Wayland smoke completed successfully.")
            return 0
        finally:
            client.close()
            terminate_process(ghostty, name="Ghostty")


if __name__ == "__main__":
    raise SystemExit(main())
