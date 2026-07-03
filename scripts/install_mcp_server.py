#!/usr/bin/env python3
"""Install sky-cua as a generic MCP server for any MCP-compatible host.

This script copies the built runtime binaries to a stable installation directory
and emits a host-ready MCP server configuration with absolute paths.

Supported host targets:
  - opencode        OpenCode via opencode.json
  - claude-code     Claude Code via `claude mcp add-json` and ~/.claude/skills
  - claude-desktop  Claude Desktop via claude_desktop_config.json
  - pi              Pi via mcp.json
  - generic         Raw .mcp.json for manual wiring (default)

Usage:
  python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host opencode
"""

from __future__ import annotations

import argparse
import contextlib
import grp
import json
import os
import shlex
import shutil
import stat
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

import _install_shared
from _install_shared import (
    BROWSER_SELECTION_ENV,
    DEFAULT_LOCAL_INSTALL_DIR,
    MCP_HOST_CHOICES,
    atomic_sibling_path,
    ensure_parent,
    install_sky_cua_skills,
    subprocess_error_detail,
    write_text_atomically,
)
from _kwin_effect import (
    deploy_kwin_effect,
    kwin_effect_deploy_failed,
    print_kwin_effect_deploy_outcome,
)
from _openclaw_install import DEFAULT_OPENCLAW_DIR, install_openclaw
from _plugin_bundle import (
    LINUX_ARM64,
    LINUX_X64,
    WINDOWS_X64,
    current_runtime_platform,
    platform_runtime_binary_base_names,
    remove_path,
    runtime_binary_path,
    runtime_binary_source_name,
    stop_unix_runtime_processes,
    stop_windows_cache_processes,
)
from deploy_freshness import STAMP_SUFFIX

CLAUDE_MCP_ADD_TIMEOUT_SECONDS = 30

# Claude Code permission rules applied at install time: deny the built-in
# computer-use MCP so desktop control routes through sky-cua, and pre-approve
# sky-cua's own tools so they never prompt. Both the server-scope and wildcard
# forms are written so the rules bite regardless of Claude Code's rule syntax.
CLAUDE_CODE_DENY_RULES = ("mcp__computer-use", "mcp__computer-use__*")
CLAUDE_CODE_ALLOW_RULES = ("mcp__sky-cua", "mcp__sky-cua__*")
AT_SPI_RESTART_TIMEOUT_SECONDS = 5
MCP_BROWSER_EVAL_ENV = "SKY_CUA_BROWSER_EVAL"
MCP_MODEL_SUPPORTS_IMAGES_ENV = "SKY_CUA_MODEL_SUPPORTS_IMAGES"
MCP_PRESENCE_ENABLED_ENV = "SKY_CUA_PRESENCE_ENABLED"
MCP_LAUNCH_POLICY_STATE = "mcp-launch-policy.json"
RECOGNIZED_MCP_LAUNCH_ENV = (
    MCP_BROWSER_EVAL_ENV,
    MCP_MODEL_SUPPORTS_IMAGES_ENV,
    MCP_PRESENCE_ENABLED_ENV,
)


@dataclass(frozen=True)
class McpLaunchPolicy:
    browser_eval: str | None = None
    model_supports_images: str | None = None
    presence_enabled: str | None = None

    def env(self) -> dict[str, str]:
        env: dict[str, str] = {}
        if self.browser_eval is not None:
            env[MCP_BROWSER_EVAL_ENV] = self.browser_eval
        if self.model_supports_images is not None:
            env[MCP_MODEL_SUPPORTS_IMAGES_ENV] = self.model_supports_images
        # Session presence (auto-unlock + lock/suspend inhibitors) is ON by
        # default for every deployed host; unset or "on" both emit "1". Disable
        # per-host with `--presence off` or SKY_CUA_PRESENCE_ENABLED=off.
        env[MCP_PRESENCE_ENABLED_ENV] = "0" if self.presence_enabled == "off" else "1"
        return env


def current_platform() -> str:
    return current_runtime_platform()


def normalize_on_off(value: str, *, source: str) -> str:
    normalized = value.strip().lower()
    if normalized in {"1", "true", "yes", "on", "enabled"}:
        return "on"
    if normalized in {"0", "false", "no", "off", "disabled"}:
        return "off"
    raise ValueError(f"{source} must be an on/off boolean, got {value!r}")


def normalize_true_false(value: str, *, source: str) -> str:
    normalized = value.strip().lower()
    if normalized in {"1", "true", "yes", "on", "supported", "enabled"}:
        return "true"
    if normalized in {"0", "false", "no", "off", "unsupported", "disabled"}:
        return "false"
    raise ValueError(f"{source} must be a true/false boolean, got {value!r}")


