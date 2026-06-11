#!/usr/bin/env python3
from __future__ import annotations

import argparse
import subprocess
import time

from _codex_exec import (
    DESKTOP_E2E_EXEC_ARGS,
    make_artifact_dir,
    prepare_chatgpt_plugin_test_home,
    read_last_message,
    require_computer_use_tool_call,
    run_codex_exec,
    with_plugin_mention,
)
from _plugin_bundle import REPO_ROOT


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run a codex-exec smoke against the installed sky-cua plugin."
    )
    parser.add_argument(
        "--symlink",
        action="store_true",
        help="Symlink the built bundle into the dedicated test Codex home.",
    )
    args = parser.parse_args()

    artifact_dir = make_artifact_dir("plugin-smoke")
    codex_home = prepare_chatgpt_plugin_test_home(
        artifact_dir=artifact_dir,
        symlink=args.symlink,
    )

    dialog = subprocess.Popen(
        [
            "zenity",
            "--info",
            "--title",
            "sky-cua codex e2e smoke",
            "--text",
            "sky-cua plugin smoke dialog",
            "--width",
            "360",
        ]
    )
    try:
        prompt = with_plugin_mention(
            """
Goal: inspect the visible zenity dialog titled `sky-cua codex e2e smoke`, report the dialog text, dismiss it, and return only the schema result.

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
""".strip(),
            codex_home,
        )
        result = run_codex_exec(
            prompt=prompt,
            artifact_dir=artifact_dir,
            output_schema=REPO_ROOT / "scripts" / "schemas" / "plugin_smoke_result.json",
            extra_env={"CODEX_HOME": str(codex_home)},
            extra_args=DESKTOP_E2E_EXEC_ARGS,
        )
        if result.exit_code != 0:
            raise SystemExit(f"codex exec exited with {result.exit_code}; inspect {artifact_dir}")
        require_computer_use_tool_call(result.transcript_path, artifact_dir=result.artifact_dir)
        message = read_last_message(result.last_message_path)
        if message.get("status") == "blocked":
            raise SystemExit(f"plugin smoke blocked: {message}; inspect {artifact_dir}")
        time.sleep(1)
        if dialog.poll() is None:
            dialog.terminate()
            try:
                dialog.wait(timeout=5)
            except subprocess.TimeoutExpired:
                dialog.kill()
            raise SystemExit(
                f"codex smoke did not dismiss the zenity dialog; inspect {artifact_dir}"
            )
        if message.get("status") != "completed":
            raise SystemExit(
                f"smoke returned non-completed status: {message}; inspect {artifact_dir}"
            )
        if not message.get("dialog_text"):
            raise SystemExit(f"smoke did not return dialog text: {message}")
        print(f"plugin smoke passed: {artifact_dir}")
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
