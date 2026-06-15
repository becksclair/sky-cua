from __future__ import annotations

import json
import os
import shutil
import subprocess
from collections.abc import Iterable
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from _plugin_bundle import (
    DEFAULT_CODEX_HOME,
    DIST_PLUGIN_ROOT,
    INSTALLED_PLUGIN_ROOT,
    REPO_ROOT,
    compat_plugin_targets_payload,
    installed_plugin_root,
    update_codex_config,
)
from _smoke_config import LIVE_SMOKE_MODEL, LIVE_SMOKE_REASONING_EFFORT

DEFAULT_MODEL = LIVE_SMOKE_MODEL
DEFAULT_REASONING_EFFORT = LIVE_SMOKE_REASONING_EFFORT
FAST_SERVICE_TIER = "fast"
COMPAT_PLUGIN_MENTION = "[@computer-use](plugin://computer-use@openai-bundled)"
LOCAL_PLUGIN_MENTION = "[@sky-cua](plugin://sky-cua@local)"
COMPUTER_USE_NAMESPACE = "mcp__computer_use__"
TOOL_VISIBILITY_SCHEMA = REPO_ROOT / "scripts" / "schemas" / "tool_visibility_result.json"
DESKTOP_E2E_EXEC_ARGS = ["--dangerously-bypass-approvals-and-sandbox"]
HOST_HARNESS_TOOL_NAMES = {
    "web.run",
    "image_gen.imagegen",
    "functions.exec_command",
    "functions.write_stdin",
    "tool_search.tool_search_tool",
    "multi_tool_use.parallel",
}
NESTED_CODEX_ENV_VARS = (
    "CODEX_CI",
    "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
    "CODEX_THREAD_ID",
)


def plugin_mention(codex_home: Path) -> str:
    """Mention for the computer-use plugin id enabled in the given home.

    Compat-first homes enable `computer-use@openai-bundled`; channel-fallback
    homes enable `sky-cua@local`. Mentioning a disabled plugin id would hand
    the model a dead reference.
    """
    if compat_plugin_targets_payload(codex_home, installed_plugin_root(codex_home)):
        return COMPAT_PLUGIN_MENTION
    return LOCAL_PLUGIN_MENTION


def with_plugin_mention(prompt: str, codex_home: Path) -> str:
    return (
        f"Use {plugin_mention(codex_home)} and its bundled computer-use skill.\n"
        "Start from a fresh `get_app_state`, inspect any returned `screenshot_path` with `view_image`,"
        " and treat the screenshot as the visual source of truth. When the tree is sparse or fallback-only,"
        " prefer compact `get_app_state` snapshots; use `element_query`, `element_limit`,"
        ' or `detail: "full"` only when the omitted fields are needed. '
        " visually focus on the target app/window inside the full screenshot while keeping click and drag"
        " coordinates in the current screenshot's pixel coordinate space. Before typing, inspect the current screenshot to confirm"
        " the target field and whether it already contains text; prefer element `value` or `text.content`"
        " readback when present, and clear or select stale contents before"
        " `type_text` when replacement is intended. Look for visible text-entry controls, action clusters,"
        " and right-click/context-menu paths instead"
        " of waiting for perfect semantics. Re-run `get_app_state` after opening menus, submenus, dialogs,"
        " text entry, clicks, scrolls, keypresses, or after any visually meaningful action before committing"
        " to the next move. If two successive fresh"
        " screenshots show no material progress after exploratory actions, change strategy or classify the"
        " run honestly instead of looping forever.\n\n"
        f"{prompt}"
    )


def build_codex_exec_env() -> dict[str, str]:
    env = os.environ.copy()
    env.setdefault("CODEX_HOME", str(DEFAULT_CODEX_HOME))
    env.setdefault("RUST_LOG", "error")
    for key in NESTED_CODEX_ENV_VARS:
        env.pop(key, None)
    if not env.get("OPENAI_API_KEY"):
        env.pop("OPENAI_API_KEY", None)
    return env


@dataclass
class CodexExecResult:
    artifact_dir: Path
    command: list[str]
    codex_home: Path
    exit_code: int
    transcript_path: Path
    stderr_path: Path
    last_message_path: Path


