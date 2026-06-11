#!/usr/bin/env python3
"""Post-deploy smoke verifying OpenClaw can use the sky-cua MCP server.

Stages:
  1. show   - `openclaw mcp show sky_cua --json`: the registration exists, the
              client binary is present, and the codex approval mode will not
              raise per-call approval prompts during agent turns.
  2. probe  - `openclaw mcp probe sky_cua --json`: OpenClaw spawns the server
              and the required computer-use and browser-use tools are listed.
  3. agent  - optional (--agent-turn): one live agent turn via
              `openclaw agent --json` that must call sky_cua browser_status
              and report structured evidence.

Usage:
  python3 scripts/live_openclaw_mcp_smoke.py
  python3 scripts/live_openclaw_mcp_smoke.py --agent-turn
  python3 scripts/live_openclaw_mcp_smoke.py --agent-turn --agent sky
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from collections.abc import Iterator
from pathlib import Path
from typing import Any

from _agent_mcp_smoke import make_artifact_dir
from install_mcp_server import CODEX_TOOLS_APPROVAL_MODE

SERVER_NAME = "sky_cua"
SMOKE_REPORT_KEY = "sky_cua_smoke"
AGENT_TURN_TOOL_NAME = f"{SERVER_NAME}__browser_status"
COMMAND_TIMEOUT_SECONDS = 60
AGENT_TURN_TIMEOUT_SECONDS = 300
TIMEOUT_RETURNCODE = 124
TOOL_SUCCESS_STATUSES = {"completed", "ok", "success", "succeeded"}
TOOL_FAILURE_STATUSES = {"error", "failed", "failure", "unsuccessful"}
OPENCLAW_TOOL_SUMMARY_CONTEXT_KEYS = {
    "completion",
    "executionTrace",
    "finalAssistantRawText",
    "finalAssistantVisibleText",
    "livenessState",
    "requestShaping",
    "replayInvalid",
    "stopReason",
}
PROBE_ATTEMPTS = 3
PROBE_RETRY_DELAY_SECONDS = 5

# A deployment is only usable when both the desktop and browser surfaces are
# advertised; this is the minimum contract an OpenClaw agent turn relies on.
REQUIRED_TOOLS = (
    f"{SERVER_NAME}__browser_status",
    f"{SERVER_NAME}__browser_list_tabs",
    f"{SERVER_NAME}__browser_open",
    f"{SERVER_NAME}__browser_snapshot",
    f"{SERVER_NAME}__browser_click",
    f"{SERVER_NAME}__doctor",
    f"{SERVER_NAME}__list_apps",
    f"{SERVER_NAME}__list_windows",
)

AGENT_TURN_PROMPT = (
    f"This is an automated sky-cua deployment smoke test. Call the MCP tool "
    f"{SERVER_NAME}__browser_status (server {SERVER_NAME}, tool browser_status) "
    f"with no arguments. Then reply with exactly one JSON object of the shape "
    f'{{"{SMOKE_REPORT_KEY}": {{"tool_called": <bool>, '
    f'"tools_visible": <bool>, "error": <string or null>}}}} and no other '
    f"text. Set tool_called to true only if the browser_status call actually "
    f"executed and returned a result. Set tools_visible to true only if "
    f"{SERVER_NAME} tools are available to you this turn."
)


def run_openclaw(
    openclaw_bin: str,
    args: list[str],
    artifact_dir: Path,
    log_name: str,
    openclaw_dir: Path | None,
    timeout: float = COMMAND_TIMEOUT_SECONDS,
    extra_env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    """Run an openclaw CLI command, always leaving stdout/stderr artifacts.

    A hung command is exactly what this smoke exists to catch, so timeouts
    return a synthetic failed result (rc=TIMEOUT_RETURNCODE) with whatever
    partial output was captured instead of raising out of the stage.
    """
    env = os.environ.copy()
    if openclaw_dir is not None:
        env["OPENCLAW_STATE_DIR"] = str(openclaw_dir)
        env["OPENCLAW_CONFIG_PATH"] = str(openclaw_dir / "openclaw.json")
    if extra_env:
        env.update(extra_env)
    argv = [openclaw_bin, *args]
    try:
        proc = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        stdout = _timeout_output_text(error.stdout)
        stderr = _timeout_output_text(error.stderr)
        stderr += f"\n[smoke] command timed out after {timeout} seconds\n"
        proc = subprocess.CompletedProcess(argv, TIMEOUT_RETURNCODE, stdout, stderr)
    (artifact_dir / f"{log_name}.stdout.log").write_text(proc.stdout, encoding="utf-8")
    (artifact_dir / f"{log_name}.stderr.log").write_text(proc.stderr, encoding="utf-8")
    return proc


def _timeout_output_text(captured: str | bytes | None) -> str:
    if captured is None:
        return ""
    if isinstance(captured, bytes):
        return captured.decode(errors="replace")
    return captured


def gateway_auth_environment(openclaw_dir: Path | None) -> dict[str, str]:
    """Resolve gateway credentials for `openclaw agent` from the systemd env file.

    `openclaw agent` talks to the running Gateway, and when gateway.auth uses a
    secret reference the CLI needs OPENCLAW_GATEWAY_TOKEN or
    OPENCLAW_GATEWAY_PASSWORD in its environment. Interactive shells usually do
    not export these, but gateway deployments keep them in
    <state_dir>/gateway.systemd.env. Values already present in the environment
    win; the file is only a fallback and is never written or logged.
    """
    if os.environ.get("OPENCLAW_GATEWAY_TOKEN") or os.environ.get("OPENCLAW_GATEWAY_PASSWORD"):
        return {}
    state_dir = (openclaw_dir or (Path.home() / ".openclaw")).expanduser()
    env_file = state_dir / "gateway.systemd.env"
    if not env_file.exists():
        return {}
    resolved: dict[str, str] = {}
    for line in env_file.read_text(encoding="utf-8").splitlines():
        name, _, value = line.partition("=")
        if name in ("OPENCLAW_GATEWAY_TOKEN", "OPENCLAW_GATEWAY_PASSWORD") and value:
            resolved[name] = value.strip().strip('"')
    return resolved


def parse_json_output(stdout: str, stage: str) -> dict[str, Any]:
    try:
        parsed = json.loads(stdout)
    except json.JSONDecodeError as error:
        raise ValueError(f"{stage}: output is not valid JSON: {error}") from error
    if not isinstance(parsed, dict):
        raise ValueError(f"{stage}: expected a JSON object, got {type(parsed).__name__}")
    return parsed


def check_show_config(config: dict[str, Any]) -> list[str]:
    """Validate the registered server config; return failure messages."""
    failures: list[str] = []

    command = config.get("command")
    if not isinstance(command, str) or not command:
        failures.append("config has no command; sky_cua is not a stdio server registration")
    elif not Path(command).exists():
        failures.append(f"client binary does not exist: {command}")

    if config.get("enabled") is False:
        failures.append("server is disabled (enabled: false); OpenClaw will not project it")

    codex = config.get("codex")
    approval_mode = codex.get("defaultToolsApprovalMode") if isinstance(codex, dict) else None
    if approval_mode != CODEX_TOOLS_APPROVAL_MODE:
        failures.append(
            f"codex.defaultToolsApprovalMode is {approval_mode!r}, expected "
            f"{CODEX_TOOLS_APPROVAL_MODE!r}. Codex 'approve' approves every tool "
            "call without user interaction; 'auto' prompts for unannotated MCP "
            "tools, which codex treats as destructive and open-world by default. "
            "Re-run: python3 scripts/install_mcp_server.py --host openclaw"
        )

    env = config.get("env")
    if not isinstance(env, dict) or "SKY_CUA_REPO_ROOT" not in env:
        failures.append("config env does not pin SKY_CUA_REPO_ROOT")

    return failures


def check_probe_result(probe: dict[str, Any]) -> list[str]:
    """Validate probe capabilities; return failure messages."""
    failures: list[str] = []

    servers = probe.get("servers")
    server = servers.get(SERVER_NAME) if isinstance(servers, dict) else None
    if not isinstance(server, dict):
        failures.append(f"probe did not connect to server {SERVER_NAME}")
        return failures

    tools = probe.get("tools")
    tool_names = (
        [name for name in tools if isinstance(name, str)] if isinstance(tools, list) else []
    )
    missing = [name for name in REQUIRED_TOOLS if name not in tool_names]
    if missing:
        failures.append(f"probe is missing required tools: {', '.join(missing)}")

    diagnostics = probe.get("diagnostics")
    if isinstance(diagnostics, list) and diagnostics:
        rendered = "; ".join(str(item) for item in diagnostics)
        failures.append(f"probe reported diagnostics: {rendered}")

    return failures


def extract_smoke_report(text: str) -> dict[str, Any] | None:
    """Extract the agent's structured smoke report from `openclaw agent --json`.

    Decodes every JSON object in the text and keeps the last one carrying the
    report, recursing into reply-text string fields that embed JSON.
    """
    report: dict[str, Any] | None = None
    for candidate in iter_decoded_json_values(text):
        found = _dig_smoke_report(candidate)
        if found is not None:
            report = found
    return report


def iter_decoded_json_values(text: str) -> Iterator[Any]:
    decoder = json.JSONDecoder()
    index = text.find("{")
    while index != -1:
        try:
            candidate, end = decoder.raw_decode(text, index)
        except json.JSONDecodeError:
            index = text.find("{", index + 1)
            continue
        yield candidate
        index = text.find("{", end)


def _dig_smoke_report(value: object) -> dict[str, Any] | None:
    if isinstance(value, dict):
        report = value.get(SMOKE_REPORT_KEY)
        if isinstance(report, dict):
            return report
        for child in value.values():
            found = _dig_smoke_report(child)
            if found is not None:
                return found
    elif isinstance(value, list):
        for child in value:
            found = _dig_smoke_report(child)
            if found is not None:
                return found
    elif isinstance(value, str) and SMOKE_REPORT_KEY in value:
        return extract_smoke_report(value)
    return None


def agent_turn_has_browser_status_tool_event(text: str) -> bool:
    """Return True only when stdout contains browser_status result evidence."""
    _report, tool_result_seen = scan_agent_turn_stdout(text)
    return tool_result_seen


def scan_agent_turn_stdout(text: str) -> tuple[dict[str, Any] | None, bool]:
    report: dict[str, Any] | None = None
    browser_status_call_ids: set[str] = set()
    tool_result_seen = False
    for line in text.splitlines():
        stripped = line.strip()
        if (
            (stripped.startswith("[tool result]") or stripped.startswith("[tool_result]"))
            and _text_names_browser_status_tool(stripped)
            and _text_has_success_status(stripped)
        ):
            tool_result_seen = True
    for candidate in iter_decoded_json_values(text):
        found = _dig_smoke_report(candidate)
        if found is not None:
            report = found
        if _has_browser_status_tool_result(candidate, browser_status_call_ids):
            tool_result_seen = True
    return report, tool_result_seen


def _has_browser_status_tool_result(value: object, call_ids: set[str]) -> bool:
    if isinstance(value, dict):
        if _record_has_successful_tool_summary(value):
            return True
        data = value.get("data")
        if _record_is_implicit_tool_result(value) and isinstance(data, dict):
            child = {**data}
            for key in (
                "toolCallId",
                "tool_call_id",
                "toolUseId",
                "tool_use_id",
                "name",
                "toolName",
                "tool_name",
                "tool",
                "title",
            ):
                if key in value:
                    child.setdefault(key, value[key])
            child.setdefault("sessionUpdate", "tool_result")
            if _has_browser_status_tool_result(child, call_ids):
                return True
        is_tool_record = _is_tool_event_record(value)
        names_browser_status = _record_directly_names_browser_status_tool(value)
        call_id = _record_tool_call_id(value)
        if is_tool_record and names_browser_status and call_id is not None:
            call_ids.add(call_id)
        matched_browser_status = is_tool_record and (
            names_browser_status or (call_id is not None and call_id in call_ids)
        )
        if matched_browser_status and _record_is_failed_tool_result(value):
            return False
        if matched_browser_status:
            if _record_is_implicit_tool_result(value) and _record_has_result_payload(value):
                return True
            if _record_has_successful_tool_result(value) and _record_has_result_payload(value):
                return True
        return any(_has_browser_status_tool_result(child, call_ids) for child in value.values())
    if isinstance(value, list):
        return any(_has_browser_status_tool_result(child, call_ids) for child in value)
    return False


def _is_tool_event_record(record: dict[str, Any]) -> bool:
    for key in ("type", "sessionUpdate", "kind", "event", "role", "method"):
        value = record.get(key)
        if isinstance(value, str) and "tool" in value.lower():
            return True
    return False


def _record_tool_call_id(record: dict[str, Any]) -> str | None:
    for key in ("toolCallId", "tool_call_id", "toolUseId", "tool_use_id", "id"):
        value = record.get(key)
        if isinstance(value, str) and value:
            return value
    return None


def _record_has_successful_tool_result(record: dict[str, Any]) -> bool:
    for key in ("status", "phase", "state", "result"):
        value = record.get(key)
        if isinstance(value, str) and value.lower() in TOOL_SUCCESS_STATUSES:
            return True
    return False


def _record_has_result_payload(record: dict[str, Any]) -> bool:
    for key in (
        "output",
        "content",
        "structuredContent",
        "structured_content",
        "toolResult",
        "tool_result",
    ):
        if key in record and record[key] is not None:
            return True
    result = record.get("result")
    return isinstance(result, (dict, list))


def _record_is_failed_tool_result(record: dict[str, Any]) -> bool:
    if record.get("is_error") is True or record.get("isError") is True:
        return True
    for key in ("status", "phase", "state", "result"):
        value = record.get(key)
        if isinstance(value, str) and value.lower() in TOOL_FAILURE_STATUSES:
            return True
    return False


def _record_is_implicit_tool_result(record: dict[str, Any]) -> bool:
    for key in ("type", "sessionUpdate", "kind", "event", "role"):
        value = record.get(key)
        if isinstance(value, str) and "toolresult" in _normalized_tool_event_kind(value):
            return not _record_is_failed_tool_result(record)
    return False


def _normalized_tool_event_kind(value: str) -> str:
    return re.sub(r"[^a-z]", "", value.lower())


def _record_directly_names_browser_status_tool(record: dict[str, Any]) -> bool:
    for key in ("name", "toolName", "tool_name", "tool", "title"):
        value = record.get(key)
        if isinstance(value, str) and _text_names_browser_status_tool(value):
            return True
    return False


def _record_has_successful_tool_summary(record: dict[str, Any]) -> bool:
    if record.keys().isdisjoint(OPENCLAW_TOOL_SUMMARY_CONTEXT_KEYS):
        return False
    summary = record.get("toolSummary")
    if not isinstance(summary, dict) or summary.get("failures") != 0:
        return False
    calls = summary.get("calls")
    if not isinstance(calls, int) or calls <= 0:
        return False
    tools = summary.get("tools")
    return isinstance(tools, list) and any(
        isinstance(tool, str) and _text_names_browser_status_tool(tool) for tool in tools
    )


def _text_names_browser_status_tool(text: str) -> bool:
    return AGENT_TURN_TOOL_NAME in text or (SERVER_NAME in text and "browser_status" in text)


def _text_has_success_status(text: str) -> bool:
    lowered = text.lower()
    if re.search(r"\b(?:no|not|never)\s+(?:completed|ok|success|succeeded)\b", lowered):
        return False
    tokens = set(re.findall(r"[a-z]+", lowered))
    return tokens.isdisjoint(TOOL_FAILURE_STATUSES) and not tokens.isdisjoint(TOOL_SUCCESS_STATUSES)


def check_agent_report(report: dict[str, Any] | None) -> list[str]:
    """Validate the agent turn's structured evidence; return failure messages."""
    if report is None:
        return [
            "agent turn produced no structured smoke report; the model likely "
            f"could not see or call {SERVER_NAME} tools"
        ]
    failures: list[str] = []
    if report.get("tools_visible") is not True:
        failures.append("agent reported sky_cua tools were not visible during the turn")
    if report.get("tool_called") is not True:
        error = report.get("error")
        detail = f" (agent error: {error})" if error else ""
        failures.append(f"agent could not execute {SERVER_NAME}__browser_status{detail}")
    return failures


