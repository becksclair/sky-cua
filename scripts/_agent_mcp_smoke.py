#!/usr/bin/env python3
"""Shared helpers for agent MCP smoke harnesses (OpenCode, Pi, Claude, OpenClaw)."""

from __future__ import annotations

import json
import os
import shlex
import shutil
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

from _smoke_config import env_flag
from deploy_freshness import assert_runtime_fresh, deployed_client_path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PI_SMOKE_MODEL = "opencode-go/kimi-k2.7-code"
TOOL_FAILURE_STATUSES = {"canceled", "cancelled", "error", "failed", "failure", "timeout"}
RESULT_PAYLOAD_KEYS = (
    "result",
    "content",
    "output",
    "structuredContent",
    "structured_content",
    "toolResult",
    "tool_result",
)
NESTED_EVENT_KEYS = (
    "arguments",
    "args",
    "details",
    "input",
    "metadata",
    "meta",
    "parameters",
    "part",
    "state",
    "toolCall",
    "tool_call",
)
AGENT_ENV_ALLOWLIST = {
    "CODEX_COMPUTER_USE_COSMIC_HELPER",
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "HOME",
    "LANG",
    "LOGNAME",
    "PATH",
    "SHELL",
    "TERM",
    "USER",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "YDOTOOL_SOCKET",
}
SKY_CUA_RUNTIME_ENV_ALLOWLIST = {
    "SKY_CUA_AGENT_CURSOR",
    "SKY_CUA_BROWSER",
    "SKY_CUA_BROWSER_EVAL",
    "SKY_CUA_COSMIC_HELPER",
    "SKY_CUA_INPUT_BACKEND",
    "SKY_CUA_MODEL_SCREENSHOT_FORMAT",
    "SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY",
    "SKY_CUA_MODEL_SCREENSHOT_MAX_HEIGHT",
    "SKY_CUA_MODEL_SCREENSHOT_MAX_WIDTH",
    "SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY",
    "SKY_CUA_OVERLAY_BACKEND",
    "SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE",
    "SKY_CUA_OVERLAY_HOST_PATH",
    "SKY_CUA_OVERLAY_HOST_TCP_ADDR",
    "SKY_CUA_ADB",
    "SKY_CUA_PHONE",
    "SKY_CUA_PHONE_BACKEND",
    "SKY_CUA_PHONE_COMPANION",
    "SKY_CUA_PHONE_COMPANION_APK",
    "SKY_CUA_PHONE_COMPANION_AUTO_INSTALL",
    "SKY_CUA_PHONE_COMPANION_OPERATOR_MODE",
    "SKY_CUA_PORTAL_EIS",
    "SKY_CUA_PRESENCE_ENABLED",
    "SKY_CUA_PRESENCE_IDLE_RELEASE_SECS",
    "SKY_CUA_PRESENCE_INHIBIT_LOCK",
    "SKY_CUA_PRESENCE_INHIBIT_SUSPEND",
    "SKY_CUA_PRESENCE_RELOCK",
    "SKY_CUA_PRESENCE_UNLOCK",
    "SKY_CUA_REPO_ROOT",
    "SKY_CUA_SCREENSHOT_CURSOR",
    "SKY_CUA_SERVICE_PATH",
    "SKY_CUA_SERVICE_TCP_ADDR",
    "SKY_CUA_SERVICE_SOCKET_PATH",
}


def make_artifact_dir(agent_name: str, profile_name: str) -> Path:
    timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    artifact_dir = REPO_ROOT / "artifacts" / f"{agent_name}-{profile_name}-smoke" / timestamp
    artifact_dir.mkdir(parents=True, exist_ok=True)
    return artifact_dir