def load_persisted_mcp_launch_policy(target_dir: Path) -> McpLaunchPolicy | None:
    path = target_dir / MCP_LAUNCH_POLICY_STATE
    if not path.exists():
        return None
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid persisted MCP launch policy {path}: {error}") from error
    if not isinstance(raw, dict):
        raise ValueError(f"persisted MCP launch policy must be a JSON object: {path}")
    return McpLaunchPolicy(
        browser_eval=(
            normalize_on_off(str(raw["browser_eval"]), source=f"{path}:browser_eval")
            if raw.get("browser_eval") is not None
            else None
        ),
        model_supports_images=(
            normalize_true_false(
                str(raw["model_supports_images"]), source=f"{path}:model_supports_images"
            )
            if raw.get("model_supports_images") is not None
            else None
        ),
        presence_enabled=(
            normalize_on_off(str(raw["presence_enabled"]), source=f"{path}:presence_enabled")
            if raw.get("presence_enabled") is not None
            else None
        ),
    )


def resolve_mcp_launch_policy(
    target_dir: Path,
    *,
    browser_eval: str | None = None,
    model_supports_images: str | None = None,
    presence_enabled: str | None = None,
    environ: dict[str, str] | None = None,
) -> McpLaunchPolicy:
    """Resolve launch policy per field: CLI, persisted state, env, defaults."""
    env = os.environ if environ is None else environ
    persisted = load_persisted_mcp_launch_policy(target_dir)

    env_browser_eval = (
        normalize_on_off(env[MCP_BROWSER_EVAL_ENV], source=MCP_BROWSER_EVAL_ENV)
        if MCP_BROWSER_EVAL_ENV in env
        else None
    )
    env_model_supports_images = (
        normalize_true_false(
            env[MCP_MODEL_SUPPORTS_IMAGES_ENV], source=MCP_MODEL_SUPPORTS_IMAGES_ENV
        )
        if MCP_MODEL_SUPPORTS_IMAGES_ENV in env
        else None
    )
    env_presence_enabled = (
        normalize_on_off(env[MCP_PRESENCE_ENABLED_ENV], source=MCP_PRESENCE_ENABLED_ENV)
        if MCP_PRESENCE_ENABLED_ENV in env
        else None
    )

    return McpLaunchPolicy(
        browser_eval=(
            normalize_on_off(browser_eval, source="--browser-eval")
            if browser_eval is not None
            else (
                persisted.browser_eval
                if persisted is not None and persisted.browser_eval is not None
                else env_browser_eval
            )
        ),
        model_supports_images=(
            normalize_true_false(model_supports_images, source="--model-supports-images")
            if model_supports_images is not None
            else (
                persisted.model_supports_images
                if persisted is not None and persisted.model_supports_images is not None
                else env_model_supports_images
            )
        ),
        presence_enabled=(
            normalize_on_off(presence_enabled, source="--presence")
            if presence_enabled is not None
            else (
                persisted.presence_enabled
                if persisted is not None and persisted.presence_enabled is not None
                else env_presence_enabled
            )
        ),
    )


def write_mcp_launch_policy_state(target_dir: Path, policy: McpLaunchPolicy) -> Path:
    path = target_dir / MCP_LAUNCH_POLICY_STATE
    payload = {
        "browser_eval": policy.browser_eval,
        "model_supports_images": policy.model_supports_images,
        "presence_enabled": policy.presence_enabled,
    }
    write_text_atomically(path, json.dumps(payload, indent=2) + "\n")
    return path


def entrypoint_path(platform_id: str, name: str) -> Path:
    if platform_id == WINDOWS_X64:
        return Path("bin") / f"{name}.exe"
    return Path("bin") / name


def source_binary_path(name: str) -> Path:
    return _install_shared.REPO_ROOT / "target" / "release" / name


def copy_executable(src: Path, dst: Path) -> None:
    ensure_parent(dst)
    temp_path = atomic_sibling_path(dst, "tmp")
    remove_path(temp_path)
    try:
        shutil.copy2(src, temp_path)
        if sys.platform != "win32" and not dst.name.endswith(".exe"):
            mode = temp_path.stat().st_mode
            temp_path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        os.replace(temp_path, dst)
    finally:
        remove_path(temp_path)
    copy_build_stamp_sidecar(src, dst)


def copy_build_stamp_sidecar(src: Path, dst: Path) -> None:
    src_stamp = src.with_name(src.name + STAMP_SUFFIX)
    dst_stamp = dst.with_name(dst.name + STAMP_SUFFIX)
    if not src_stamp.exists():
        remove_path(dst_stamp)
        return

    ensure_parent(dst_stamp)
    temp_path = atomic_sibling_path(dst_stamp, "tmp")
    remove_path(temp_path)
    try:
        shutil.copy2(src_stamp, temp_path)
        os.replace(temp_path, dst_stamp)
    finally:
        remove_path(temp_path)


def find_built_binary(name: str) -> Path | None:
    candidate = source_binary_path(runtime_binary_source_name(current_platform(), name))
    if candidate.exists():
        return candidate
    return None


