#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
import time

from _app_server_harness import (
    require_computer_use_item,
    run_rich_app_server_turn,
)
from _codex_exec import make_artifact_dir
from _plugin_bundle import REPO_ROOT


def main() -> int:
    artifact_dir = make_artifact_dir("app-server-smoke")
    dialog = subprocess.Popen(
        [
            "zenity",
            "--info",
            "--title",
            "sky-cua rich app-server smoke",
            "--text",
            "sky-cua rich harness dialog",
            "--width",
            "360",
        ]
    )
    try:
        prompt = """
Goal: inspect the visible zenity dialog titled `sky-cua rich app-server smoke`, report the dialog text, dismiss it, and return only the schema result.

Required workflow:
- The MCP server is named `computer-use`; in Codex tool calls this may appear as namespaced tools such as `mcp__computer_use__list_apps`.
- Use the computer-use MCP tools directly: start with `mcp__computer_use__list_apps`, then `mcp__computer_use__get_app_state`, then `mcp__computer_use__click` or `mcp__computer_use__perform_secondary_action` as needed.
- Re-run `get_app_state` after dismissal if you need confirmation.

Rules:
- Do not use `list_mcp_resources` or other MCP-resource discovery as a substitute for the actual computer-use tools.
- Do not use shell commands, process inspection, OCR, or xdotool/wmctrl tricks to read the dialog text or close the dialog. The only allowed shell use is launching a missing target app, which this smoke does not require.
- Prefer the skill's hybrid workflow: use the tree for structure, screenshots for confirmation, and physical actions if the obvious button is easier than a semantic click.
- Include a screenshot_path from plugin state if you can.
- If blocked by portal approval or missing app state, classify the result honestly instead of inventing success.
            """.strip()
        result = run_rich_app_server_turn(
            prompt=prompt,
            artifact_dir=artifact_dir,
            output_schema=REPO_ROOT / "scripts" / "schemas" / "plugin_smoke_result.json",
        )
        require_computer_use_item(result.transcript_path)
        message = json.loads(result.last_message_path.read_text())
        if message.get("status") != "completed":
            raise SystemExit(
                f"rich app-server smoke returned non-completed status: {message}; inspect {artifact_dir}"
            )
        if not message.get("dialog_text"):
            raise SystemExit(
                f"rich app-server smoke did not return dialog text: {message}; inspect {artifact_dir}"
            )
        time.sleep(1)
        if dialog.poll() is None:
            dialog.terminate()
            try:
                dialog.wait(timeout=5)
            except subprocess.TimeoutExpired:
                dialog.kill()
            raise SystemExit(
                f"rich app-server smoke did not dismiss the zenity dialog; inspect {artifact_dir}"
            )
        print(f"rich app-server smoke passed: {artifact_dir}")
        print(json.dumps(message, indent=2))
        return 0
    finally:
        if dialog.poll() is None:
            dialog.terminate()
            try:
                dialog.wait(timeout=2)
            except subprocess.TimeoutExpired:
                dialog.kill()


if __name__ == "__main__":
    raise SystemExit(main())