def run_show_stage(
    args: argparse.Namespace, artifact_dir: Path
) -> tuple[dict[str, Any], list[str]]:
    """Stage 1: the registered config is sane."""
    show = run_openclaw(
        args.openclaw_bin,
        ["mcp", "show", SERVER_NAME, "--json"],
        artifact_dir,
        "mcp-show",
        args.openclaw_dir,
    )
    if show.returncode != 0:
        failures = [
            f"openclaw mcp show {SERVER_NAME} failed (rc={show.returncode}); "
            "is sky_cua registered? Run scripts/install_mcp_server.py --host openclaw"
        ]
        return {"ok": False, "returncode": show.returncode}, failures
    try:
        failures = check_show_config(parse_json_output(show.stdout, "mcp show"))
    except ValueError as error:
        failures = [str(error)]
    return {"ok": not failures, "failures": failures}, failures


def run_probe_stage(
    args: argparse.Namespace, artifact_dir: Path
) -> tuple[dict[str, Any], list[str]]:
    """Stage 2: OpenClaw can spawn the server and sees the required tools.

    Probe spawns a fresh client; right after an install or `mcp reload` it can
    lose a one-off race against concurrent sky-cua sessions, so retry before
    declaring the deployment broken.
    """
    failures: list[str] = []
    attempt = 0
    for attempt in range(1, PROBE_ATTEMPTS + 1):
        probe = run_openclaw(
            args.openclaw_bin,
            ["mcp", "probe", SERVER_NAME, "--json"],
            artifact_dir,
            f"mcp-probe-{attempt}",
            args.openclaw_dir,
        )
        if probe.returncode != 0:
            failures = [f"openclaw mcp probe {SERVER_NAME} failed (rc={probe.returncode})"]
        else:
            try:
                failures = check_probe_result(parse_json_output(probe.stdout, "mcp probe"))
            except ValueError as error:
                failures = [str(error)]
        if not failures:
            break
        if attempt < PROBE_ATTEMPTS:
            time.sleep(PROBE_RETRY_DELAY_SECONDS)
    return {"ok": not failures, "failures": failures, "attempts": attempt}, failures