def install_binaries(target_dir: Path) -> Path:
    """Copy runtime binaries into target_dir and return the client entrypoint path."""
    platform_id = current_platform()
    installed_client: Path | None = None

    for name in platform_runtime_binary_base_names(platform_id):
        src = find_built_binary(name)
        if src is None:
            raise FileNotFoundError(
                f"binary not found: {name}. Build first with 'cargo build --release'."
            )
        dst = target_dir / entrypoint_path(platform_id, name)
        copy_executable(src, dst)
        if name == "sky-cua-client":
            installed_client = dst

    # Also copy cross-platform binaries if they exist in the repo (e.g. from a
    # prior build on another machine or from CI artifacts).
    for foreign_platform in (LINUX_X64, LINUX_ARM64, WINDOWS_X64):
        if foreign_platform == platform_id:
            continue
        for name in platform_runtime_binary_base_names(foreign_platform):
            foreign_src = (
                _install_shared.REPO_ROOT
                / "target"
                / "release"
                / runtime_binary_source_name(foreign_platform, name)
            )
            if not foreign_src.exists():
                continue
            foreign_dst = target_dir / runtime_binary_path(foreign_platform, name)
            copy_executable(foreign_src, foreign_dst)

    if installed_client is None:
        raise RuntimeError("sky-cua-client binary was not installed")

    return installed_client


def install_bundle_binaries(bundle_root: Path, target_dir: Path) -> Path:
    """Copy runtime binaries from a built plugin bundle into target_dir.

    Deploy scripts use this instead of install_binaries so every channel
    ships the exact bits staged in the bundle, not whatever happens to sit
    in target/release.
    """
    platform_id = current_platform()
    installed_client: Path | None = None

    for name in platform_runtime_binary_base_names(platform_id):
        src = bundle_root / runtime_binary_path(platform_id, name)
        if not src.exists():
            raise FileNotFoundError(
                f"bundle binary not found: {src}. Build first with scripts/build_plugin.py."
            )
        dst = target_dir / entrypoint_path(platform_id, name)
        copy_executable(src, dst)
        if name == "sky-cua-client":
            installed_client = dst

    for foreign_platform in (LINUX_X64, LINUX_ARM64, WINDOWS_X64):
        if foreign_platform == platform_id:
            continue
        for name in platform_runtime_binary_base_names(foreign_platform):
            foreign_src = bundle_root / runtime_binary_path(foreign_platform, name)
            if not foreign_src.exists():
                continue
            copy_executable(foreign_src, target_dir / runtime_binary_path(foreign_platform, name))

    if installed_client is None:
        raise RuntimeError("sky-cua-client binary was not installed")

    return installed_client


def generate_mcp_config(
    client_path: Path,
    target_dir: Path,
    resource_root: Path | None = None,
    launch_policy: McpLaunchPolicy | None = None,
) -> dict[str, object]:
    """Build an MCP server config dict with absolute paths."""
    root = runtime_resource_root(resource_root)
    policy = launch_policy or McpLaunchPolicy()
    return {
        "mcpServers": {
            "computer-use": {
                "command": str(client_path),
                "args": ["mcp"],
                "env": {
                    "SKY_CUA_REPO_ROOT": str(root),
                    **policy.env(),
                },
                "env_vars": [
                    "CODEX_COMPUTER_USE_COSMIC_HELPER",
                    "DBUS_SESSION_BUS_ADDRESS",
                    "DESKTOP_SESSION",
                    "DISPLAY",
                    "SKY_CUA_AGENT_CURSOR",
                    BROWSER_SELECTION_ENV,
                    MCP_BROWSER_EVAL_ENV,
                    "SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS",
                    "SKY_CUA_COSMIC_HELPER",
                    "SKY_CUA_INPUT_BACKEND",
                    "SKY_CUA_INPUT_HELPER_SOCKET",
                    MCP_MODEL_SUPPORTS_IMAGES_ENV,
                    "SKY_CUA_MODEL_SCREENSHOT_FORMAT",
                    "SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY",
                    "SKY_CUA_MODEL_SCREENSHOT_MAX_HEIGHT",
                    "SKY_CUA_MODEL_SCREENSHOT_MAX_WIDTH",
                    "SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY",
                    "SKY_CUA_OVERLAY_BACKEND",
                    "SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE",
                    "SKY_CUA_OVERLAY_HOST_PATH",
                    "SKY_CUA_OVERLAY_HOST_TCP_ADDR",
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
                    "SKY_CUA_XKB_LAYOUT",
                    "SKY_CUA_XKB_MODEL",
                    "SKY_CUA_XKB_OPTIONS",
                    "SKY_CUA_XKB_RULES",
                    "SKY_CUA_XKB_VARIANT",
                    "WAYLAND_DISPLAY",
                    "XDG_CURRENT_DESKTOP",
                    "XDG_RUNTIME_DIR",
                    "XDG_SESSION_TYPE",
                    "YDOTOOL_SOCKET",
                ],
                "cwd": str(target_dir),
            }
        }
    }


def write_mcp_json(target_dir: Path, config: dict[str, object]) -> Path:
    path = target_dir / ".mcp.json"
    write_text_atomically(path, json.dumps(config, indent=2) + "\n")
    return path


def runtime_resource_root(bundle_root: Path | None) -> Path:
    """Root containing runtime resources such as resources/app-instructions."""
    return (bundle_root or _install_shared.REPO_ROOT).resolve()


