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
import os
import subprocess
import sys
import time
from collections import deque
from collections.abc import Callable
from pathlib import Path

from _agent_mcp_smoke import (
    DEFAULT_OPENCODE_SMOKE_MODEL,
    DEFAULT_PI_SMOKE_MODEL,
    TOOL_FAILURE_STATUSES,
    dismissed_json_from_text,
    make_artifact_dir,
    run_agent,
    write_result,
)

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
        "prompt_suffix": "dismiss it by confirming OK",
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
        "prompt_suffix": "dismiss it by confirming OK",
    },
}
PI_MCP_WRAPPER_GUIDANCE = (
    "When using Pi's generic mcp tool, pass an object with tool set to the sky-cua "
    "tool name and args set to a JSON object, not a JSON string. Prefer "
    'sky_cua_observe with args {"surface":"desktop","element_query":"<title>"}, '
    'then sky_cua_desktop_keyboard with args {"operation":"press_key","key":"Enter"}. '
    "Do not call desktop list_resources with title_contains; desktop windows do not support "
    "that filter."
)
SKY_CUA_ACTION_TOOL_NAMES = {
    "activate_window",
    "browser_claim_tab",
    "browser_eval",
    "browser_input",
    "browser_move_mouse",
    "browser_navigate",
    "browser_open",
    "browser_scroll",
    "desktop_action",
    "desktop_keyboard",
    "desktop_pointer",
    "desktop_scroll",
    "desktop_semantic",
    "desktop_set_value",
    "desktop_toggle",
    "phone_app_action",
    "phone_app_force_stop",
    "phone_app_install",
    "phone_connection",
    "phone_keyboard",
    "phone_notification_action",
    "phone_notification_reply",
    "phone_pair_wireless",
    "phone_pointer",
    "phone_setup",
}


def build_agent_prompt(*, agent: str, fixture_title: str, prompt_suffix: str) -> str:
    prompt = (
        f"Use the sky-cua MCP tools (server name sky_cua, sky-cua, or computer-use). "
        f"Find the dialog titled '{fixture_title}' and {prompt_suffix}. "
        f"Keep the interaction simple and direct; use window/state/click/keyboard tools only as needed. "
        f"For a focused confirmation dialog, desktop_keyboard Enter is acceptable. "
        f"Do not use shell commands, process inspection, OCR, window-manager commands, "
        f"global keyboard shortcuts, or non-sky-cua desktop shortcuts as substitutes for sky-cua MCP tools. "
    )
    if agent == "pi":
        prompt += PI_MCP_WRAPPER_GUIDANCE + " "
    prompt += (
        "After a successful sky-cua action, return immediately without extra verification loops. "
        "Return a JSON object with keys: dialog_text (string or null), dismissed (boolean)."
    )
    return prompt


def _parse_dismissed_from_stdout(stdout_path: Path) -> bool | None:
    """Scan the agent's stdout for a JSON object with a 'dismissed' key."""
    if not stdout_path.exists():
        return None
    latest: bool | None = None
    tail: deque[str] = deque(maxlen=512)
    with stdout_path.open(encoding="utf-8") as stdout_file:
        for line in stdout_file:
            tail.append(line)
            parsed = _dismissed_from_stdout_line(line)
            if parsed is not None:
                latest = parsed
    if latest is not None:
        return latest
    return dismissed_json_from_text("".join(tail))


def _dismissed_from_stdout_line(line: str) -> bool | None:
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        return dismissed_json_from_text(line)
    if isinstance(event, dict):
        latest: bool | None = None
        for payload in _assistant_text_payloads_from_event(event):
            parsed = dismissed_json_from_text(payload)
            if parsed is not None:
                latest = parsed
        if latest is not None:
            return latest
    return dismissed_json_from_text(line)


def _assistant_text_payloads_from_event(event: dict[str, object]) -> list[str]:
    message = event.get("message")
    if not isinstance(message, dict) or message.get("role") != "assistant":
        return []
    payloads: list[str] = []
    content = message.get("content")
    if isinstance(content, list):
        for item in content:
            if isinstance(item, dict) and item.get("type") == "text":
                text = item.get("text")
                if isinstance(text, str):
                    payloads.append(text)
    return payloads


def _stdout_has_sky_cua_tool_evidence(stdout_path: Path) -> bool | None:
    """Return true when JSON-mode stdout records a sky-cua/computer-use tool call."""
    return _stdout_has_sky_cua_tool_evidence_matching(
        stdout_path,
        start_predicate=_is_sky_cua_tool_start,
        record_predicate=_record_has_successful_sky_cua_tool_identity,
    )


