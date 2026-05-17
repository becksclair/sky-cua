from __future__ import annotations

import json
import os
import subprocess
from collections.abc import Iterable
from pathlib import Path
from typing import Any

TITLE = "sky-cua text readback smoke"
INITIAL_VALUE = "stale-readback"
REPLACEMENT_VALUE = "verified-readback"


def run_zenity_readback_dialog() -> subprocess.Popen[str]:
    return subprocess.Popen(
        [
            "zenity",
            "--entry",
            f"--title={TITLE}",
            "--text=sky-cua text readback smoke",
            f"--entry-text={INITIAL_VALUE}",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=dict(os.environ),
    )


def text_readback_prompt() -> str:
    return f"""
Goal: use computer-use to inspect the visible zenity entry dialog titled `{TITLE}`, prove the entry currently contains `{INITIAL_VALUE}` from `get_app_state` readback, replace it with `{REPLACEMENT_VALUE}`, prove the replacement from a fresh `get_app_state`, submit the dialog, and return only the schema result.

Required workflow:
- The MCP server is named `computer-use`; in Codex tool calls this may appear as namespaced tools such as `mcp__computer_use__list_apps`.
- Use the computer-use MCP tools directly: start with `mcp__computer_use__list_apps`, then `mcp__computer_use__get_app_state`.
- Inspect the editable element's `value` and/or `text.content`; do not rely only on OCR or the screenshot for the entry contents.
- Use `mcp__computer_use__set_value` on the editable element to replace the value with `{REPLACEMENT_VALUE}`.
- Re-run `get_app_state` and verify the editable element's `value` or `text.content` is exactly `{REPLACEMENT_VALUE}` before submitting.
- Submit with `click` on OK or `press_key` Enter through computer-use.

Rules:
- Do not use shell commands, process inspection, OCR-only proof, xdotool, wmctrl, or clipboard tricks to read or change the entry.
- If blocked by portal approval or missing app state, classify the result honestly instead of inventing success.
- Set `initial_value` to the value observed from snapshot readback and `replacement_value` to the value observed from the fresh post-edit snapshot.
""".strip()


def require_submitted_value(dialog: subprocess.Popen[str], artifact_dir: Path) -> None:
    if dialog.poll() is None:
        dialog.terminate()
        try:
            dialog.wait(timeout=5)
        except subprocess.TimeoutExpired:
            dialog.kill()
        raise SystemExit(f"text readback smoke did not submit the dialog; inspect {artifact_dir}")
    stdout, stderr = dialog.communicate(timeout=5)
    if dialog.returncode != 0:
        raise SystemExit(
            f"zenity text readback dialog exited with {dialog.returncode}; "
            f"stdout={stdout!r} stderr={stderr!r}; inspect {artifact_dir}"
        )
    if stdout.strip() != REPLACEMENT_VALUE:
        raise SystemExit(
            f"expected submitted value {REPLACEMENT_VALUE!r}, got {stdout.strip()!r}; "
            f"inspect {artifact_dir}"
        )


def close_dialog_if_running(dialog: subprocess.Popen[str]) -> None:
    if dialog.poll() is None:
        dialog.terminate()
        try:
            dialog.wait(timeout=2)
        except subprocess.TimeoutExpired:
            dialog.kill()


def require_text_readback_message(message: dict[str, Any], artifact_dir: Path) -> None:
    if message.get("status") != "completed":
        raise SystemExit(
            f"text readback smoke returned non-completed status: {message}; inspect {artifact_dir}"
        )
    if message.get("initial_value") != INITIAL_VALUE:
        raise SystemExit(
            f"text readback smoke did not report initial value from readback: {message}; "
            f"inspect {artifact_dir}"
        )
    if message.get("replacement_value") != REPLACEMENT_VALUE:
        raise SystemExit(
            f"text readback smoke did not report replacement value from readback: {message}; "
            f"inspect {artifact_dir}"
        )


def require_transcript_readback_values(items: Iterable[dict[str, Any]], artifact_dir: Path) -> None:
    get_state_items = [item for item in items if _contains_string(item, "get_app_state")]
    saw_initial = any(_contains_string(item, INITIAL_VALUE) for item in get_state_items)
    saw_replacement = any(_contains_string(item, REPLACEMENT_VALUE) for item in get_state_items)
    if not saw_initial or not saw_replacement:
        sample = json.dumps(get_state_items[-3:], indent=2, sort_keys=True)[:4000]
        raise RuntimeError(
            "text readback transcript did not prove both initial and replacement values "
            f"inside get_app_state tool items; saw_initial={saw_initial} "
            f"saw_replacement={saw_replacement}. Inspect {artifact_dir}\n{sample}"
        )


def _contains_string(value: Any, needle: str) -> bool:
    if isinstance(value, str):
        return needle in value
    if isinstance(value, dict):
        return any(_contains_string(item, needle) for item in value.values())
    if isinstance(value, list):
        return any(_contains_string(item, needle) for item in value)
    return False
