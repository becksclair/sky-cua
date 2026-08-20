"""OpenCode global config file merging helpers.

These helpers discover, read, merge, back up, and atomically update the
global OpenCode config (``opencode.json``/``opencode.jsonc``) so the
sky-cua MCP servers are registered.

Exported callers: ``standalone_release.install_payload``.
"""

from __future__ import annotations

import hashlib
import json
import os
import stat
import tempfile
from collections.abc import Mapping
from pathlib import Path

OPENCODE_GLOBAL_CONFIG_NAMES = ("opencode.json", "opencode.jsonc")
OPENCODE_MANAGED_SERVERS = ("sky_cua", "node_repl")
OPENCODE_MCP_TIMEOUT_MS = 30_000
OPENCODE_BACKUP_DIR_NAME = ".sky-cua-backups"

# Genuine operator runtime-config overrides that may be forwarded to the MCP
# server. Session variables (XDG_RUNTIME_DIR, Wayland/X11 display, dbus,
# compositor) and runtime roots (repo/release/docs) are intentionally excluded:
# sky-cua auto-detects those at startup, and because MCP `env` merges with the
# parent process, an empty block still inherits any operator-set values the
# parent already has. The default install forwards nothing.
OVERRIDE_ENV_KEYS = (
    "SKY_CUA_BROWSER_CONTROL_MODE",
    "SKY_CUA_CODEX_BROWSER_SOCKET_PATH",
    "SKY_CUA_PHONE_DIRECT",
    "SKY_CUA_PHONE_DIRECT_ADVERTISED_ENDPOINT",
    "SKY_CUA_PHONE_DIRECT_ENROLLMENT_TTL_MS",
    "SKY_CUA_PHONE_DIRECT_LISTEN_ADDR",
    "SKY_CUA_PHONE_DIRECT_STATE_PATH",
)


def _explicit_override_env(active_env: Mapping[str, str]) -> dict[str, str]:
    return {
        key: active_env[key].strip()
        for key in OVERRIDE_ENV_KEYS
        if key in active_env and active_env[key].strip()
    }


def _strip_jsonc_comments(text: str) -> str:
    """Strip ``//`` and ``/* */`` comments while preserving string literals.

    Sufficient for OpenCode config files; does not handle JSON5 trailing commas.
    """
    out: list[str] = []
    i = 0
    n = len(text)
    in_string = False
    string_quote = ""
    while i < n:
        ch = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if in_string:
            out.append(ch)
            if ch == "\\" and nxt:
                out.append(nxt)
                i += 2
                continue
            if ch == string_quote:
                in_string = False
            i += 1
            continue
        if ch in ('"', "'"):
            in_string = True
            string_quote = ch
            out.append(ch)
            i += 1
            continue
        if ch == "/" and nxt == "/":
            i += 2
            while i < n and text[i] != "\n":
                i += 1
            continue
        if ch == "/" and nxt == "*":
            i += 2
            while i + 1 < n and not (text[i] == "*" and text[i + 1] == "/"):
                i += 1
            i += 2
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def _build_opencode_servers(
    install_root: Path,
    *,
    timeout_ms: int = OPENCODE_MCP_TIMEOUT_MS,
    env: Mapping[str, str] | None = None,
) -> dict[str, object]:
    client = str(install_root / "bin/sky-cua-client")
    node_repl = str(install_root / "bin/node_repl")
    active_env = os.environ if env is None else env
    # sky-cua detects its desktop session (XDG_RUNTIME_DIR, Wayland/X11 display,
    # dbus, compositor) and runtime roots (repo/release/docs) on its own at
    # startup, so the MCP host config must not pin them. Only honour explicit
    # operator overrides here; the default install emits an empty environment.
    shared_env: dict[str, str] = _explicit_override_env(active_env)
    return {
        "sky_cua": {
            "type": "local",
            "command": [client, "mcp"],
            "cwd": str(install_root),
            "environment": shared_env,
            "enabled": True,
            "timeout": timeout_ms,
        },
        "node_repl": {
            "type": "local",
            "command": [node_repl],
            "cwd": str(install_root),
            "environment": {
                **shared_env,
                "CODEX_NODE_REPL_PATH": node_repl,
                "NODE_REPL_NODE_PATH": str(install_root / "bin/node"),
                "NODE_REPL_NODE_MODULE_DIRS": str(install_root / "lib/node_modules"),
                "PLAYWRIGHT_BROWSERS_PATH": str(install_root / "share/playwright"),
            },
            "enabled": True,
            "timeout": timeout_ms,
        },
    }


def _opencode_global_config_dir(env: Mapping[str, str], home: Path) -> Path:
    configured = env.get("XDG_CONFIG_HOME", "").strip()
    base = Path(configured).expanduser() if configured else home / ".config"
    return base / "opencode"


def _opencode_existing_config_path(config_dir: Path) -> Path | None:
    if not config_dir.is_absolute():
        raise ValueError(f"OpenCode config directory must be absolute: {config_dir}")
    resolved = config_dir.resolve()
    for name in OPENCODE_GLOBAL_CONFIG_NAMES:
        candidate = resolved / name
        if candidate.is_file() and not candidate.is_symlink():
            return candidate
    return None