def run_agent(
    agent: str,
    prompt: str,
    artifact_dir: Path,
    cwd: Path | None = None,
    timeout: float = 300,
    model: str | None = None,
    gate_deploy: bool = True,
) -> subprocess.CompletedProcess[str]:
    """Invoke an agent CLI with a prompt and capture output."""
    # Deploy-freshness gate: the agent reaches sky-cua through its own MCP config,
    # which points at the locally deployed runtime — refuse to run against a stale
    # deploy (the live-test cua-deploy gate). gate_deploy=False for unit tests that
    # fake the agent subprocess.
    if gate_deploy:
        assert_runtime_fresh(deployed_client_path())
    stdout_path = artifact_dir / f"{agent}.stdout.log"
    stderr_path = artifact_dir / f"{agent}.stderr.log"
    selected_model: str | None = None

    if agent == "opencode":
        # OpenCode requires a pseudo-TTY to produce output when invoked
        # non-interactively. Use `script` to provide one.
        model_arg = f" --model {shlex.quote(model)}" if model else ""
        command = f"opencode run --format json{model_arg} {shlex.quote(prompt)}"
        argv = [
            "script",
            "-q",
            "-e",
            "-c",
            command,
            "/dev/null",
        ]
    elif agent == "pi":
        pi_model = model or os.environ.get("SKY_CUA_SMOKE_PI_MODEL", DEFAULT_PI_SMOKE_MODEL)
        selected_model = pi_model
        argv = [
            "pi",
            "--model",
            pi_model,
            "--no-session",
            "--no-builtin-tools",
            "--mode",
            "json",
            "-p",
            prompt,
        ]
    elif agent == "openclaw":
        # `openclaw agent` requires an explicit session target, and OpenClaw
        # resumes one codex thread per session key, so each smoke run gets a
        # fresh key instead of resuming a stale thread.
        session_key = f"sky-cua-agent-smoke-{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}"
        argv = ["openclaw", "agent", "--session-key", session_key, "--message", prompt, "--json"]
    elif agent == "claude":
        claude_bin = shutil.which("claude") or shutil.which("openclaude")
        if claude_bin is None:
            raise FileNotFoundError("neither claude nor openclaude is on PATH")
        claude_model = os.environ.get("SKY_CUA_SMOKE_CLAUDE_MODEL", "claude-sonnet-4-6")
        argv = [
            claude_bin,
            "--dangerously-skip-permissions",
            "--model",
            claude_model,
            "-p",
            prompt,
        ]
    else:
        raise ValueError(f"unknown agent: {agent}")

    env = agent_environment()
    for key in model_auth_environment_keys(agent, selected_model):
        value = os.environ.get(key)
        if value:
            env[key] = value

    keep_raw_agent_log = env_flag("SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG")
    stdout_capture_path = stdout_path
    raw_stdout_path: Path | None = None
    if agent in {"opencode", "pi"} and not keep_raw_agent_log:
        with tempfile.NamedTemporaryFile(
            prefix=f"{agent}.stdout.",
            suffix=".raw.jsonl",
            dir=artifact_dir,
            delete=False,
        ) as raw_file:
            raw_stdout_path = Path(raw_file.name)
        stdout_capture_path = raw_stdout_path

    # opencode loads a project-level opencode.json/opencode.jsonc from its working
    # directory, which in a repo worktree can override the sky_cua MCP server to a
    # different binary (e.g. a sibling main checkout) that lacks the tools under
    # test. Run opencode from a neutral directory when the caller did not pin a
    # cwd, so it uses the user's global config — whose sky_cua points at the
    # deployed runtime the deploy-freshness gate validates. Other agents keep the
    # repo cwd.
    run_cwd = cwd or REPO_ROOT
    neutral_cwd: Path | None = None
    if agent == "opencode" and cwd is None:
        neutral_cwd = Path(tempfile.mkdtemp(prefix="sky-cua-opencode-cwd-"))
        run_cwd = neutral_cwd

    try:
        with (
            stdout_capture_path.open("w", encoding="utf-8") as stdout,
            stderr_path.open("w", encoding="utf-8") as stderr,
        ):
            try:
                proc = subprocess.run(
                    argv,
                    cwd=run_cwd,
                    stdout=stdout,
                    stderr=stderr,
                    text=True,
                    timeout=timeout,
                    env=env,
                )
            except subprocess.TimeoutExpired:
                proc = subprocess.CompletedProcess(argv, returncode=-9, stdout="", stderr="")
    finally:
        if neutral_cwd is not None:
            shutil.rmtree(neutral_cwd, ignore_errors=True)

    if raw_stdout_path is not None:
        redact_pi_json_stdout(raw_stdout_path, stdout_path)
        raw_stdout_path.unlink(missing_ok=True)

    return proc


def agent_environment() -> dict[str, str]:
    env = {
        key: value
        for key, value in os.environ.items()
        if key in AGENT_ENV_ALLOWLIST
        or key in SKY_CUA_RUNTIME_ENV_ALLOWLIST
        or key.startswith("LC_")
    }
    return env


def model_auth_environment_keys(agent: str, selected_model: str | None) -> tuple[str, ...]:
    if agent == "opencode":
        return (
            "CONTEXT7_API_KEY",
            "FIREWORKS_API_KEY",
            "MOONSHOT_API_KEY",
            "OPENAI_API_KEY",
            "OPENCODE_API_KEY",
        )
    if agent == "openclaw":
        return (
            "OPENCLAW_CONFIG_PATH",
            "OPENCLAW_GATEWAY_PASSWORD",
            "OPENCLAW_GATEWAY_TOKEN",
            "OPENCLAW_STATE_DIR",
        )
    if agent != "pi" or selected_model is None:
        return ()
    provider = selected_model.split("/", 1)[0].lower()
    shared_keys = ("CONTEXT7_API_KEY",)
    if provider in {"opencode", "opencode-go"}:
        return (*shared_keys, "OPENCODE_API_KEY")
    if provider == "fireworks":
        return (*shared_keys, "FIREWORKS_API_KEY")
    if provider == "openai":
        return (*shared_keys, "OPENAI_API_KEY")
    if provider == "moonshot":
        return (*shared_keys, "MOONSHOT_API_KEY")
    return shared_keys