def install_opencode(
    target_dir: Path,
    client_path: Path,
    resource_root: Path | None = None,
    launch_policy: McpLaunchPolicy | None = None,
) -> Path:
    """Update or create opencode.json in the target directory."""
    root = runtime_resource_root(resource_root)
    policy = launch_policy or McpLaunchPolicy()
    opencode_config: dict[str, object] = {
        "$schema": "https://opencode.ai/config.json",
        "mcp": {
            "sky_cua": {
                "type": "local",
                "command": [str(client_path), "mcp"],
                "environment": {
                    "SKY_CUA_REPO_ROOT": str(root),
                    **policy.env(),
                },
                "enabled": True,
                "timeout": 30000,
            }
        },
    }

    path = target_dir / "opencode.json"
    write_text_atomically(path, json.dumps(opencode_config, indent=2) + "\n")
    return path


def install_claude_desktop(
    target_dir: Path,
    client_path: Path,
    resource_root: Path | None = None,
    launch_policy: McpLaunchPolicy | None = None,
) -> Path:
    """Emit a Claude Desktop config snippet and print instructions."""
    root = runtime_resource_root(resource_root)
    policy = launch_policy or McpLaunchPolicy()
    snippet: dict[str, object] = {
        "mcpServers": {
            "computer-use": {
                "command": str(client_path),
                "args": ["mcp"],
                "env": {
                    "SKY_CUA_REPO_ROOT": str(root),
                    **policy.env(),
                },
            }
        }
    }

    path = target_dir / "claude_desktop_config.json"
    write_text_atomically(path, json.dumps(snippet, indent=2) + "\n")
    return path


def install_claude_code(
    target_dir: Path,
    client_path: Path,
    claude_config_dir: Path | None = None,
    resource_root: Path | None = None,
    launch_policy: McpLaunchPolicy | None = None,
) -> Path:
    """Register sky-cua with Claude Code and copy skills into ~/.claude/skills.

    Claude Code stdio MCP servers inherit the parent process environment, so the
    config only pins SKY_CUA_REPO_ROOT plus any explicit browser selection.
    """
    root = runtime_resource_root(resource_root)
    policy = launch_policy or McpLaunchPolicy()
    server: dict[str, object] = {
        "type": "stdio",
        "command": str(client_path),
        "args": ["mcp"],
        "env": {
            "SKY_CUA_REPO_ROOT": str(root),
            **policy.env(),
        },
    }
    # Claude Code reserves the MCP server name "computer-use" for its native
    # integration, so the Claude-facing registration uses "sky-cua".
    snippet = {"mcpServers": {"sky-cua": server}}
    path = target_dir / "claude_code_mcp.json"
    write_text_atomically(path, json.dumps(snippet, indent=2) + "\n")

    claude_dir = (claude_config_dir or (Path.home() / ".claude")).expanduser()
    if claude_dir.exists():
        install_sky_cua_skills(claude_dir / "skills")
        print(f"Installed sky-cua skills into {claude_dir / 'skills'}")

    try:
        settings_path = configure_claude_code_permissions(claude_dir)
    except OSError as error:
        print(
            f"warning: could not configure Claude Code permissions in "
            f"{claude_dir / 'settings.json'} ({error}).",
            file=sys.stderr,
        )
    else:
        if settings_path is not None:
            print(
                f"Configured Claude Code permissions in {settings_path} "
                "(deny built-in computer-use, auto-approve sky-cua)."
            )

    claude_bin = shutil.which("claude")
    if claude_bin is None:
        print(
            "claude CLI not found on PATH; register manually with:\n"
            f"  claude mcp add-json --scope user sky-cua "
            f"{shlex.quote(json.dumps(server, separators=(',', ':')))}"
        )
        return path

    register_claude_code_server(claude_bin, server, path)
    return path


def register_claude_code_server(
    claude_bin: str,
    server: dict[str, object],
    snippet_path: Path,
) -> None:
    """Register (or re-register) the sky-cua server at user scope.

    `claude mcp add-json` refuses existing names, so updates remove the stale
    entry first; Claude Code respawns the stdio server lazily on next use.
    """
    add_command = [
        claude_bin,
        "mcp",
        "add-json",
        "--scope",
        "user",
        "sky-cua",
        json.dumps(server, separators=(",", ":")),
    ]
    for attempt in ("add", "replace"):
        try:
            subprocess.run(
                add_command,
                check=True,
                timeout=CLAUDE_MCP_ADD_TIMEOUT_SECONDS,
                capture_output=True,
            )
            verb = "Registered" if attempt == "add" else "Re-registered"
            print(f"{verb} MCP server sky-cua with Claude Code (user scope).")
            return
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            stderr = subprocess_error_detail(error).removeprefix(": ")
            if attempt == "add" and "already exists" in stderr:
                remove = subprocess.run(
                    [claude_bin, "mcp", "remove", "--scope", "user", "sky-cua"],
                    check=False,
                    timeout=CLAUDE_MCP_ADD_TIMEOUT_SECONDS,
                    capture_output=True,
                )
                if remove.returncode == 0:
                    continue
                stderr = remove.stderr.decode(errors="replace").strip()
            detail = f": {stderr}" if stderr else ""
            print(
                f"warning: claude mcp registration failed ({error}{detail}); "
                f"merge {snippet_path} into your Claude Code MCP config manually.",
                file=sys.stderr,
            )
            return


