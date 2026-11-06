"""Merge fixed-root MCP registrations into existing Claude host configuration.

This standalone installer helper uses only the standard library. It never
changes authentication, providers, project configuration, or other servers.
"""

from __future__ import annotations

import json
import os
import stat
import tempfile
import threading
import time
from collections.abc import Mapping
from contextlib import ExitStack
from dataclasses import dataclass
from pathlib import Path
from types import TracebackType
from typing import NotRequired, TypedDict

CLAUDE_MANAGED_SERVERS = ("sky-cua", "node_repl")
_OVERRIDE_ENV_KEYS = (
    "SKY_CUA_BROWSER_CONTROL_MODE",
    "SKY_CUA_CODEX_BROWSER_SOCKET_PATH",
    "SKY_CUA_PHONE_DIRECT",
    "SKY_CUA_PHONE_DIRECT_ADVERTISED_ENDPOINT",
    "SKY_CUA_PHONE_DIRECT_ENROLLMENT_TTL_MS",
    "SKY_CUA_PHONE_DIRECT_LISTEN_ADDR",
    "SKY_CUA_PHONE_DIRECT_STATE_PATH",
)


class ClaudeConfigReport(TypedDict):
    status: str
    config_path: str
    servers: list[str]
    reason: NotRequired[str]


@dataclass(frozen=True)
class _ConfigPlan:
    path: Path
    target: Path
    text: str | None
    mode: int
    status: str
    reason: str | None = None

    def report(self) -> ClaudeConfigReport:
        result: ClaudeConfigReport = {
            "status": self.status,
            "config_path": str(self.path),
            "servers": list(CLAUDE_MANAGED_SERVERS),
        }
        if self.reason is not None:
            result["reason"] = self.reason
        return result