def _opencode_env_hazards(env: Mapping[str, str]) -> tuple[str, ...]:
    messages = {
        "OPENCODE_CONFIG": "selects a higher-precedence custom config",
        "OPENCODE_CONFIG_CONTENT": "supplies a higher-precedence inline config",
        "OPENCODE_CONFIG_DIR": "selects a higher-precedence custom config directory",
    }
    return tuple(
        f"{name} {description}"
        for name, description in messages.items()
        if env.get(name, "").strip()
    )


def _merge_opencode_servers(
    existing: dict[str, object], servers: Mapping[str, object]
) -> dict[str, object]:
    merged = dict(existing)
    mcp = merged.get("mcp")
    if not isinstance(mcp, dict):
        mcp = {}
    mcp = dict(mcp)
    for name, definition in servers.items():
        mcp[name] = definition
    merged["mcp"] = mcp
    return merged


def _write_opencode_backup(config_path: Path, content: bytes, mode: int) -> Path:
    backup_dir = config_path.parent / OPENCODE_BACKUP_DIR_NAME
    backup_dir.mkdir(parents=True, exist_ok=True)
    backup_dir.chmod(0o700)
    digest = hashlib.sha256(content).hexdigest()
    backup_path = backup_dir / f"{config_path.name}.{digest}.{mode:04o}.json"
    if backup_path.exists() and backup_path.read_bytes() == content:
        backup_path.chmod(0o600)
        return backup_path
    if backup_path.exists():
        raise RuntimeError(f"OpenCode rollback snapshot collision: {backup_path}")
    backup_path.write_bytes(content)
    backup_path.chmod(0o600)
    return backup_path


def install_opencode_config(
    install_root: Path,
    *,
    home: Path,
    env: Mapping[str, str],
    timeout_ms: int = OPENCODE_MCP_TIMEOUT_MS,
) -> dict[str, object]:
    """Update the global OpenCode config with the two managed MCP servers.

    No-ops when no global ``opencode.json``/``opencode.jsonc`` exists yet, so
    a fresh install never writes outside its scope. Refuses to write when
    ``OPENCODE_CONFIG``/``OPENCODE_CONFIG_CONTENT``/``OPENCODE_CONFIG_DIR``
    selects a higher-precedence source. Backs the previous file up under
    ``~/.config/opencode/.sky-cua-backups/``.
    """
    config_dir = _opencode_global_config_dir(env, home)
    config_path = _opencode_existing_config_path(config_dir)
    if config_path is None:
        return {
            "status": "no_global_config",
            "config_path": None,
            "servers": list(OPENCODE_MANAGED_SERVERS),
        }
    hazards = _opencode_env_hazards(env)
    if hazards:
        raise RuntimeError("OpenCode config precedence hazard(s): " + "; ".join(hazards))

    original_bytes = config_path.read_bytes()
    original_mode = stat.S_IMODE(config_path.stat().st_mode)
    try:
        original_text = original_bytes.decode("utf-8")
    except UnicodeDecodeError as error:
        raise RuntimeError(f"OpenCode config is not UTF-8: {config_path}") from error

    try:
        existing = json.loads(_strip_jsonc_comments(original_text))
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"OpenCode config is not valid JSONC: {config_path}: {error.msg}"
        ) from error
    if not isinstance(existing, dict):
        raise RuntimeError(f"OpenCode config root must be an object: {config_path}")

    servers = _build_opencode_servers(
        install_root,
        timeout_ms=timeout_ms,
        env=env,
    )
    merged = _merge_opencode_servers(existing, servers)
    rendered = json.dumps(merged, indent=2, sort_keys=False) + "\n"
    rendered_bytes = rendered.encode("utf-8")

    if original_bytes == rendered_bytes:
        return {
            "status": "unchanged",
            "config_path": str(config_path),
            "servers": list(OPENCODE_MANAGED_SERVERS),
        }

    backup_path = _write_opencode_backup(config_path, original_bytes, original_mode)
    try:
        fd, tmp_path_str = tempfile.mkstemp(
            dir=str(config_path.parent), prefix=f".{config_path.name}.tmp."
        )
        try:
            os.write(fd, rendered_bytes)
            os.fsync(fd)
            os.chmod(tmp_path_str, original_mode)
        finally:
            os.close(fd)
        os.replace(tmp_path_str, str(config_path))
        parsed_after = json.loads(config_path.read_text(encoding="utf-8"))
        if not isinstance(parsed_after, dict) or parsed_after != merged:
            raise RuntimeError("OpenCode config readback differs after atomic write")
    except BaseException:
        try:
            config_path.write_bytes(original_bytes)
            config_path.chmod(original_mode)
        except BaseException as restore_error:
            raise RuntimeError(
                "Failed to restore OpenCode config after write failure"
            ) from restore_error
        raise

    return {
        "status": "updated",
        "config_path": str(config_path),
        "backup_path": str(backup_path),
        "servers": list(OPENCODE_MANAGED_SERVERS),
    }