def configure_claude_code_permissions(claude_dir: Path) -> Path | None:
    """Deny the built-in computer-use MCP and auto-approve sky-cua tools.

    Merges the deny/allow rules into ``<claude_dir>/settings.json`` (user
    scope) without disturbing existing settings, creating the file when absent.
    Idempotent: re-running adds no duplicate rules and does not rewrite an
    already-correct file. Returns the settings path, or ``None`` when the
    existing file is unreadable, not valid UTF-8, or cannot be parsed as a JSON
    object with object-shaped ``permissions`` and array-shaped rule lists (left
    untouched with a warning) so a malformed file never aborts the install.
    """
    settings_path = claude_dir / "settings.json"
    try:
        text = settings_path.read_text(encoding="utf-8") if settings_path.exists() else ""
    except (OSError, UnicodeDecodeError) as error:
        print(
            f"warning: not updating {settings_path}: cannot read existing file "
            f"({error}); fix it by hand.",
            file=sys.stderr,
        )
        return None
    if text.strip():
        try:
            settings = json.loads(text)
        except json.JSONDecodeError as error:
            print(
                f"warning: not updating {settings_path}: existing file fails JSON "
                f"validation ({error}); fix it by hand.",
                file=sys.stderr,
            )
            return None
        if not isinstance(settings, dict):
            print(
                f"warning: not updating {settings_path}: top-level value is not a JSON object.",
                file=sys.stderr,
            )
            return None
    else:
        settings = {}

    permissions = settings.setdefault("permissions", {})
    if not isinstance(permissions, dict):
        print(
            f"warning: not updating {settings_path}: 'permissions' is not a JSON object.",
            file=sys.stderr,
        )
        return None

    changed = False
    for key, rules in (("deny", CLAUDE_CODE_DENY_RULES), ("allow", CLAUDE_CODE_ALLOW_RULES)):
        entries = permissions.setdefault(key, [])
        if not isinstance(entries, list):
            print(
                f"warning: not updating {settings_path}: permissions.{key} is not a JSON array.",
                file=sys.stderr,
            )
            return None
        for rule in rules:
            if rule not in entries:
                entries.append(rule)
                changed = True

    if not changed:
        # changed is False only when the rules were read from an existing
        # non-empty file, so there is nothing new to write.
        return settings_path
    write_text_atomically(settings_path, json.dumps(settings, indent=2) + "\n")
    return settings_path


def install_pi(
    target_dir: Path,
    client_path: Path,
    pi_agent_dir: Path | None = None,
    resource_root: Path | None = None,
    launch_policy: McpLaunchPolicy | None = None,
) -> Path:
    """Emit a Pi mcp.json config snippet for merging into ~/.pi/agent/mcp.json.

    Pi does not support the ``env`` field in MCP server configs, so we generate a
    small wrapper script that sets ``SKY_CUA_REPO_ROOT`` and then execs the real
    client binary.
    """
    wrapper_path = target_dir / "pi_mcp_wrapper.sh"
    root = runtime_resource_root(resource_root)
    policy = launch_policy or McpLaunchPolicy()
    policy_exports = [
        f"export {name}={shlex.quote(value)}\n" for name, value in policy.env().items()
    ]
    wrapper_content = "".join(
        [
            "#!/usr/bin/env bash\n",
            f"export SKY_CUA_REPO_ROOT={shlex.quote(str(root))}\n",
            *policy_exports,
            f'exec {shlex.quote(str(client_path))} mcp "$@"\n',
        ]
    )
    write_text_atomically(wrapper_path, wrapper_content, mode=0o755)

    snippet: dict[str, object] = {
        "mcpServers": {
            "sky_cua": {
                "command": str(wrapper_path),
                "lifecycle": "lazy",
                "directTools": True,
            }
        }
    }

    path = target_dir / "pi_mcp.json"
    write_text_atomically(path, json.dumps(snippet, indent=2) + "\n")

    agent_dir = (pi_agent_dir or (Path.home() / ".pi" / "agent")).expanduser()
    if agent_dir.exists():
        merge_pi_mcp_config(agent_dir / "mcp.json", snippet)
        install_pi_skills(agent_dir / "skills")
    return path


def merge_pi_mcp_config(config_path: Path, snippet: dict[str, object]) -> None:
    ensure_parent(config_path)
    if config_path.exists():
        config = json.loads(config_path.read_text(encoding="utf-8"))
        if not isinstance(config, dict):
            raise ValueError(f"Pi MCP config must be a JSON object: {config_path}")
    else:
        config = {}

    servers = config.setdefault("mcpServers", {})
    if not isinstance(servers, dict):
        raise ValueError(f"Pi MCP config mcpServers must be a JSON object: {config_path}")
    snippet_servers = snippet.get("mcpServers")
    if not isinstance(snippet_servers, dict):
        raise ValueError("generated Pi MCP snippet is missing mcpServers")
    servers["sky_cua"] = snippet_servers["sky_cua"]
    write_text_atomically(config_path, json.dumps(config, indent=2) + "\n")


def install_pi_skills(skills_dir: Path) -> None:
    install_sky_cua_skills(skills_dir)