def ensure_plugin_ready(artifact_dir: Path, install: bool, symlink: bool = False) -> None:
    if INSTALLED_PLUGIN_ROOT.exists():
        return
    if not install:
        raise FileNotFoundError(
            f"installed plugin not found at {INSTALLED_PLUGIN_ROOT}; rerun with --install"
        )
    subprocess.run(["python3", str(REPO_ROOT / "scripts" / "build_plugin.py")], check=True)
    subprocess.run(
        [
            "python3",
            str(REPO_ROOT / "scripts" / "install_plugin.py"),
            "--bundle-root",
            str(DIST_PLUGIN_ROOT),
            *(["--symlink"] if symlink else []),
        ],
        check=True,
    )


def prepare_chatgpt_plugin_test_home(*, artifact_dir: Path, symlink: bool = False) -> Path:
    codex_home = (artifact_dir / "codex-home").resolve()
    auth_src = DEFAULT_CODEX_HOME / "auth.json"
    if not auth_src.exists():
        raise FileNotFoundError(
            f"ChatGPT auth file not found at {auth_src}; cannot prepare ChatGPT-auth plugin test home"
        )

    codex_home.mkdir(parents=True, exist_ok=True)
    shutil.copy2(auth_src, codex_home / "auth.json")

    subprocess.run(["python3", str(REPO_ROOT / "scripts" / "build_plugin.py")], check=True)

    subprocess.run(
        [
            "python3",
            str(REPO_ROOT / "scripts" / "install_plugin.py"),
            "--bundle-root",
            str(DIST_PLUGIN_ROOT),
            "--codex-home",
            str(codex_home),
            *(["--symlink"] if symlink else []),
        ],
        check=True,
    )
    update_codex_config(
        codex_home / "config.toml",
        disable_apps=True,
        fast_service_tier=True,
        compat_enablement=compat_plugin_targets_payload(
            codex_home, installed_plugin_root(codex_home)
        ),
    )
    return codex_home


def make_artifact_dir(name: str) -> Path:
    stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    path = REPO_ROOT / "artifacts" / "codex-e2e" / name / stamp
    path.mkdir(parents=True, exist_ok=True)
    return path


def run_codex_exec(
    *,
    prompt: str,
    artifact_dir: Path,
    output_schema: Path,
    model: str = DEFAULT_MODEL,
    reasoning_effort: str = DEFAULT_REASONING_EFFORT,
    workdir: Path | None = None,
    extra_args: Iterable[str] = (),
    extra_env: dict[str, str] | None = None,
) -> CodexExecResult:
    workdir = (workdir or artifact_dir / "workdir").resolve()
    workdir.mkdir(parents=True, exist_ok=True)

    transcript_path = artifact_dir / "codex-output.jsonl"
    stderr_path = artifact_dir / "codex-stderr.txt"
    last_message_path = artifact_dir / "last-message.json"
    prompt_path = artifact_dir / "prompt.txt"
    prompt_path.write_text(prompt)

    command = [
        "codex",
        "exec",
        "--json",
        "--ephemeral",
        "--skip-git-repo-check",
        "--color",
        "never",
        "--model",
        model,
        "-c",
        f'model_reasoning_effort="{reasoning_effort}"',
        "-c",
        "features.fast_mode=true",
        "-c",
        f'service_tier="{FAST_SERVICE_TIER}"',
        "--output-schema",
        str(output_schema),
        "--output-last-message",
        str(last_message_path),
        "--cd",
        str(workdir),
        *extra_args,
        prompt,
    ]

    env = build_codex_exec_env()
    if extra_env:
        env.update(extra_env)
    codex_home = Path(env["CODEX_HOME"]).resolve()

    completed = subprocess.run(
        command,
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
    )
    transcript_path.write_text(completed.stdout)
    stderr_path.write_text(completed.stderr)
    metadata = {
        "command": command,
        "codex_home": str(codex_home),
        "exit_code": completed.returncode,
        "transcript_path": str(transcript_path),
        "stderr_path": str(stderr_path),
        "last_message_path": str(last_message_path),
        "workdir": str(workdir),
    }
    (artifact_dir / "metadata.json").write_text(json.dumps(metadata, indent=2))
    return CodexExecResult(
        artifact_dir=artifact_dir,
        command=command,
        codex_home=codex_home,
        exit_code=completed.returncode,
        transcript_path=transcript_path,
        stderr_path=stderr_path,
        last_message_path=last_message_path,
    )