def _stdout_has_sky_cua_action_tool_evidence(stdout_path: Path) -> bool | None:
    """Return true when stdout records a successful sky-cua action tool call."""
    return _stdout_has_sky_cua_tool_evidence_matching(
        stdout_path,
        start_predicate=_is_sky_cua_action_tool_start,
        record_predicate=_record_has_successful_sky_cua_action_tool_identity,
    )


def _stdout_has_sky_cua_tool_evidence_matching(
    stdout_path: Path,
    *,
    start_predicate: Callable[[dict[str, object]], bool],
    record_predicate: Callable[[object], bool],
) -> bool | None:
    if not stdout_path.exists():
        return None
    saw_json_event = False
    pending_matching_tool_ids: set[str] = set()
    pending_anonymous_matching_tool = False
    with stdout_path.open(encoding="utf-8") as stdout_file:
        for line in stdout_file:
            event = _json_object_from_stdout_line(line)
            if event is not None:
                saw_json_event = True
                if start_predicate(event):
                    call_id = _tool_call_id_from_event(event)
                    if call_id is not None:
                        pending_matching_tool_ids.add(call_id)
                    else:
                        pending_anonymous_matching_tool = True
                elif _is_tool_completion_event(event):
                    call_id = _tool_call_id_from_event(event)
                    if (
                        call_id is not None
                        and call_id in pending_matching_tool_ids
                        and _is_successful_tool_completion_event(event)
                    ):
                        return True
                    if call_id is not None:
                        pending_matching_tool_ids.discard(call_id)
                    elif pending_anonymous_matching_tool:
                        pending_anonymous_matching_tool = False
                        if _is_successful_tool_completion_event(event):
                            return True
            found = _tool_evidence_from_stdout_line_matching(line, record_predicate)
            if found is True:
                return True
            if found is False:
                saw_json_event = True
    return False if saw_json_event else None


def _tool_evidence_from_stdout_line(line: str) -> bool | None:
    return _tool_evidence_from_stdout_line_matching(
        line,
        _record_has_successful_sky_cua_tool_identity,
    )


def _tool_evidence_from_stdout_line_matching(
    line: str,
    record_predicate: Callable[[object], bool],
) -> bool | None:
    event = _json_object_from_stdout_line(line)
    if event is None:
        return None
    event_type = event.get("type")
    if _is_completed_tool_event(event_type) and record_predicate(event):
        return True
    tool_results = event.get("toolResults")
    return isinstance(tool_results, list) and record_predicate(tool_results)


def _json_object_from_stdout_line(line: str) -> dict[str, object] | None:
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        return None
    if not isinstance(event, dict):
        return None
    return event


def _tool_call_id_from_event(event: dict[str, object]) -> str | None:
    for key in ("toolCallId", "tool_call_id", "callId", "call_id", "id"):
        value = event.get(key)
        if isinstance(value, str) and value:
            return value
    return None


def _is_sky_cua_tool_start(event: dict[str, object]) -> bool:
    event_type = event.get("type")
    if not isinstance(event_type, str) or event_type.lower().replace("-", "_") not in {
        "tool_execution_start",
        "tool_use_start",
    }:
        return False
    tool = event.get("tool")
    return isinstance(tool, str) and _field_names_sky_cua_tool("tool", tool)


def _is_sky_cua_action_tool_start(event: dict[str, object]) -> bool:
    event_type = event.get("type")
    if not isinstance(event_type, str) or event_type.lower().replace("-", "_") not in {
        "tool_execution_start",
        "tool_use_start",
    }:
        return False
    tool = event.get("tool")
    return isinstance(tool, str) and _field_names_sky_cua_action_tool("tool", tool)


def _is_successful_tool_completion_event(event: dict[str, object]) -> bool:
    if not _is_tool_completion_event(event):
        return False
    return not _record_is_failed_tool_result(event) and _record_has_tool_result_payload(event)


def _is_tool_completion_event(event: dict[str, object]) -> bool:
    event_type = event.get("type")
    if not isinstance(event_type, str):
        return False
    return event_type.lower().replace("-", "_") in {
        "tool_result",
        "tool_execution_end",
        "tool_use_end",
    }


def _is_completed_tool_event(event_type: object) -> bool:
    if not isinstance(event_type, str):
        return False
    normalized = event_type.lower().replace("-", "_")
    return normalized in {"tool_result", "tool_execution_end", "tool_use"}