def link_current_platform_binaries(target_dir: Path, bin_dir: Path) -> None:
    platform_id = current_platform()
    bin_dir.mkdir(parents=True, exist_ok=True)
    for name in platform_runtime_binary_base_names(platform_id):
        entrypoint = entrypoint_path(platform_id, name)
        src = target_dir / entrypoint
        dst = bin_dir / entrypoint.name
        if dst.exists() or dst.is_symlink():
            dst.unlink()
        try:
            dst.symlink_to(src)
            print(f"Symlinked {dst} -> {src}")
        except (OSError, NotImplementedError):
            copy_executable(src, dst)
            print(f"Copied {src} -> {dst}")


def restart_runtime_processes(target_dir: Path, *, refresh_accessibility: bool = True) -> None:
    """Stop installed sky-cua runtime processes so hosts respawn fresh binaries."""
    if refresh_accessibility:
        refresh_accessibility_bus()
    if sys.platform == "win32":
        stop_windows_cache_processes(target_dir)
    else:
        stop_unix_runtime_processes([target_dir])


def install_input_helper_service(
    target_dir: Path,
    *,
    socket_group: str | None = None,
) -> None:
    if not sys.platform.startswith("linux"):
        print("warning: --input-helper is only supported on Linux; skipping", file=sys.stderr)
        return
    helper_path = target_dir / entrypoint_path(current_platform(), "sky-cua-input-helper")
    if not helper_path.exists():
        raise FileNotFoundError(
            f"sky-cua-input-helper binary not found at {helper_path}; build/install binaries first"
        )
    group = socket_group or default_input_helper_group()
    env_path = target_dir / "input-helper.env"
    service_path = target_dir / "sky-cua-input-helper.service"
    env_path.write_text(
        "\n".join(
            [
                "SKY_CUA_INPUT_HELPER_SOCKET=/run/sky-cua/input-helper.sock",
                "SKY_CUA_INPUT_HELPER_SOCKET_MODE=0660",
                f"SKY_CUA_INPUT_HELPER_SOCKET_GROUP={group}",
                "",
            ]
        ),
        encoding="utf-8",
    )
    service_path.write_text(
        "\n".join(
            [
                "[Unit]",
                "Description=sky-cua privileged uinput helper",
                "After=systemd-udevd.service",
                "",
                "[Service]",
                "Type=simple",
                "EnvironmentFile=/etc/sky-cua/input-helper.env",
                f"ExecStart={helper_path} serve",
                "Restart=on-failure",
                "RestartSec=1s",
                "RuntimeDirectory=sky-cua",
                "RuntimeDirectoryMode=0775",
                "",
                "[Install]",
                "WantedBy=multi-user.target",
                "",
            ]
        ),
        encoding="utf-8",
    )
    run_sudo(["install", "-D", "-m", "0644", str(env_path), "/etc/sky-cua/input-helper.env"])
    run_sudo(
        [
            "install",
            "-D",
            "-m",
            "0644",
            str(service_path),
            "/etc/systemd/system/sky-cua-input-helper.service",
        ]
    )
    run_sudo(["systemctl", "daemon-reload"])
    run_sudo(["systemctl", "enable", "--now", "sky-cua-input-helper.service"])
    run_sudo(["systemctl", "restart", "sky-cua-input-helper.service"])
    print(
        "Installed and started sky-cua-input-helper.service "
        f"(socket group: {group}, socket: /run/sky-cua/input-helper.sock)."
    )


def default_input_helper_group() -> str:
    value = os.environ.get("SKY_CUA_INPUT_HELPER_SOCKET_GROUP")
    if value:
        return value
    return grp.getgrgid(os.getgid()).gr_name


def run_sudo(args: list[str]) -> None:
    subprocess.run(["sudo", *args], check=True)