def transcript_mcp_tool_calls(transcript_path: Path) -> list[dict]:
    calls: list[dict] = []
    for raw_line in transcript_path.read_text().splitlines():
        if not raw_line.strip():
            continue
        try:
            event = json.loads(raw_line)
        except json.JSONDecodeError:
            continue
        if event.get("type") not in {"item.started", "item.completed"}:
            continue
        item = event.get("item", {})
        if item.get("type") != "mcp_tool_call":
            continue
        calls.append(item)
    return calls


def probe_visible_tool_names(
    *, artifact_dir: Path, model: str = DEFAULT_MODEL
) -> tuple[list[str], CodexExecResult]:
    probe_artifact_dir = artifact_dir / "tool-visibility-probe"
    probe_artifact_dir.mkdir(parents=True, exist_ok=True)
    prompt = (
        "Do not call any tools. Return only JSON matching the schema.\n"
        "List the exact tool names currently available to you in this run.\n"
        "Preserve spelling exactly as shown in your available tool list."
    )
    result = run_codex_exec(
        prompt=prompt,
        artifact_dir=probe_artifact_dir,
        output_schema=TOOL_VISIBILITY_SCHEMA,
        model=model,
    )
    if result.exit_code != 0 or not result.last_message_path.exists():
        return [], result
    message = read_last_message(result.last_message_path)
    tool_names = message.get("tool_names", [])
    if not isinstance(tool_names, list):
        return [], result
    return [name for name in tool_names if isinstance(name, str)], result


def read_codex_auth_mode(codex_home: Path) -> str | None:
    auth_path = codex_home / "auth.json"
    if not auth_path.exists():
        return None
    try:
        payload = json.loads(auth_path.read_text())
    except json.JSONDecodeError:
        return None
    auth_mode = payload.get("auth_mode")
    return auth_mode if isinstance(auth_mode, str) else None


def tool_names_match_host_harness(tool_names: list[str]) -> bool:
    present = {name for name in tool_names if name in HOST_HARNESS_TOOL_NAMES}
    return len(present) >= 3


def require_computer_use_tool_call(
    transcript_path: Path, *, artifact_dir: Path | None = None, model: str = DEFAULT_MODEL
) -> None:
    calls = transcript_mcp_tool_calls(transcript_path)
    if not any(call.get("server") == "computer-use" for call in calls):
        if artifact_dir is None:
            raise RuntimeError(
                "codex exec did not issue any MCP calls against the installed computer-use plugin"
            )

        tool_names, probe_result = probe_visible_tool_names(artifact_dir=artifact_dir, model=model)
        tool_sample = ", ".join(tool_names[:12]) if tool_names else "none"
        probe_hint = (
            f"inspect {probe_result.artifact_dir}" if probe_result else f"inspect {artifact_dir}"
        )
        auth_mode = read_codex_auth_mode(probe_result.codex_home)
        if any(name.startswith(COMPUTER_USE_NAMESPACE) for name in tool_names):
            raise RuntimeError(
                "codex exec could see the installed computer-use tools but still never called them; "
                f"visible tools included {tool_sample}. {probe_hint}"
            )
        if tool_names_match_host_harness(tool_names) or any(
            name.startswith("functions.") or name.startswith("mcp__codex_apps__")
            for name in tool_names
        ):
            auth_hint = ""
            if auth_mode == "chatgpt":
                auth_hint = (
                    " The current CODEX_HOME is using ChatGPT auth, so Codex is talking to "
                    "chatgpt.com/backend-api/codex rather than api.openai.com/v1."
                )
            raise RuntimeError(
                "codex exec could not see any installed computer-use tools. "
                f"Visible tools were {tool_sample}. This run is seeing the host harness tool surface "
                "instead of the local plugin set, so no remote-control prompt will appear."
                f"{auth_hint} "
                "Use the dedicated ChatGPT-auth test home with features.apps=false for plugin E2E, "
                "or fall back to direct MCP stdio when you only need backend proof. "
                f"{probe_hint}"
            )
        raise RuntimeError(
            "codex exec did not issue any MCP calls against the installed computer-use plugin, "
            f"and the follow-up tool-visibility probe saw: {tool_sample}. {probe_hint}"
        )


def read_last_message(path: Path) -> dict:
    return json.loads(path.read_text())