def _record_has_successful_sky_cua_tool_identity(
    value: object,
    *,
    parent_failed: bool = False,
    parent_has_tool_result_payload: bool = False,
) -> bool:
    if isinstance(value, list):
        return any(
            _record_has_successful_sky_cua_tool_identity(
                item,
                parent_failed=parent_failed,
                parent_has_tool_result_payload=parent_has_tool_result_payload,
            )
            for item in value
        )
    if not isinstance(value, dict):
        return False
    current_has_payload = parent_has_tool_result_payload or _record_has_tool_result_payload(value)
    current_failed = parent_failed or _record_is_failed_tool_result(value)
    if _record_directly_names_sky_cua_tool(value):
        return not current_failed and current_has_payload
    for key, item in value.items():
        normalized_key = key.lower().replace("-", "_")
        if normalized_key in {
            "arguments",
            "args",
            "details",
            "input",
            "metadata",
            "meta",
            "parameters",
            "part",
            "state",
            "toolcall",
            "tool_call",
        } and _record_has_successful_sky_cua_tool_identity(
            item,
            parent_failed=current_failed,
            parent_has_tool_result_payload=current_has_payload,
        ):
            return True
    return False


def _record_directly_names_sky_cua_tool(record: dict[str, object]) -> bool:
    for key, item in record.items():
        normalized_key = key.lower().replace("-", "_")
        if (
            normalized_key in {"toolname", "tool_name", "tool", "name"}
            and isinstance(item, str)
            and _field_names_sky_cua_tool(normalized_key, item)
        ):
            return True
    return False


def _record_has_successful_sky_cua_action_tool_identity(
    value: object,
    *,
    parent_failed: bool = False,
    parent_has_tool_result_payload: bool = False,
) -> bool:
    if isinstance(value, list):
        return any(
            _record_has_successful_sky_cua_action_tool_identity(
                item,
                parent_failed=parent_failed,
                parent_has_tool_result_payload=parent_has_tool_result_payload,
            )
            for item in value
        )
    if not isinstance(value, dict):
        return False
    current_has_payload = parent_has_tool_result_payload or _record_has_tool_result_payload(value)
    current_failed = parent_failed or _record_is_failed_tool_result(value)
    if _record_directly_names_sky_cua_action_tool(value):
        return not current_failed and current_has_payload
    for key, item in value.items():
        normalized_key = key.lower().replace("-", "_")
        if normalized_key in {
            "arguments",
            "args",
            "details",
            "input",
            "metadata",
            "meta",
            "parameters",
            "part",
            "state",
            "toolcall",
            "tool_call",
        } and _record_has_successful_sky_cua_action_tool_identity(
            item,
            parent_failed=current_failed,
            parent_has_tool_result_payload=current_has_payload,
        ):
            return True
    return False


def _record_directly_names_sky_cua_action_tool(record: dict[str, object]) -> bool:
    for key, item in record.items():
        normalized_key = key.lower().replace("-", "_")
        if (
            normalized_key in {"toolname", "tool_name", "tool", "name"}
            and isinstance(item, str)
            and _field_names_sky_cua_action_tool(normalized_key, item)
        ):
            return True
    return False


def _field_names_sky_cua_tool(normalized_key: str, value: str) -> bool:
    if normalized_key in {"toolname", "tool_name", "tool", "name"}:
        return value.startswith(
            ("sky_cua_", "mcp__computer_use__", "mcp__sky-cua__", "mcp__sky_cua__")
        )
    if normalized_key in {"server", "server_name", "mcp_server", "servername"}:
        return value in {"sky_cua", "sky-cua", "computer-use", "mcp__computer_use"}
    return False


def _field_names_sky_cua_action_tool(normalized_key: str, value: str) -> bool:
    if normalized_key not in {"toolname", "tool_name", "tool", "name"}:
        return False
    return _sky_cua_tool_base_name(value) in SKY_CUA_ACTION_TOOL_NAMES


def _sky_cua_tool_base_name(value: str) -> str | None:
    for prefix in (
        "sky_cua_",
        "mcp__computer_use__",
        "mcp__sky-cua__",
        "mcp__sky_cua__",
    ):
        if value.startswith(prefix):
            return value.removeprefix(prefix)
    return None