def refresh_accessibility_bus() -> None:
    """Best-effort reset of a wedged user AT-SPI bus before sky-cua reconnects."""
    if not sys.platform.startswith("linux"):
        return
    if not os.environ.get("DBUS_SESSION_BUS_ADDRESS") or not os.environ.get("XDG_RUNTIME_DIR"):
        return
    systemctl = shutil.which("systemctl")
    if systemctl is None:
        return
    import pwd

    user = os.environ.get("USER")
    try:
        expected_user = pwd.getpwuid(os.getuid()).pw_name
    except KeyError:
        expected_user = None
    if user and expected_user and user != expected_user:
        print(
            f"warning: USER={user!r} does not match current uid {os.getuid()}; "
            "skipping AT-SPI registry pkill",
            file=sys.stderr,
        )
        user = None
    pkill = shutil.which("pkill")
    if pkill and user:
        with contextlib.suppress(OSError, subprocess.TimeoutExpired):
            subprocess.run(
                [pkill, "-u", user, "-f", r"(^|/)at-spi2-registryd( |$)"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
                timeout=AT_SPI_RESTART_TIMEOUT_SECONDS,
            )
    try:
        result = subprocess.run(
            [systemctl, "--user", "restart", "at-spi-dbus-bus.service"],
            capture_output=True,
            text=True,
            check=False,
            timeout=AT_SPI_RESTART_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"warning: could not refresh user AT-SPI accessibility bus: {error}", file=sys.stderr)
        return
    if result.returncode == 0:
        print("Refreshed user AT-SPI accessibility bus.")
        return
    detail = (result.stderr or result.stdout).strip()
    if detail:
        print(
            f"warning: could not refresh user AT-SPI accessibility bus: {detail}", file=sys.stderr
        )


def install_local_mcp_server(
    target_dir: Path,
    host: str,
    *,
    openclaw_dir: Path = DEFAULT_OPENCLAW_DIR,
    restart_runtime: bool = False,
    bundle_root: Path | None = None,
    claude_config_dir: Path | None = None,
    refresh_accessibility: bool = True,
    install_input_helper: bool = False,
    input_helper_group: str | None = None,
    browser_eval: str | None = None,
    model_supports_images: str | None = None,
    presence_enabled: str | None = None,
) -> tuple[Path, Path]:
    """Install runtime binaries and host config; optionally restart installed runtimes.

    Returns ``(client_path, config_path)``. Other deploy scripts call this to
    refresh a local MCP-server install alongside their own publishing steps.
    With ``bundle_root``, binaries come from that built bundle instead of
    target/release so all channels ship identical bits.
    """
    launch_policy = resolve_mcp_launch_policy(
        target_dir,
        browser_eval=browser_eval,
        model_supports_images=model_supports_images,
        presence_enabled=presence_enabled,
    )
    target_dir.mkdir(parents=True, exist_ok=True)

    if restart_runtime and sys.platform == "win32":
        restart_runtime_processes(target_dir, refresh_accessibility=refresh_accessibility)

    if bundle_root is not None:
        resource_root = runtime_resource_root(bundle_root)
        client_path = install_bundle_binaries(bundle_root, target_dir)
    else:
        resource_root = runtime_resource_root(None)
        client_path = install_binaries(target_dir)

    seeded = _install_shared.seed_machine_config_from_environment()
    if seeded is not None:
        print(f"Seeded machine config browser selection: {seeded}")

    if host == "opencode":
        config_path = install_opencode(target_dir, client_path, resource_root, launch_policy)
    elif host == "claude-code":
        config_path = install_claude_code(
            target_dir, client_path, claude_config_dir, resource_root, launch_policy
        )
    elif host == "claude-desktop":
        config_path = install_claude_desktop(target_dir, client_path, resource_root, launch_policy)
    elif host == "pi":
        config_path = install_pi(
            target_dir, client_path, resource_root=resource_root, launch_policy=launch_policy
        )
    elif host == "openclaw":
        config_path = install_openclaw(
            target_dir,
            client_path,
            openclaw_dir=openclaw_dir,
            resource_root=resource_root,
            launch_env=launch_policy.env(),
        )
    else:
        config = generate_mcp_config(client_path, target_dir, resource_root, launch_policy)
        config_path = write_mcp_json(target_dir, config)
    write_mcp_launch_policy_state(target_dir, launch_policy)

    if restart_runtime:
        restart_runtime_processes(target_dir, refresh_accessibility=refresh_accessibility)
        print(f"Stopped installed sky-cua runtime processes rooted under: {target_dir}")

    if install_input_helper:
        install_input_helper_service(target_dir, socket_group=input_helper_group)

    return client_path, config_path


def print_next_steps(host: str, target_dir: Path, client_path: Path, config_path: Path) -> None:
    print(f"\nInstalled sky-cua MCP server to: {target_dir}")
    print(f"Client binary: {client_path}")
    print(f"Config written: {config_path}")
    print(
        "For local development updates, rerun this installer with --restart-runtime "
        "to stop installed sky-cua MCP daemons after copying new binaries."
    )

    if host == "opencode":
        print("\nNext steps for OpenCode:")
        print(f"  1. Copy {config_path} to your opencode project root as opencode.json")
        print("     OR merge the 'mcp.sky_cua' section into your existing opencode.json")
        print("  2. Run: opencode mcp list")
        print(
            "  3. Run: opencode run 'Use the sky_cua MCP tool "
            'list_resources with surface="desktop" and resource="apps"\''
        )
        print("  4. Restart or reload the OpenCode session if it does not reconnect automatically")
    elif host == "claude-code":
        print("\nNext steps for Claude Code:")
        print(f"  1. Snippet written for inspection: {config_path}")
        print("  2. If the claude CLI was found, the sky-cua server was registered at user scope;")
        print("     otherwise run the printed claude mcp add-json command")
        print("  3. If ~/.claude exists, sky-cua skills were copied into ~/.claude/skills")
        print("  4. ~/.claude/settings.json now denies the built-in computer-use MCP and")
        print("     auto-approves the sky-cua tools (mcp__sky-cua__*)")
        print(
            "  5. Run: claude mcp list, then ask Claude to use the sky-cua "
            'list_resources tool with surface="desktop" and resource="apps"'
        )
    elif host == "claude-desktop":
        print("\nNext steps for Claude Desktop:")
        print(f"  1. Merge {config_path} into your Claude Desktop config:")
        if sys.platform == "darwin":
            print("     ~/Library/Application Support/Claude/claude_desktop_config.json")
        elif sys.platform == "linux":
            print("     ~/.config/Claude/claude_desktop_config.json")
        else:
            print("     %APPDATA%\\Claude\\claude_desktop_config.json")
        print("  2. Restart Claude Desktop")
    elif host == "pi":
        print("\nNext steps for Pi:")
        print(f"  1. Snippet written for inspection: {config_path}")
        print("  2. If ~/.pi/agent exists, sky_cua was merged into ~/.pi/agent/mcp.json")
        print("     and sky-cua skills were copied into ~/.pi/agent/skills")
        print("  3. Ensure pi-mcp-adapter is installed: npm install -g pi-mcp-adapter")
        print("  4. Restart Pi or run /reload after --restart-runtime stops the old MCP process")
    elif host == "openclaw":
        print("\nNext steps for OpenClaw:")
        print(f"  1. Snippet written for inspection: {config_path}")
        print("  2. sky_cua was registered with: openclaw mcp set sky_cua <config>")
        print("     and cached MCP runtimes were reloaded with: openclaw mcp reload")
        print("  3. Verify the deployment: python3 scripts/live_openclaw_mcp_smoke.py")
        print("     (add --agent-turn to also run a live agent turn through the Gateway)")
    else:
        print("\nNext steps for generic MCP hosts:")
        print(f"  1. Reference {config_path} in your host's MCP server registry")
        print("  2. Ensure the host forwards desktop session env vars to the spawned process")
        print("     or rely on the runtime's built-in auto-detection")


def main() -> int:
    parser = argparse.ArgumentParser(description="Install sky-cua as a generic MCP server.")
    parser.add_argument(
        "--target-dir",
        type=Path,
        default=DEFAULT_LOCAL_INSTALL_DIR,
        help=f"Installation directory (default: {DEFAULT_LOCAL_INSTALL_DIR})",
    )
    parser.add_argument(
        "--host",
        choices=MCP_HOST_CHOICES,
        default="generic",
        help="Host-specific MCP config format to emit.",
    )
    parser.add_argument(
        "--openclaw-dir",
        type=Path,
        default=DEFAULT_OPENCLAW_DIR,
        help=f"OpenClaw state directory for --host openclaw (default: {DEFAULT_OPENCLAW_DIR}).",
    )
    parser.add_argument(
        "--claude-config-dir",
        type=Path,
        default=None,
        help="Claude Code config directory for --host claude-code (default: ~/.claude).",
    )
    parser.add_argument(
        "--browser-eval",
        choices=("on", "off"),
        default=None,
        help="Persist browser_eval availability for launched servers.",
    )
    parser.add_argument(
        "--presence",
        choices=("on", "off"),
        default=None,
        help=(
            "Persist session-presence (auto-unlock) for launched servers. "
            "Deploys default to on; pass off to disable."
        ),
    )
    parser.add_argument(
        "--model-supports-images",
        choices=("true", "false"),
        default=None,
        help="Persist an explicit model image-capability override for launched servers.",
    )
    parser.add_argument(
        "--bin-dir",
        type=Path,
        default=None,
        help="Also symlink/copy binaries into this directory (e.g. ~/.local/bin).",
    )
    parser.add_argument(
        "--restart-runtime",
        action="store_true",
        help=(
            "After installing, refresh the user AT-SPI bus on Linux and stop sky-cua runtime "
            "processes rooted under --target-dir so MCP hosts respawn updated binaries."
        ),
    )
    parser.add_argument(
        "--input-helper",
        action="store_true",
        help=(
            "On Linux, install and start the privileged root sky-cua-input-helper "
            "systemd service for uinput keyboard injection and raw pointer observation."
        ),
    )
    parser.add_argument(
        "--input-helper-group",
        default=None,
        help=(
            "Group allowed to connect to /run/sky-cua/input-helper.sock when "
            "--input-helper is used (default: current user's primary group, or "
            "SKY_CUA_INPUT_HELPER_SOCKET_GROUP)."
        ),
    )
    parser.add_argument(
        "--kwin-effect",
        action="store_true",
        help=(
            "Also build, install (sudo cmake --install), and reload the sky-cua "
            "KWin agent-cursor effect (Linux/KDE only)."
        ),
    )
    args = parser.parse_args()

    target_dir = args.target_dir.resolve()
    client_path, config_path = install_local_mcp_server(
        target_dir,
        args.host,
        openclaw_dir=args.openclaw_dir.expanduser().resolve(),
        restart_runtime=args.restart_runtime,
        claude_config_dir=(
            args.claude_config_dir.expanduser().resolve()
            if args.claude_config_dir is not None
            else None
        ),
        install_input_helper=args.input_helper,
        input_helper_group=args.input_helper_group,
        browser_eval=args.browser_eval,
        model_supports_images=args.model_supports_images,
        presence_enabled=args.presence,
    )

    if args.bin_dir:
        link_current_platform_binaries(target_dir, args.bin_dir.expanduser().resolve())

    if args.kwin_effect:
        outcome = deploy_kwin_effect(build_dir=target_dir / "kwin-effect-build")
        print_kwin_effect_deploy_outcome(outcome)
        if kwin_effect_deploy_failed(outcome):
            print(
                f"error: KWin effect {outcome.effect_id} did not converge; "
                f"restored {outcome.rollback_effect_id or 'no previous effect'}",
                file=sys.stderr,
            )
            return 1

    print_next_steps(args.host, target_dir, client_path, config_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
