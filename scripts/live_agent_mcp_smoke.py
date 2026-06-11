#!/usr/bin/env python3
"""Generic agent MCP smoke harness for sky-cua.

Supports OpenCode, Pi, and Claude Code against various desktop fixtures.
Usage:
  python3 scripts/live_agent_mcp_smoke.py --agent opencode --fixture zenity
  python3 scripts/live_agent_mcp_smoke.py --agent pi --fixture kdialog
  python3 scripts/live_agent_mcp_smoke.py --agent claude --fixture zenity
"""

from __future__ import annotations

import argparse
import contextlib
import json
import subprocess
import sys
import time
from pathlib import Path

from _agent_mcp_smoke import make_artifact_dir, run_agent, write_result

FIXTURES = {
    "zenity": {
        "argv": [
            "zenity",
            "--info",
            "--title",
            "sky-cua agent smoke",
            "--text",
            "Agent sky-cua smoke dialog",
            "--width",
            "360",
        ],
        "title": "sky-cua agent smoke",
        "prompt_suffix": "dismiss it by clicking OK",
    },
    "kdialog": {
        "argv": [
            "kdialog",
            "--title",
            "sky-cua agent smoke",
            "--msgbox",
            "Agent sky-cua smoke dialog",
        ],
        "title": "sky-cua agent smoke",
        "prompt_suffix": "dismiss it by clicking OK",
    },
    "kate": {
        "argv": ["kate", "--new"],
        "title": "Untitled",
        "prompt_suffix": "type 'hello from sky-cua' into the editor, then save the file",
    },
    "ghostty": {
        "argv": ["ghostty"],
        "title": "Ghostty",
        "prompt_suffix": "type 'hello from sky-cua' into the terminal",
    },
}


def _parse_dismissed_from_stdout(stdout_path: Path) -> bool | None:
    """Scan the agent's stdout for a JSON object with a 'dismissed' key."""
    if not stdout_path.exists():
        return None
    text = stdout_path.read_text(encoding="utf-8")
    # Look for the last JSON code block or bare JSON object
    for block in reversed(text.split("```json")):
        json_text = block.split("```")[0] if "```" in block else block
        # Try the whole block first (multi-line JSON)
        stripped = json_text.strip()
        if stripped:
            try:
                obj = json.loads(stripped)
                if isinstance(obj, dict) and "dismissed" in obj:
                    return bool(obj["dismissed"])
            except json.JSONDecodeError:
                pass
        # Fall back to line-by-line for inline JSON
        for line in reversed(stripped.splitlines()):
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
                if isinstance(obj, dict) and "dismissed" in obj:
                    return bool(obj["dismissed"])
            except json.JSONDecodeError:
                # Try to extract the last {} object on the line
                start = line.rfind("{")
                end = line.rfind("}")
                if start != -1 and end != -1 and start < end:
                    try:
                        obj = json.loads(line[start : end + 1])
                        if isinstance(obj, dict) and "dismissed" in obj:
                            return bool(obj["dismissed"])
                    except json.JSONDecodeError:
                        pass
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description="Generic agent MCP smoke harness.")
    parser.add_argument(
        "--agent",
        choices=("opencode", "pi", "claude", "openclaw"),
        required=True,
        help="Agent to use for driving sky-cua.",
    )
    parser.add_argument(
        "--fixture",
        choices=tuple(FIXTURES.keys()),
        default="zenity",
        help="Desktop fixture to launch.",
    )
    args = parser.parse_args()

    fixture = FIXTURES[args.fixture]
    artifact_dir = make_artifact_dir(args.agent, args.fixture)

    dialog = subprocess.Popen(fixture["argv"])

    try:
        prompt = (
            f"Use the sky-cua MCP tools (server name sky_cua, sky-cua, or computer-use). "
            f"Load the computer-use skill if available. "
            f"Find the dialog titled '{fixture['title']}', "
            f"read its text, {fixture['prompt_suffix']}, and confirm it is gone. "
            f"Return a JSON object with keys: dialog_text (string), dismissed (boolean)."
        )

        proc = run_agent(args.agent, prompt, artifact_dir)

        # Parse the agent's stdout for a JSON result with a "dismissed" field;
        # this is the authoritative signal that the agent completed its task.
        stdout_path = artifact_dir / f"{args.agent}.stdout.log"
        agent_dismissed = _parse_dismissed_from_stdout(stdout_path)

        time.sleep(1)
        dialog_alive = dialog.poll() is None

        result = write_result(
            artifact_dir, args.agent, proc, dialog_alive, extra={"agent_dismissed": agent_dismissed}
        )

        # Trust the agent's reported result when available; fall back to
        # process-poll when the agent didn't produce a parseable result.
        ok = agent_dismissed if agent_dismissed is not None else not dialog_alive
        ok = ok and proc.returncode == 0

        if not ok:
            print(
                f"{args.agent} {args.fixture} smoke FAILED: {artifact_dir}",
                file=sys.stderr,
            )
            print(json.dumps(result, indent=2), file=sys.stderr)
            return 1

        print(f"{args.agent} {args.fixture} smoke passed: {artifact_dir}")
        print(json.dumps(result, indent=2))
        return 0

    finally:
        if dialog.poll() is None:
            dialog.terminate()
            try:
                dialog.wait(timeout=2)
            except subprocess.TimeoutExpired:
                dialog.kill()
                with contextlib.suppress(subprocess.TimeoutExpired):
                    dialog.wait(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