def redact_pi_json_stdout(raw_path: Path, output_path: Path) -> None:
    if not raw_path.exists():
        output_path.write_text("", encoding="utf-8")
        return
    with (
        raw_path.open(encoding="utf-8") as raw_file,
        output_path.open("w", encoding="utf-8") as output_file,
    ):
        for line in raw_file:
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(event, dict):
                continue
            redacted = redacted_pi_event(event)
            if redacted is not None:
                output_file.write(json.dumps(redacted, sort_keys=True) + "\n")


def redacted_pi_event(event: dict[str, Any]) -> dict[str, Any] | None:
    redacted: dict[str, Any] = {}
    event_type = event.get("type")
    if isinstance(event_type, str):
        redacted["type"] = event_type
    for key in ("toolName", "tool_name", "tool", "name", "server", "server_name", "mcp_server"):
        value = event.get(key)
        if isinstance(value, str) and safe_tool_identity_field(key, value):
            redacted[key] = value
    for key in ("arguments", "args", "input", "parameters"):
        promote_tool_identity(event.get(key), redacted)
    for key in ("isError", "is_error", "status", "phase", "state"):
        value = event.get(key)
        if isinstance(value, (bool, str)):
            redacted[key] = value
    dismissed = dismissed_from_mapping(event)
    if dismissed is not None:
        redacted["dismissed"] = dismissed
    if any(key in event and event[key] is not None for key in RESULT_PAYLOAD_KEYS):
        redacted["result"] = {"redacted": True}
        if any(payload_declares_failure(event.get(key)) for key in RESULT_PAYLOAD_KEYS):
            redacted["result_declares_failure"] = True
    if event_declares_failure(event):
        redacted["result_declares_failure"] = True
    message = redacted_assistant_message(event.get("message"))
    if message is not None:
        redacted["message"] = message
    tool_results = event.get("toolResults")
    if isinstance(tool_results, list):
        redacted_tool_results = [
            redacted_pi_event(item) for item in tool_results if isinstance(item, dict)
        ]
        compact_tool_results = [item for item in redacted_tool_results if item is not None]
        if compact_tool_results:
            redacted["toolResults"] = compact_tool_results
    for key in NESTED_EVENT_KEYS:
        value = event.get(key)
        if isinstance(value, dict):
            nested = redacted_pi_event(value)
            if nested is not None:
                redacted[key] = nested
            continue
        if isinstance(value, list):
            nested_items = [redacted_pi_event(item) for item in value if isinstance(item, dict)]
            compact_items = [item for item in nested_items if item is not None]
            if compact_items:
                redacted[key] = compact_items
    if set(redacted) == {"type"}:
        return None
    return redacted if redacted else None


def promote_tool_identity(value: object, redacted: dict[str, Any]) -> None:
    if not isinstance(value, dict):
        return
    for source_key, target_key in (
        ("toolName", "toolName"),
        ("tool_name", "tool_name"),
        ("tool", "tool"),
        ("name", "name"),
        ("server", "server"),
        ("server_name", "server_name"),
        ("mcp_server", "mcp_server"),
    ):
        item = value.get(source_key)
        if (
            isinstance(item, str)
            and target_key not in redacted
            and safe_tool_identity_field(source_key, item)
        ):
            redacted[target_key] = item
    nested = value.get("toolCall") or value.get("tool_call") or value.get("details")
    if nested is not value:
        promote_tool_identity(nested, redacted)


def safe_tool_identity_field(key: str, value: str) -> bool:
    normalized_key = key.lower().replace("-", "_")
    if normalized_key in {"toolname", "tool_name", "tool", "name"}:
        if value == "mcp" or value.startswith(
            ("sky_cua_", "mcp__computer_use__", "mcp__sky-cua__", "mcp__sky_cua__")
        ):
            return True
        # Keep phone tool names regardless of the agent's namespace spelling
        # (opencode names them `sky-cua_phone_connect`, others bare/`mcp__`-
        # prefixed). Tool names are non-sensitive; results stay redacted.
        return tool_base_name(value).startswith("phone_")
    if normalized_key in {"server", "server_name", "mcp_server", "servername"}:
        return value in {"sky_cua", "sky-cua", "computer-use", "mcp__computer_use"}
    return False