def _plan_config(
    path: Path, *, create: bool, servers: Mapping[str, object], reason: str
) -> _ConfigPlan:
    target = path.resolve(strict=False) if path.is_symlink() else path
    exists = target.exists()
    if not exists and not create:
        return _ConfigPlan(path, target, None, 0o600, "skipped", reason)
    if exists and not target.is_file():
        raise ValueError(f"Claude config must be a regular file: {path}")
    mode = stat.S_IMODE(target.stat().st_mode) if exists else 0o600
    current: dict[str, object] = {}
    if exists:
        try:
            parsed = json.loads(target.read_text(encoding="utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError(f"Claude config is not valid UTF-8 JSON: {path}") from error
        if not isinstance(parsed, dict):
            raise ValueError(f"Claude config root must be an object: {path}")
        current = parsed
    previous_servers = current.get("mcpServers", {})
    if not isinstance(previous_servers, dict):
        raise ValueError(f"Claude mcpServers must be an object: {path}")
    merged_servers = dict(previous_servers)
    for name, definition in servers.items():
        previous = previous_servers.get(name, {})
        if not isinstance(previous, dict):
            raise ValueError(f"Claude MCP server {name} must be an object: {path}")
        merged_definition = dict(previous)
        # Transport and launch fields are owned by this registration. Keep any
        # unrelated per-server metadata while retiring a previous HTTP launch.
        merged_definition.pop("url", None)
        merged_definition.pop("headers", None)
        assert isinstance(definition, dict)
        merged_definition.update(definition)
        merged_servers[name] = merged_definition
    merged = {**current, "mcpServers": merged_servers}
    if exists and merged == current:
        return _ConfigPlan(path, target, None, mode, "unchanged")
    return _ConfigPlan(
        path,
        target,
        json.dumps(merged, indent=2) + "\n",
        mode,
        "updated" if exists else "created",
        None if exists else "Claude Code directory exists; created its user MCP config",
    )


# Claude Code uses proper-lockfile's mkdir/mtime protocol on the configured
# (logical) path, even when the config itself is a symlink. Keep its 10-second
# stale threshold and 5-second heartbeat.
_LOCK_STALE_SECONDS = 10.0
_LOCK_HEARTBEAT_SECONDS = 5.0
_LOCK_TIMEOUT_SECONDS = 30.0
_LOCK_POLL_SECONDS = 0.1


class _ConfigLock:
    def __init__(self, config_path: Path) -> None:
        self.path = Path(f"{config_path}.lock")
        self._stamp: tuple[int, int, int] | None = None
        self._failure: str | None = None
        self._mutex = threading.Lock()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def _current_stamp(self) -> tuple[int, int, int]:
        value = self.path.stat()
        return value.st_dev, value.st_ino, value.st_mtime_ns

    def _check(self) -> None:
        if self._failure is not None:
            raise RuntimeError(self._failure)
        try:
            current = self._current_stamp()
        except OSError as error:
            raise RuntimeError(f"Claude config lock was lost: {self.path}") from error
        if current != self._stamp:
            raise RuntimeError(f"Claude config lock ownership changed: {self.path}")

    def assert_owned(self) -> None:
        with self._mutex:
            self._check()

    def _heartbeat(self) -> None:
        while not self._stop.wait(_LOCK_HEARTBEAT_SECONDS):
            with self._mutex:
                try:
                    self._check()
                    expected = self._stamp
                    os.utime(self.path, None)
                    refreshed = self._current_stamp()
                    # A contender can replace the directory between our check
                    # and utime. Never adopt that new owner's lock as ours.
                    if expected is None or refreshed[:2] != expected[:2]:
                        raise RuntimeError(f"Claude config lock ownership changed: {self.path}")
                    self._stamp = refreshed
                except (OSError, RuntimeError) as error:
                    self._failure = f"Claude config lock heartbeat failed: {self.path}: {error}"
                    return

    def __enter__(self) -> _ConfigLock:
        deadline = time.monotonic() + _LOCK_TIMEOUT_SECONDS
        while True:
            if time.monotonic() >= deadline:
                raise RuntimeError(f"Timed out waiting for Claude config lock: {self.path}")
            try:
                self.path.mkdir()
            except FileExistsError:
                try:
                    observed = self._current_stamp()
                    if time.time_ns() - observed[2] > _LOCK_STALE_SECONDS * 1_000_000_000:
                        # Recheck immediately before stale reclamation so a
                        # heartbeat observed in between cancels the removal.
                        if self._current_stamp() == observed:
                            self.path.rmdir()
                        continue
                except FileNotFoundError:
                    continue
                time.sleep(_LOCK_POLL_SECONDS)
            else:
                self._stamp = self._current_stamp()
                self._thread = threading.Thread(target=self._heartbeat, daemon=True)
                self._thread.start()
                return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join()
        try:
            self.assert_owned()
        except RuntimeError:
            if exc_type is None:
                raise
        else:
            self.path.rmdir()


def _apply_config(plan: _ConfigPlan, lock: _ConfigLock, servers: Mapping[str, object]) -> None:
    lock.assert_owned()
    if plan.text is None:
        return
    plan.target.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix=f".{plan.target.name}.tmp-", dir=plan.target.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            os.fchmod(output.fileno(), plan.mode)
            output.write(plan.text)
        lock.assert_owned()
        os.replace(temporary, plan.target)
    finally:
        temporary.unlink(missing_ok=True)
    lock.assert_owned()
    # Compare only fields owned by this installer. A Desktop writer may add
    # unrelated state without participating in this cooperative lock protocol.
    verified = _plan_config(plan.path, create=True, servers=servers, reason="")
    if verified.text is not None:
        raise RuntimeError(f"Claude managed MCP registration readback differs: {plan.path}")


def install_claude_config(
    install_root: Path, *, home: Path, env: Mapping[str, str]
) -> dict[str, ClaudeConfigReport]:
    """Register both MCP launchers for installed Claude Code and Desktop hosts.

    Code uses ``~/.claude.json``; an existing ``~/.claude`` directory also
    authorizes creating that file. Desktop is changed only when its Linux
    ``Claude/claude_desktop_config.json`` already exists. Validate both files
    before writing either one, preserving existing modes and symlink targets.
    """
    override_env = {key: env[key].strip() for key in _OVERRIDE_ENV_KEYS if env.get(key, "").strip()}
    servers: dict[str, object] = {
        "sky-cua": {
            "type": "stdio",
            "command": str(install_root / "bin/sky-cua-client"),
            "args": ["mcp"],
            "env": override_env,
        },
        "node_repl": {
            "type": "stdio",
            "command": str(install_root / "bin/node_repl"),
            "args": [],
            "env": override_env,
        },
    }
    config_base = Path(env.get("XDG_CONFIG_HOME") or home / ".config").expanduser()
    code = _plan_config(
        home / ".claude.json",
        create=(home / ".claude").is_dir(),
        servers=servers,
        reason="Claude Code user config and directory are absent",
    )
    desktop = _plan_config(
        config_base / "Claude/claude_desktop_config.json",
        create=False,
        servers=servers,
        reason="Claude Desktop config is absent",
    )
    # Preflight above catches malformed host files before creating any locks.
    # Then re-read *both* under their locks before writing either: even a plan
    # that was originally unchanged can need repair after waiting for Claude.
    plans = [code, desktop]
    with ExitStack() as stack:
        locks = {
            plan.path: stack.enter_context(_ConfigLock(plan.path))
            for plan in sorted(plans, key=lambda item: str(item.path))
            if plan.status != "skipped"
        }
        plans = [
            _plan_config(
                plan.path,
                create=plan.status != "skipped",
                servers=servers,
                reason=plan.reason or "",
            )
            if plan.path in locks
            else plan
            for plan in plans
        ]
        for plan in plans:
            if plan.path in locks:
                _apply_config(plan, locks[plan.path], servers)
    return {"code": plans[0].report(), "desktop": plans[1].report()}
