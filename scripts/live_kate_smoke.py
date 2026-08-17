#!/usr/bin/env python3
"""Operator-run Kate smoke for heuristics-backed editor replacement.

This launches Kate on XWayland, targets the live editor through MCP, proves
that `set_value` uses the heuristics-backed physical replacement path, then
saves the file and verifies the final bytes on disk.
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
    appshot_action_fences,
    grouped_structured_result,
    isolated_daemon_env,
    require_no_portal_approval_pending,
    require_ok,
    wait_for_app_snapshot_result,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
KATE_TITLE = "proof.txt"
EXPECTED_TEXT = "hello from sky-cua kate proof\nsecond line\n"


def require_installed(binary: str) -> None:
    if shutil.which(binary) is None:
        raise RuntimeError(f"required binary is not installed: {binary}")


def launch_kate(path: Path) -> subprocess.Popen[str]:
    env = dict(os.environ)
    env["QT_QPA_PLATFORM"] = "xcb"
    return subprocess.Popen(
        ["kate", "--new", "--block", str(path)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        cwd=REPO_ROOT,
        env=env,
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


def find_kate_editor(snapshot: dict[str, Any]) -> dict[str, Any]:
    candidates = [
        element
        for element in snapshot.get("elements", [])
        if element.get("role") == "text"
        and element.get("backend_ref")
        and element.get("bounds")
        and float((element.get("bounds") or {}).get("width", 0) or 0) > 500
        and float((element.get("bounds") or {}).get("height", 0) or 0) > 300
    ]
    if not candidates:
        raise RuntimeError(
            "did not find a plausible Kate editor element.\n"
            f"elements={json.dumps(snapshot.get('elements', []), indent=2, sort_keys=True)}"
        )

    return max(
        candidates,
        key=lambda element: float(element["bounds"]["width"]) * float(element["bounds"]["height"]),
    )


def main() -> int:
    print("Starting live Kate smoke.")
    print("If KDE shows a portal approval prompt, approve it so the test can continue.\n")

    require_installed("kate")

    with tempfile.TemporaryDirectory(prefix="sky-cua-kate-smoke-") as tmpdir:
        tmpdir_path = Path(tmpdir)
        file_path = tmpdir_path / KATE_TITLE
        file_path.write_text("old value\n", encoding="utf-8")
        socket_path = tmpdir_path / "service.sock"

        kate = launch_kate(file_path)
        client = McpClient(
            [str(CLIENT), "mcp"],
            extra_env=isolated_daemon_env({"SKY_CUA_SERVICE_SOCKET_PATH": str(socket_path)}),
        )
        try:
            client.initialize()
            tools = {tool["name"] for tool in client.tools_list()}
            missing = sorted(
                {"desktop_keyboard", "desktop_set_value", "list_resources", "observe"} - tools
            )
            if missing:
                raise RuntimeError(f"MCP server did not advertise required tools: {missing}")

            result = wait_for_app_snapshot_result(
                client,
                KATE_TITLE,
                deadline=time.time() + 30,
            )
            if result.get("isError"):
                structured = result.get("structuredContent") or {}
                code = structured.get("code")
                if code == "PortalApprovalPending":
                    raise RuntimeError(
                        "Kate smoke is still waiting on portal approval. "
                        "Approve the KDE dialog and re-run.\n"
                        f"result={json.dumps(result, indent=2, sort_keys=True)}"
                    )
                raise RuntimeError(
                    "Kate smoke failed before a snapshot was produced.\n"
                    f"result={json.dumps(result, indent=2, sort_keys=True)}"
                )

            snapshot = result["structuredContent"]
            require_no_portal_approval_pending(snapshot, "Kate snapshot")
            editor = find_kate_editor(snapshot)

            focused_app = snapshot.get("focused_app") or {}
            if KATE_TITLE.lower() not in ((focused_app.get("window_title") or "").lower()):
                raise RuntimeError(
                    "Kate smoke did not focus the expected file window.\n"
                    f"focused_app={json.dumps(focused_app, indent=2, sort_keys=True)}"
                )

            set_value = client.tools_call(
                30,
                "desktop_set_value",
                {
                    **appshot_action_fences(snapshot),
                    "element_index": editor["element_index"],
                    "value": EXPECTED_TEXT,
                },
            )
            require_ok(set_value, "Kate set_value")
            set_diagnostics = grouped_structured_result(set_value).get("diagnostics") or []
            heuristic_diag = next(
                (
                    entry
                    for entry in set_diagnostics
                    if entry.get("code") == "HeuristicSetValueFallbackUsed"
                ),
                None,
            )
            if heuristic_diag is None:
                raise RuntimeError(
                    "Kate set_value did not surface the heuristics-backed fallback diagnostic.\n"
                    f"result={json.dumps(set_value, indent=2, sort_keys=True)}"
                )
            details = heuristic_diag.get("details") or ""
            if "routing=prefer_physical_fallback" not in details:
                raise RuntimeError(
                    "Kate set_value did not report the expected routing preference.\n"
                    f"diagnostic={json.dumps(heuristic_diag, indent=2, sort_keys=True)}"
                )

            save = client.tools_call(
                31,
                "desktop_keyboard",
                {
                    "operation": "press_key",
                    **appshot_action_fences(snapshot),
                    "key": "Ctrl+S",
                },
            )
            require_ok(save, "Kate save")
            save_message = (grouped_structured_result(save).get("message") or "").lower()
            if "x11 input fallback" not in save_message:
                raise RuntimeError(
                    "Kate save did not route through the X11 keyboard fallback.\n"
                    f"result={json.dumps(save, indent=2, sort_keys=True)}"
                )

            deadline = time.time() + 10
            final_text = None
            while time.time() < deadline:
                final_text = file_path.read_text(encoding="utf-8")
                if final_text == EXPECTED_TEXT:
                    break
                time.sleep(0.25)
            if final_text != EXPECTED_TEXT:
                raise RuntimeError(
                    "Kate smoke saved the wrong file contents.\n"
                    f"expected={EXPECTED_TEXT!r}\n"
                    f"actual={final_text!r}"
                )

            print(f"Focused app: {focused_app}")
            print(
                "Editor target: "
                + json.dumps(
                    {
                        "element_index": editor["element_index"],
                        "role": editor.get("role"),
                        "name": editor.get("name"),
                        "bounds": editor.get("bounds"),
                    },
                    sort_keys=True,
                )
            )
            print(f"set_value diagnostics: {json.dumps(set_diagnostics, sort_keys=True)}")
            print(f"save result: {json.dumps(grouped_structured_result(save), sort_keys=True)}")
            print(f"final file bytes: {final_text!r}")
            print("\nLive Kate smoke completed successfully.")
            return 0
        finally:
            client.close()
            terminate_process(kate, name="Kate")


if __name__ == "__main__":
    raise SystemExit(main())