def _record_is_failed_tool_result(record: dict[str, object]) -> bool:
    if record.get("is_error") is True or record.get("isError") is True:
        return True
    if _record_declares_failure(record):
        return True
    for key, item in record.items():
        normalized_key = key.lower().replace("-", "_")
        if (
            normalized_key in {"status", "phase", "state", "result"}
            and isinstance(item, str)
            and item.lower() in TOOL_FAILURE_STATUSES
        ):
            return True
    for key in (
        "content",
        "output",
        "result",
        "structuredContent",
        "structured_content",
        "toolResult",
        "tool_result",
        "state",
    ):
        item = record.get(key)
        if isinstance(item, dict) and _record_is_failed_tool_result(item):
            return True
    return False


def _record_declares_failure(record: dict[str, object]) -> bool:
    if record.get("is_error") is True or record.get("isError") is True:
        return True
    if record.get("result_declares_failure") is True:
        return True
    error = record.get("error")
    if isinstance(error, str) and error.strip():
        return True
    if isinstance(error, (dict, list)) and error:
        return True
    for key in ("status", "phase", "state"):
        item = record.get(key)
        if isinstance(item, str) and item.lower() in TOOL_FAILURE_STATUSES:
            return True
    return False


def _record_has_tool_result_payload(record: dict[str, object]) -> bool:
    for key in (
        "output",
        "content",
        "structuredContent",
        "structured_content",
        "toolResult",
        "tool_result",
    ):
        item = record.get(key)
        if item is not None:
            return True
    state = record.get("state")
    if isinstance(state, dict) and _record_has_tool_result_payload(state):
        return True
    result = record.get("result")
    return isinstance(result, (dict, list))


def run_fixture_smoke(
    *,
    agent: str,
    fixture_name: str,
    model: str | None = None,
    profile_name: str | None = None,
) -> int:
    fixture = FIXTURES[fixture_name]
    smoke_name = profile_name or fixture_name
    artifact_dir = make_artifact_dir(agent, smoke_name)
    effective_model = model
    if agent == "opencode" and effective_model is None:
        effective_model = os.environ.get(
            "SKY_CUA_SMOKE_OPENCODE_MODEL", DEFAULT_OPENCODE_SMOKE_MODEL
        )
    if agent == "pi" and effective_model is None:
        effective_model = os.environ.get("SKY_CUA_SMOKE_PI_MODEL", DEFAULT_PI_SMOKE_MODEL)

    dialog = subprocess.Popen(fixture["argv"])

    try:
        prompt = build_agent_prompt(
            agent=agent,
            fixture_title=fixture["title"],
            prompt_suffix=fixture["prompt_suffix"],
        )

        proc = run_agent(agent, prompt, artifact_dir, model=effective_model)

        # Parse the agent's stdout for a JSON result with a "dismissed" field.
        # The observed fixture state is the acceptance signal; the self-report
        # is retained as diagnostic evidence because small models can dismiss a
        # dialog with a real tool action and still summarize it poorly.
        stdout_path = artifact_dir / f"{agent}.stdout.log"
        agent_dismissed = _parse_dismissed_from_stdout(stdout_path)
        tool_evidence = _stdout_has_sky_cua_tool_evidence(stdout_path)
        action_tool_evidence = _stdout_has_sky_cua_action_tool_evidence(stdout_path)
        requires_tool_evidence = agent in {"opencode", "pi"}

        time.sleep(1)
        dialog_alive = dialog.poll() is None
        dialog_dismissed = not dialog_alive

        ok = proc.returncode == 0 and dialog_dismissed
        if requires_tool_evidence:
            ok = ok and action_tool_evidence is True

        result = write_result(
            artifact_dir,
            agent,
            proc,
            dialog_alive,
            extra={
                "agent_dismissed": agent_dismissed,
                "dialog_dismissed": dialog_dismissed,
                "fixture": fixture_name,
                "model": effective_model,
                "ok": ok,
                "action_tool_evidence": action_tool_evidence,
                "requires_tool_evidence": requires_tool_evidence,
                "tool_evidence": tool_evidence,
            },
        )

        if not ok:
            print(
                f"{agent} {smoke_name} smoke FAILED: {artifact_dir}",
                file=sys.stderr,
            )
            print(json.dumps(result, indent=2), file=sys.stderr)
            return 1

        print(f"{agent} {smoke_name} smoke passed: {artifact_dir}")
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


