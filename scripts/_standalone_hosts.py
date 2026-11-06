"""Standalone host configuration and shared projection/configuration helpers."""

from __future__ import annotations

import json
import os
import shlex
import shutil
import subprocess
import tomllib
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import Any

Runner = Callable[..., subprocess.CompletedProcess[str]]

PI_MCP_RUNTIME_ENV = (
    "PATH",
    "SKY_CUA_BROWSER_EVAL",
    "SKY_CUA_MODEL_SUPPORTS_IMAGES",
    "SKY_CUA_PRESENCE_ENABLED",
    "SKY_CUA_BROWSER_CONTROL_MODE",
    "SKY_CUA_CODEX_BROWSER_SOCKET_PATH",
    "SKY_CUA_PHONE_DIRECT",
    "SKY_CUA_PHONE_DIRECT_ADVERTISED_ENDPOINT",
    "SKY_CUA_PHONE_DIRECT_ENROLLMENT_TTL_MS",
    "SKY_CUA_PHONE_DIRECT_LISTEN_ADDR",
    "SKY_CUA_PHONE_DIRECT_STATE_PATH",
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XAUTHORITY",
)


def _remove_path(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink(missing_ok=True)
    elif path.is_dir():
        shutil.rmtree(path)


def _copy_tree(source: Path, destination: Path) -> None:
    if source.is_symlink() or not source.is_dir():
        raise FileNotFoundError(f"required directory is missing: {source}")
    shutil.copytree(source, destination, dirs_exist_ok=True, symlinks=False)


def _link_or_copy_directory(target: Path, link_path: Path) -> None:
    """Symlink ``link_path`` -> ``target``, falling back to a directory copy."""
    if link_path.is_symlink():
        try:
            if link_path.readlink() == target:
                return
        except OSError:
            pass
        link_path.unlink()
    elif link_path.exists():
        _remove_path(link_path)
    link_path.parent.mkdir(parents=True, exist_ok=True)
    try:
        link_path.symlink_to(target, target_is_directory=target.is_dir())
    except (OSError, NotImplementedError, FileExistsError):
        _remove_path(link_path)
        if not target.is_absolute():
            target = (link_path.parent / target).resolve()
        shutil.copytree(target, link_path, symlinks=False)


def _toml_string(value: Path | str) -> str:
    return json.dumps(str(value))


def _upsert_toml_key(config_text: str, header: str, key: str, rendered_value: str) -> str:
    import re

    section_re = re.compile(
        rf"(^\[{re.escape(header)}\][ \t]*(?:#[^\r\n]*)?\r?\n)(.*?)(?=^\[|\Z)",
        re.MULTILINE | re.DOTALL,
    )
    match = section_re.search(config_text)
    line = f"{key} = {rendered_value}"
    if match is None:
        separator = "" if not config_text or config_text.endswith("\n") else "\n"
        return f"{config_text}{separator}\n[{header}]\n{line}\n"
    body = match.group(2)
    key_re = re.compile(rf"^{re.escape(key)}\s*=.*$", re.MULTILINE)
    if key_re.search(body):
        new_body = key_re.sub(lambda _match: line, body, count=1)
    else:
        suffix = "" if body.endswith(("\n", "\r\n")) or body == "" else "\n"
        new_body = f"{body}{suffix}{line}\n"
    return config_text[: match.start(2)] + new_body + config_text[match.end(2) :]


def _install_openclaw_node_repl(
    install_root: Path,
    *,
    home: Path,
    env: Mapping[str, str],
    which: Callable[[str], str | None],
    runner: Runner,
) -> bool:
    openclaw = which("openclaw")
    if openclaw is None:
        return False
    definition: dict[str, Any] = {
        "enabled": True,
        "command": str(install_root / "bin/node_repl"),
        "args": [],
        "cwd": str(install_root),
        # node_repl derives its runtime paths (node binary, module root,
        # Playwright browsers, launcher path) from its own binary location at
        # startup, so the host config must not pin them. Keep an empty env so
        # the process inherits the parent environment and auto-detects.
        "env": {},
        "connectionTimeoutMs": 120_000,
        "requestTimeoutMs": 3_600_000,
        "supportsParallelToolCalls": False,
    }
    command_env = os.environ.copy()
    command_env.update(env)
    command_env["HOME"] = str(home)
    projected_bin = (home / ".local/bin").resolve()
    command_env["PATH"] = os.pathsep.join(
        entry
        for entry in command_env.get("PATH", "").split(os.pathsep)
        if entry and Path(entry).expanduser().resolve() != projected_bin
    )
    result = runner(
        [
            openclaw,
            "mcp",
            "set",
            "node_repl",
            json.dumps(definition, sort_keys=True, separators=(",", ":")),
        ],
        check=False,
        capture_output=True,
        text=True,
        env=command_env,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise RuntimeError(f"OpenClaw node_repl registration failed: {detail}")
    return True


def _set_top_level_toml_string(text: str, key: str, value: str) -> str:
    """Set one top-level TOML string while preserving unrelated formatting."""
    lines = text.splitlines(keepends=True)
    first_table = next(
        (index for index, line in enumerate(lines) if line.lstrip().startswith("[")),
        len(lines),
    )
    replacement = f'{key} = "{value}"\n'
    matches = [
        index
        for index, line in enumerate(lines[:first_table])
        if line.split("=", 1)[0].strip() == key and "=" in line
    ]
    if len(matches) > 1:
        raise ValueError(f"OpenClaw Codex config has duplicate top-level {key!r} entries")
    if matches:
        lines[matches[0]] = replacement
    else:
        lines.insert(first_table, replacement)
    return "".join(lines)


def _write_text_atomically(path: Path, text: str) -> None:
    write_path = path.resolve(strict=False) if path.is_symlink() else path
    write_path.parent.mkdir(parents=True, exist_ok=True)
    mode = write_path.stat().st_mode if write_path.exists() else 0o600
    temporary = write_path.with_name(f".{write_path.name}.tmp-{os.getpid()}")
    _remove_path(temporary)
    try:
        temporary.write_text(text, encoding="utf-8")
        temporary.chmod(mode)
        os.replace(temporary, write_path)
    finally:
        _remove_path(temporary)


def _install_pi_mcp(
    install_root: Path,
    *,
    home: Path,
    env: Mapping[str, str],
) -> Path | None:
    """Project the fixed-root MCP launcher into an existing Pi installation."""
    wrapper = install_root / "pi_mcp_wrapper.sh"
    forwarded = {
        name: value.strip() for name in PI_MCP_RUNTIME_ENV if (value := env.get(name, "")).strip()
    }
    forwarded.setdefault("SKY_CUA_PRESENCE_ENABLED", "1")
    exports = "".join(f"export {name}={shlex.quote(value)}\n" for name, value in forwarded.items())
    wrapper.write_text(
        "".join(
            (
                "#!/bin/bash\n",
                f"export SKY_CUA_REPO_ROOT={shlex.quote(str(install_root))}\n",
                exports,
                "export SKY_CUA_MCP_CALLER_PROVENANCE=pi\n",
                f'exec {shlex.quote(str(install_root / "bin/sky-cua-client"))} mcp "$@"\n',
            )
        ),
        encoding="utf-8",
    )
    wrapper.chmod(0o755)

    agent_dir = home / ".pi/agent"
    if not agent_dir.is_dir():
        return None
    extension_source = install_root / "resources/pi/sky-cua-image-capability.ts"
    extension_dir = agent_dir / "extensions"
    extension_dir.mkdir(parents=True, exist_ok=True)
    extension_path = extension_dir / "sky-cua-image-capability.ts"
    if extension_path.exists() or extension_path.is_symlink():
        if extension_path.is_symlink():
            expected = extension_source.resolve(strict=False)
            actual = extension_path.resolve(strict=False)
            if actual != expected:
                # Legacy installs under /home/bex and other prior fixed roots left
                # dangling or absolute symlinks that still point at a sky-cua
                # managed copy of this exact extension. Treat those as managed
                # so host migrations (bex -> ubuntu) and reinstalls converge
                # instead of failing the entire deploy.
                try:
                    raw_target = str(extension_path.readlink())
                except OSError:
                    raw_target = ""
                is_legacy_managed = (
                    raw_target.endswith("sky-cua-image-capability.ts") and "sky-cua" in raw_target
                )
                if not is_legacy_managed:
                    raise ValueError(
                        f"refusing to replace an unmanaged Pi extension: {extension_path}"
                    )
        else:
            # Regular file at the extension path: only replace if it is
            # byte-identical to the shipped source (e.g. a prior copy
            # instead of a symlink). Otherwise preserve the user's file.
            try:
                if extension_path.read_bytes() != extension_source.read_bytes():
                    raise ValueError(
                        f"refusing to replace an unmanaged Pi extension: {extension_path}"
                    )
            except OSError as exc:
                raise ValueError(
                    f"refusing to replace an unmanaged Pi extension: {extension_path}"
                ) from exc
        extension_path.unlink()
    extension_path.symlink_to(extension_source)
    config_path = agent_dir / "mcp.json"
    if config_path.exists():
        config = json.loads(config_path.read_text(encoding="utf-8"))
        if not isinstance(config, dict):
            raise ValueError(f"Pi MCP config must be a JSON object: {config_path}")
    else:
        config = {}
    servers = config.setdefault("mcpServers", {})
    if not isinstance(servers, dict):
        raise ValueError(f"Pi MCP config mcpServers must be a JSON object: {config_path}")
    servers["sky_cua"] = {
        "command": str(wrapper),
        "args": [],
        "lifecycle": "lazy",
        "directTools": True,
    }
    _write_text_atomically(config_path, json.dumps(config, indent=2) + "\n")
    return config_path


def _configure_openclaw_no_prompt_permissions(
    *,
    home: Path,
    env: Mapping[str, str],
    openclaw: str,
    runner: Runner,
) -> tuple[Path, ...]:
    """Converge OpenClaw and every existing Codex home to full-auto operation."""
    config_paths = sorted((home / ".openclaw/agents").glob("*/agent/codex-home/config.toml"))
    planned: list[tuple[Path, str]] = []
    for config_path in config_paths:
        original = config_path.read_text(encoding="utf-8")
        updated = _set_top_level_toml_string(original, "approval_policy", "never")
        updated = _set_top_level_toml_string(updated, "sandbox_mode", "danger-full-access")
        tomllib.loads(updated)
        if updated != original:
            planned.append((config_path, updated))

    batch = [
        {
            "path": "plugins.entries.codex.config.appServer.mode",
            "value": "yolo",
        },
        {
            "path": "plugins.entries.codex.config.appServer.approvalPolicy",
            "value": "never",
        },
        {
            "path": "plugins.entries.codex.config.appServer.sandbox",
            "value": "danger-full-access",
        },
        {
            "path": "plugins.entries.codex.config.codexPlugins.enabled",
            "value": True,
        },
        {
            "path": "plugins.entries.codex.config.codexPlugins.allow_all_plugins",
            "value": True,
        },
        {
            "path": "plugins.entries.codex.config.codexPlugins.allow_destructive_actions",
            "value": "approve",
        },
    ]
    command_env = os.environ.copy()
    command_env.update(env)
    command_env["HOME"] = str(home)
    result = runner(
        [openclaw, "config", "set", "--batch-json", json.dumps(batch, separators=(",", ":"))],
        check=False,
        capture_output=True,
        text=True,
        env=command_env,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise RuntimeError(f"OpenClaw no-prompt permission configuration failed: {detail}")
    for config_path, updated in planned:
        _write_text_atomically(config_path, updated)
    return tuple(config_paths)
