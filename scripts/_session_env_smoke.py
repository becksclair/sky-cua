from __future__ import annotations

import json
import os
import shutil
import subprocess
from collections.abc import Iterable
from pathlib import Path
from typing import Any

TITLE = "sky-cua session env smoke"
SUBMITTED_VALUE = "session-env-ok"
GRAPHICAL_SESSION_ENV_KEYS = [
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
]


def import_current_env_to_systemd() -> None:
    keys = [key for key in [*GRAPHICAL_SESSION_ENV_KEYS, "PATH"] if os.environ.get(key)]
    if not keys:
        return
    subprocess.run(
        ["systemctl", "--user", "import-environment", *keys],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )


def stripped_desktop_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = {key: value for key, value in os.environ.items() if key not in GRAPHICAL_SESSION_ENV_KEYS}
    for key in GRAPHICAL_SESSION_ENV_KEYS:
        env[key] = ""
    path_parts = ["/tmp", "/usr/bin", "/bin"]
    if codex_path := shutil.which("codex"):
        codex_dir = str(Path(codex_path).parent)
        if codex_dir not in path_parts:
            path_parts.insert(0, codex_dir)
    env["PATH"] = os.pathsep.join(path_parts)
    if extra:
        env.update(extra)
    return env


def run_session_env_dialog() -> subprocess.Popen[str]:
    return subprocess.Popen(
        [
            "zenity",
            "--entry",
            f"--title={TITLE}",
            "--text=sky-cua session env smoke",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=dict(os.environ),
    )


def session_env_prompt() -> str:
    return f"""
Goal: use computer-use to operate the visible zenity entry dialog titled `{TITLE}` after first checking that detached Linux session-env repair is visible.

Required workflow:
- Use the computer-use MCP tools directly.
- Call `doctor` before app interaction and inspect its structured `session_env` report or text summary.
- Confirm that session environment repair was visible through `SessionEnvRepaired` or a populated `session_env` report.
- Use `list_apps` and `get_app_state` to find the zenity dialog.
- Set the entry text to `{SUBMITTED_VALUE}` using computer-use, then submit the dialog.
- Return only the schema result.

Rules:
- Do not use shell commands, process inspection, xdotool, wmctrl, or clipboard tricks.
- The entry starts empty; do not clear it or send Ctrl+A before typing unless fresh state or screenshot evidence shows stale text.
- If session-env repair is not visible or the app cannot be discovered, classify the result honestly.
""".strip()


def require_session_env_doctor(report: dict[str, Any], artifact_dir: Path) -> None:
    session_env = report.get("session_env") or {}
    repaired = session_env.get("repaired") or []
    path_changed = bool(session_env.get("path_changed"))
    if not repaired and not path_changed:
        raise RuntimeError(
            "doctor report did not show session-env repair.\n"
            f"report={json.dumps(report, indent=2, sort_keys=True)[:8000]}\n"
            f"inspect {artifact_dir}"
        )
    final_path = session_env.get("final_path") or ""
    if "/usr/bin" not in final_path or "/bin" not in final_path:
        raise RuntimeError(
            "doctor report did not show normalized PATH defaults.\n"
            f"session_env={json.dumps(session_env, indent=2, sort_keys=True)}\n"
            f"inspect {artifact_dir}"
        )


def require_session_env_transcript(items: Iterable[dict[str, Any]], artifact_dir: Path) -> None:
    items = list(items)
    saw_repair = any(
        _contains_string(item, "SessionEnvRepaired") or _contains_string(item, "session_env")
        for item in items
    )
    if not saw_repair:
        sample = json.dumps(items[-5:], indent=2, sort_keys=True)[:4000]
        raise RuntimeError(
            "agent transcript did not prove session-env repair was visible; "
            f"inspect {artifact_dir}\n{sample}"
        )


def require_submitted_value(dialog: subprocess.Popen[str], artifact_dir: Path) -> None:
    if dialog.poll() is None:
        try:
            dialog.wait(timeout=10)
        except subprocess.TimeoutExpired as error:
            dialog.terminate()
            try:
                dialog.wait(timeout=5)
            except subprocess.TimeoutExpired:
                dialog.kill()
            raise SystemExit(
                f"session env smoke did not submit dialog; inspect {artifact_dir}"
            ) from error
    stdout, stderr = dialog.communicate(timeout=5)
    if dialog.returncode != 0 or stdout.strip() != SUBMITTED_VALUE:
        raise SystemExit(
            f"expected submitted value {SUBMITTED_VALUE!r}, got returncode={dialog.returncode} "
            f"stdout={stdout!r} stderr={stderr!r}; inspect {artifact_dir}"
        )


def close_dialog_if_running(dialog: subprocess.Popen[str]) -> None:
    if dialog.poll() is None:
        dialog.terminate()
        try:
            dialog.wait(timeout=2)
        except subprocess.TimeoutExpired:
            dialog.kill()


def _contains_string(value: Any, needle: str) -> bool:
    if isinstance(value, str):
        return needle in value
    if isinstance(value, dict):
        return any(_contains_string(item, needle) for item in value.values())
    if isinstance(value, list):
        return any(_contains_string(item, needle) for item in value)
    return False