PI_WIRING_GUIDANCE = (
    "When using Pi's generic mcp tool, pass an object with tool set to the sky-cua tool name and "
    "args set to a JSON object, not a JSON string. Use sky_cua_doctor with args {} or "
    'sky_cua_observe with args {"surface":"desktop"}.'
)


def build_wiring_prompt(agent: str) -> str:
    """Prompt for the minimal MCP wiring check: list tools, call one read-only tool."""
    prompt = (
        "Use the sky-cua MCP tools (server name sky_cua, sky-cua, or computer-use). "
        "This is a wiring check, not a task: confirm the sky-cua tool schema is available to you, "
        "then call exactly ONE read-only sky-cua tool to prove the connection works. "
        'Prefer `doctor` (no arguments) or `observe` with {"surface":"desktop"}. '
        "Do not perform any input actions (no clicks, typing, key presses, or scrolling) and do not "
        "use shell commands, process inspection, or non-sky-cua tools. "
    )
    if agent == "pi":
        prompt += PI_WIRING_GUIDANCE + " "
    prompt += (
        "After the read-only tool returns, stop and return a JSON object with keys: "
        "tools_listed (boolean: whether the sky-cua tool schema was visible), "
        "read_only_tool_called (string: the tool name you called), "
        "error (string or null: any error you observed)."
    )
    return prompt


def run_wiring_check(*, agent: str, model: str | None = None) -> int:
    """Minimal MCP smoke: prove the agent sees the sky-cua schema and a read-only call succeeds.

    This replaces the dialog-dismiss task for opencode/pi in the consolidated matrix. It proves
    only that MCP is wired for the agent: the schema loads and one read-only tool (doctor/observe)
    returns without error. Substantive tool-use coverage lives in the codex CUA smoke.
    """
    artifact_dir = make_artifact_dir(agent, "wiring")
    effective_model = model
    if agent == "opencode" and effective_model is None:
        effective_model = os.environ.get(
            "SKY_CUA_SMOKE_OPENCODE_MODEL", DEFAULT_OPENCODE_SMOKE_MODEL
        )
    if agent == "pi" and effective_model is None:
        effective_model = os.environ.get("SKY_CUA_SMOKE_PI_MODEL", DEFAULT_PI_SMOKE_MODEL)

    prompt = build_wiring_prompt(agent)
    proc = run_agent(agent, prompt, artifact_dir, model=effective_model)

    # A successful read-only sky-cua tool call is the acceptance signal: the schema loaded and a
    # tool returned without error. Any sky-cua tool name counts (doctor/observe are read-only).
    stdout_path = artifact_dir / f"{agent}.stdout.log"
    tool_evidence = _stdout_has_sky_cua_tool_evidence(stdout_path)
    ok = proc.returncode == 0 and tool_evidence is True

    result = write_result(
        artifact_dir,
        agent,
        proc,
        dialog_alive=False,
        extra={
            "mode": "wiring",
            "model": effective_model,
            "ok": ok,
            "tool_evidence": tool_evidence,
        },
    )

    if not ok:
        print(f"{agent} wiring smoke FAILED: {artifact_dir}", file=sys.stderr)
        print(json.dumps(result, indent=2), file=sys.stderr)
        return 1

    print(f"{agent} wiring smoke passed: {artifact_dir}")
    print(json.dumps(result, indent=2))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Generic agent MCP smoke harness.")
    parser.add_argument(
        "--agent",
        choices=("opencode", "pi", "claude", "openclaw"),
        required=True,
        help="Agent to use for driving sky-cua.",
    )
    parser.add_argument(
        "--mode",
        choices=("wiring", "dialog"),
        default="wiring",
        help=(
            "wiring (default): minimal schema + one read-only tool call. "
            "dialog: legacy find-and-dismiss-a-dialog task against --fixture."
        ),
    )
    parser.add_argument(
        "--fixture",
        choices=tuple(FIXTURES.keys()),
        default="zenity",
        help="Desktop fixture to launch (dialog mode only).",
    )
    parser.add_argument(
        "--model",
        default=None,
        help=(
            "Agent model override. "
            f"OpenCode defaults to {DEFAULT_OPENCODE_SMOKE_MODEL}; "
            f"Pi defaults to {DEFAULT_PI_SMOKE_MODEL}."
        ),
    )
    args = parser.parse_args()
    if args.mode == "wiring":
        return run_wiring_check(agent=args.agent, model=args.model)
    return run_fixture_smoke(agent=args.agent, fixture_name=args.fixture, model=args.model)


if __name__ == "__main__":
    raise SystemExit(main())
