#!/usr/bin/env python3
"""Post-deploy smoke verifying OpenClaw can use the sky-cua MCP server.

Stages:
  1. show   - `openclaw mcp show sky_cua --json`: the registration exists, the
              client binary is present, and the codex approval mode will not
              silently block tool calls in unattended agent turns.
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
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from _agent_mcp_smoke import make_artifact_dir

SERVER_NAME = "sky_cua"
SMOKE_REPORT_KEY = "sky_cua_smoke"
COMMAND_TIMEOUT_SECONDS = 60
AGENT_TURN_TIMEOUT_SECONDS = 300
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
    env = os.environ.copy()
    if openclaw_dir is not None:
        env["OPENCLAW_STATE_DIR"] = str(openclaw_dir)
        env["OPENCLAW_CONFIG_PATH"] = str(openclaw_dir / "openclaw.json")
    if extra_env:
        env.update(extra_env)
    proc = subprocess.run(
        [openclaw_bin, *args],
        capture_output=True,
        text=True,
        timeout=timeout,
        env=env,
        check=False,
    )
    (artifact_dir / f"{log_name}.stdout.log").write_text(proc.stdout, encoding="utf-8")
    (artifact_dir / f"{log_name}.stderr.log").write_text(proc.stderr, encoding="utf-8")
    return proc


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
    if approval_mode != "auto":
        failures.append(
            f"codex.defaultToolsApprovalMode is {approval_mode!r}, expected 'auto'. "
            "Other modes defer MCP tool calls to the codex app-server approval "
            "policy; with approvalPolicy 'never' (the unattended default) the "
            "tools list but every call is blocked during agent turns. "
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


def find_json_object_with_key(text: str, key: str) -> dict[str, Any] | None:
    """Find the last JSON object in text that contains key at any depth."""
    decoder = json.JSONDecoder()
    found: dict[str, Any] | None = None
    index = text.find("{")
    while index != -1:
        try:
            candidate, end = decoder.raw_decode(text, index)
        except json.JSONDecodeError:
            index = text.find("{", index + 1)
            continue
        if isinstance(candidate, dict) and _contains_key(candidate, key):
            found = candidate
        index = text.find("{", end)
    return found


def _contains_key(value: object, key: str) -> bool:
    if isinstance(value, dict):
        if key in value:
            return True
        return any(_contains_key(child, key) for child in value.values())
    if isinstance(value, list):
        return any(_contains_key(child, key) for child in value)
    if isinstance(value, str):
        return key in value
    return False


def extract_smoke_report(stdout: str) -> dict[str, Any] | None:
    """Extract the agent's structured smoke report from `openclaw agent --json`.

    The report may appear as a nested object or embedded in a reply-text
    string field, so string fields containing the marker are re-scanned.
    """
    container = find_json_object_with_key(stdout, SMOKE_REPORT_KEY)
    if container is None:
        return None
    return _dig_smoke_report(container)


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
        nested = find_json_object_with_key(value, SMOKE_REPORT_KEY)
        if nested is not None and nested is not value:
            return _dig_smoke_report(nested)
    return None


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

    # Stage 1: registered config is sane.
    show = run_openclaw(
        args.openclaw_bin,
        ["mcp", "show", SERVER_NAME, "--json"],
        artifact_dir,
        "mcp-show",
        args.openclaw_dir,
    )
    if show.returncode != 0:
        failures.append(
            f"openclaw mcp show {SERVER_NAME} failed (rc={show.returncode}); "
            "is sky_cua registered? Run scripts/install_mcp_server.py --host openclaw"
        )
        stages["show"] = {"ok": False, "returncode": show.returncode}
    else:
        try:
            config = parse_json_output(show.stdout, "mcp show")
            stage_failures = check_show_config(config)
        except ValueError as error:
            stage_failures = [str(error)]
        failures.extend(stage_failures)
        stages["show"] = {"ok": not stage_failures, "failures": stage_failures}

    # Stage 2: OpenClaw can spawn the server and sees the required tools.
    # Probe spawns a fresh client; right after an install or `mcp reload` it
    # can lose a one-off race against concurrent sky-cua sessions, so retry
    # a couple of times before declaring the deployment broken.
    stage_failures = []
    attempts = 0
    for attempt in range(1, PROBE_ATTEMPTS + 1):
        attempts = attempt
        probe = run_openclaw(
            args.openclaw_bin,
            ["mcp", "probe", SERVER_NAME, "--json"],
            artifact_dir,
            f"mcp-probe-{attempt}",
            args.openclaw_dir,
        )
        if probe.returncode != 0:
            stage_failures = [f"openclaw mcp probe {SERVER_NAME} failed (rc={probe.returncode})"]
        else:
            try:
                probe_result = parse_json_output(probe.stdout, "mcp probe")
                stage_failures = check_probe_result(probe_result)
            except ValueError as error:
                stage_failures = [str(error)]
        if not stage_failures:
            break
        if attempt < PROBE_ATTEMPTS:
            time.sleep(PROBE_RETRY_DELAY_SECONDS)
    failures.extend(stage_failures)
    stages["probe"] = {
        "ok": not stage_failures,
        "failures": stage_failures,
        "attempts": attempts,
    }

    # Stage 3 (optional): a real agent turn can see and execute the tools.
    if args.agent_turn:
        agent_args = ["agent", "--message", AGENT_TURN_PROMPT, "--json"]
        if args.agent:
            agent_args.extend(["--agent", args.agent])
        if args.session_key:
            agent_args.extend(["--session-key", args.session_key])
        try:
            turn = run_openclaw(
                args.openclaw_bin,
                agent_args,
                artifact_dir,
                "agent-turn",
                args.openclaw_dir,
                timeout=AGENT_TURN_TIMEOUT_SECONDS,
                extra_env=gateway_auth_environment(args.openclaw_dir),
            )
        except subprocess.TimeoutExpired:
            failures.append(f"agent turn timed out after {AGENT_TURN_TIMEOUT_SECONDS} seconds")
            stages["agent_turn"] = {"ok": False, "timeout": True}
        else:
            report = extract_smoke_report(turn.stdout)
            stage_failures = check_agent_report(report)
            if turn.returncode != 0:
                stage_failures.append(f"openclaw agent exited with rc={turn.returncode}")
            failures.extend(stage_failures)
            stages["agent_turn"] = {
                "ok": not stage_failures,
                "failures": stage_failures,
                "report": report,
            }

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