def tool_base_name(value: str) -> str:
    """Strip an MCP/server namespace prefix to the bare tool name.

    Tolerant of the spellings agents use: ``mcp__<server>__<tool>``,
    ``sky-cua_<tool>`` / ``sky_cua_<tool>``, ``computer-use_<tool>``.
    """
    token = value.strip()
    if token.startswith("mcp__") and "__" in token[len("mcp__") :]:
        token = token.split("__", 2)[-1]
    for prefix in ("sky-cua_", "sky_cua_", "computer-use_", "computer_use_"):
        if token.startswith(prefix):
            return token[len(prefix) :]
    return token


def payload_declares_failure(value: object) -> bool:
    if isinstance(value, list):
        return any(payload_declares_failure(item) for item in value)
    if not isinstance(value, dict):
        return False
    if value.get("is_error") is True or value.get("isError") is True:
        return True
    error = value.get("error")
    if isinstance(error, str) and error.strip():
        return True
    if isinstance(error, (dict, list)) and error:
        return True
    for key in ("status", "phase", "state", "result"):
        item = value.get(key)
        if isinstance(item, str) and item.lower() in TOOL_FAILURE_STATUSES:
            return True
    return any(
        payload_declares_failure(value.get(key))
        for key in (*RESULT_PAYLOAD_KEYS, *NESTED_EVENT_KEYS)
    )


def event_declares_failure(event: dict[str, Any]) -> bool:
    if event.get("is_error") is True or event.get("isError") is True:
        return True
    error = event.get("error")
    if isinstance(error, str) and error.strip():
        return True
    if isinstance(error, (dict, list)) and error:
        return True
    for key in ("status", "phase", "state", "result"):
        item = event.get(key)
        if isinstance(item, str) and item.lower() in TOOL_FAILURE_STATUSES:
            return True
    return any(
        payload_declares_failure(event.get(key))
        for key in (*RESULT_PAYLOAD_KEYS, *NESTED_EVENT_KEYS)
    )


def redacted_assistant_message(message: object) -> dict[str, Any] | None:
    if not isinstance(message, dict) or message.get("role") != "assistant":
        return None
    payloads: list[dict[str, str]] = []
    content = message.get("content")
    if isinstance(content, list):
        for item in content:
            if not isinstance(item, dict) or item.get("type") != "text":
                continue
            text = item.get("text")
            if not isinstance(text, str):
                continue
            dismissed = dismissed_json_from_text(text)
            if dismissed is not None:
                payloads.append({"type": "text", "text": json.dumps({"dismissed": dismissed})})
    return {"role": "assistant", "content": payloads} if payloads else None


def dismissed_json_from_text(text: str) -> bool | None:
    for block in reversed(text.split("```json")):
        json_text = block.split("```", 1)[0] if "```" in block else block
        stripped = json_text.strip()
        if not stripped:
            continue
        dismissed = dismissed_json_from_jsonish_text(stripped)
        if dismissed is not None:
            return dismissed
    return None


def dismissed_json_from_jsonish_text(text: str) -> bool | None:
    try:
        value = json.loads(text)
    except json.JSONDecodeError:
        value = None
    if isinstance(value, dict):
        dismissed = dismissed_from_mapping(value)
        if dismissed is not None:
            return dismissed
    for line in reversed(text.strip().splitlines()):
        stripped = line.strip()
        if not stripped:
            continue
        try:
            value = json.loads(stripped)
        except json.JSONDecodeError:
            start = stripped.rfind("{")
            end = stripped.rfind("}")
            if start == -1 or end == -1 or start >= end:
                continue
            try:
                value = json.loads(stripped[start : end + 1])
            except json.JSONDecodeError:
                continue
        if isinstance(value, dict):
            dismissed = dismissed_from_mapping(value)
            if dismissed is not None:
                return dismissed
    return None


def dismissed_from_mapping(value: dict[str, Any]) -> bool | None:
    dismissed = value.get("dismissed")
    if isinstance(dismissed, bool):
        return dismissed
    if isinstance(dismissed, str) and dismissed.strip().lower() in {"true", "false"}:
        return dismissed.strip().lower() == "true"
    return None


def write_result(
    artifact_dir: Path,
    agent: str,
    proc: subprocess.CompletedProcess[str],
    dialog_alive: bool,
    extra: dict[str, Any] | None = None,
) -> dict[str, Any]:
    result: dict[str, Any] = {
        "agent": agent,
        "returncode": proc.returncode,
        "artifact_dir": str(artifact_dir),
        "stdout": str(artifact_dir / f"{agent}.stdout.log"),
        "stderr": str(artifact_dir / f"{agent}.stderr.log"),
        "dialog_alive_after_run": dialog_alive,
        "ok": not dialog_alive and proc.returncode == 0,
    }
    if extra:
        result.update(extra)

    result_path = artifact_dir / "result.json"
    result_path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result