def run_agent_turn_stage(
    args: argparse.Namespace, artifact_dir: Path
) -> tuple[dict[str, Any], list[str]]:
    """Stage 3: a real agent turn can see and execute the tools."""
    agent_args = ["agent", "--message", AGENT_TURN_PROMPT, "--json"]
    if args.agent:
        agent_args.extend(["--agent", args.agent])
    if args.session_key:
        agent_args.extend(["--session-key", args.session_key])
    turn = run_openclaw(
        args.openclaw_bin,
        agent_args,
        artifact_dir,
        "agent-turn",
        args.openclaw_dir,
        timeout=AGENT_TURN_TIMEOUT_SECONDS,
        extra_env=gateway_auth_environment(args.openclaw_dir),
    )
    report, tool_result_seen = scan_agent_turn_stdout(turn.stdout)
    if turn.returncode == TIMEOUT_RETURNCODE:
        failures = [f"agent turn timed out after {AGENT_TURN_TIMEOUT_SECONDS} seconds"]
        return {
            "ok": False,
            "timeout": True,
            "returncode": TIMEOUT_RETURNCODE,
            "failures": failures,
            "report": report,
            "tool_result_seen": tool_result_seen,
        }, failures
    failures = check_agent_report(report)
    if not tool_result_seen:
        failures.append(
            f"agent turn transcript did not show a completed {AGENT_TURN_TOOL_NAME} result"
        )
    if turn.returncode != 0:
        failures.append(f"openclaw agent exited with rc={turn.returncode}")
    return {
        "ok": not failures,
        "failures": failures,
        "report": report,
        "tool_result_seen": tool_result_seen,
    }, failures


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Verify OpenClaw can use the deployed sky-cua MCP server."
    )
    parser.add_argument(
        "--openclaw-bin",
        default="openclaw",
        help="OpenClaw CLI binary (default: openclaw).",
    )
    parser.add_argument(
        "--openclaw-dir",
        type=Path,
        default=None,
        help="OpenClaw state directory override (sets OPENCLAW_STATE_DIR/CONFIG_PATH).",
    )
    parser.add_argument(
        "--agent-turn",
        action="store_true",
        help="Also run one live agent turn through the Gateway (costs a model call).",
    )
    parser.add_argument(
        "--agent",
        default=None,
        help="Agent id for the live turn (default: OpenClaw's routing default).",
    )
    parser.add_argument(
        "--session-key",
        default=None,
        help=(
            "Session key for the live turn. Defaults to a fresh per-run key: "
            "OpenClaw pins a codex thread per session, so reusing a key would "
            "resume a thread whose MCP server state predates this deployment."
        ),
    )
    args = parser.parse_args()

    artifact_dir = make_artifact_dir("openclaw", "mcp")
    if args.session_key is None:
        args.session_key = f"sky-cua-mcp-smoke-{artifact_dir.name}"

    stages: dict[str, Any] = {}
    failures: list[str] = []
    stages["show"], stage_failures = run_show_stage(args, artifact_dir)
    failures.extend(stage_failures)
    stages["probe"], stage_failures = run_probe_stage(args, artifact_dir)
    failures.extend(stage_failures)
    if args.agent_turn:
        stages["agent_turn"], stage_failures = run_agent_turn_stage(args, artifact_dir)
        failures.extend(stage_failures)

    result: dict[str, Any] = {
        "server": SERVER_NAME,
        "artifact_dir": str(artifact_dir),
        "stages": stages,
        "ok": not failures,
    }
    (artifact_dir / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")

    if failures:
        print(f"openclaw sky-cua MCP smoke FAILED: {artifact_dir}", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    skipped = "" if args.agent_turn else " (agent turn skipped; pass --agent-turn to include it)"
    print(f"openclaw sky-cua MCP smoke passed: {artifact_dir}{skipped}")
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
