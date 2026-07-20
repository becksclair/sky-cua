"""Transactional helpers for named OpenClaw MCP CLI definitions."""

from __future__ import annotations

import json
import shlex
import subprocess
from collections.abc import Callable, Mapping, Sequence
from typing import cast

CommandRunner = Callable[..., subprocess.CompletedProcess[str]]


class OpenClawCliTransactionError(RuntimeError):
    """An OpenClaw CLI snapshot or write could not be completed."""


def run_openclaw_command(
    runner: CommandRunner,
    command: list[str],
    *,
    env: dict[str, str],
    timeout: int,
) -> subprocess.CompletedProcess[str]:
    try:
        return runner(
            command,
            check=False,
            env=env,
            timeout=timeout,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise OpenClawCliTransactionError(
            f"OpenClaw command could not complete: {shlex.join(command)}: {error}"
        ) from error


def command_result_detail(result: subprocess.CompletedProcess[str]) -> str:
    detail = (result.stderr or result.stdout or "").strip()
    return f": {detail}" if detail else ""


def snapshot_servers(
    runner: CommandRunner,
    openclaw_bin: str,
    env: dict[str, str],
    names: Sequence[str],
    *,
    timeout: int,
) -> dict[str, dict[str, object] | None]:
    command = [openclaw_bin, "mcp", "show", "--json"]
    result = run_openclaw_command(runner, command, env=env, timeout=timeout)
    if result.returncode != 0:
        raise OpenClawCliTransactionError(
            f"could not snapshot OpenClaw MCP definitions: {shlex.join(command)}"
            f"{command_result_detail(result)}"
        )
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise OpenClawCliTransactionError("OpenClaw MCP snapshot was not valid JSON") from error
    if not isinstance(payload, dict):
        raise OpenClawCliTransactionError("OpenClaw MCP snapshot was not an object")
    snapshot: dict[str, dict[str, object] | None] = {}
    for name in names:
        value = payload.get(name)
        if value is not None and not isinstance(value, dict):
            raise OpenClawCliTransactionError(f"OpenClaw MCP definition {name} is not an object")
        snapshot[name] = cast(dict[str, object] | None, value)
    return snapshot


def set_server(
    runner: CommandRunner,
    openclaw_bin: str,
    name: str,
    definition: Mapping[str, object],
    env: dict[str, str],
    *,
    timeout: int,
) -> subprocess.CompletedProcess[str]:
    command = [
        openclaw_bin,
        "mcp",
        "set",
        name,
        json.dumps(definition, sort_keys=True, separators=(",", ":")),
    ]
    return run_openclaw_command(runner, command, env=env, timeout=timeout)


def restore_servers(
    runner: CommandRunner,
    openclaw_bin: str,
    env: dict[str, str],
    names: Sequence[str],
    snapshots: Mapping[str, dict[str, object] | None],
    *,
    timeout: int,
) -> list[str]:
    failures: list[str] = []
    snapshot_known = True
    try:
        current = snapshot_servers(runner, openclaw_bin, env, names, timeout=timeout)
    except OpenClawCliTransactionError:
        snapshot_known = False
        current = dict.fromkeys(names)
    for name in reversed(names):
        original = snapshots[name]
        if snapshot_known and current.get(name) == original:
            continue
        try:
            if original is None:
                result = run_openclaw_command(
                    runner,
                    [openclaw_bin, "mcp", "unset", name],
                    env=env,
                    timeout=timeout,
                )
            else:
                result = set_server(
                    runner,
                    openclaw_bin,
                    name,
                    original,
                    env,
                    timeout=timeout,
                )
        except OpenClawCliTransactionError:
            failures.append(name)
            continue
        if result.returncode != 0:
            failures.append(name)
    return failures
