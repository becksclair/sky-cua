#!/usr/bin/env python3
"""Shared helpers for agent MCP smoke harnesses (OpenCode and Pi)."""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import time
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]


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
) -> subprocess.CompletedProcess[str]:
    """Invoke an agent CLI with a prompt and capture output."""
    stdout_path = artifact_dir / f"{agent}.stdout.log"
    stderr_path = artifact_dir / f"{agent}.stderr.log"

    if agent == "opencode":
        # OpenCode requires a pseudo-TTY to produce output when invoked
        # non-interactively. Use `script` to provide one.
        argv = [
            "script",
            "-q",
            "-c",
            f"opencode run {shlex.quote(prompt)}",
            "/dev/null",
        ]
    elif agent == "pi":
        argv = ["pi", "-p", prompt]
    else:
        raise ValueError(f"unknown agent: {agent}")

    # Forward the Fireworks API key when available so Pi can use the
    # same model endpoint as OpenCode (firepass/accounts/fireworks/...).
    env = os.environ.copy()
    fireworks_key = os.environ.get("FIREWORKS_API_KEY")
    if fireworks_key:
        env["FIREWORKS_API_KEY"] = fireworks_key

    with (
        stdout_path.open("w", encoding="utf-8") as stdout,
        stderr_path.open("w", encoding="utf-8") as stderr,
    ):
        try:
            proc = subprocess.run(
                argv,
                cwd=cwd or REPO_ROOT,
                stdout=stdout,
                stderr=stderr,
                text=True,
                timeout=timeout,
                env=env,
            )
        except subprocess.TimeoutExpired:
            proc = subprocess.CompletedProcess(argv, returncode=-9, stdout="", stderr="")

    return proc


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
