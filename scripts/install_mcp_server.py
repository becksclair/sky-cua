#!/usr/bin/env python3
"""Install sky-cua as a generic MCP server for any MCP-compatible host.

This script copies the built runtime binaries to a stable installation directory
and emits a host-ready MCP server configuration with absolute paths.

Supported host targets:
  - opencode        OpenCode via opencode.json
  - claude-desktop  Claude Desktop via claude_desktop_config.json
  - pi              Pi via mcp.json
  - generic         Raw .mcp.json for manual wiring (default)

Usage:
  python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host opencode
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import stat
import subprocess
import sys
from pathlib import Path

from _kwin_effect import deploy_kwin_effect
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

# Relative to the script location, which lives under scripts/
REPO_ROOT = Path(__file__).resolve().parents[1]
BROWSER_SELECTION_ENV = "SKY_CUA_BROWSER"
SKY_CUA_SKILLS = ("computer-use", "browser-use")
DEFAULT_OPENCLAW_DIR = Path.home() / ".openclaw"
OPENCLAW_MCP_SET_TIMEOUT_SECONDS = 30


def current_platform() -> str:
    return current_runtime_platform()


def entrypoint_path(platform_id: str, name: str) -> Path:
    if platform_id == WINDOWS_X64:
        return Path("bin") / f"{name}.exe"
    return Path("bin") / name


def source_binary_path(name: str) -> Path:
    return REPO_ROOT / "target" / "release" / name


def ensure_parent(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


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


def atomic_sibling_path(path: Path, suffix: str) -> Path:
    return path.with_name(f".{path.name}.{suffix}-{os.getpid()}")


def write_text_atomically(path: Path, text: str, mode: int | None = None) -> None:
    ensure_parent(path)
    write_path = path.resolve(strict=False) if path.is_symlink() else path
    if write_path != path:
        ensure_parent(write_path)
    target_mode = mode
    if target_mode is None and write_path.exists():
        target_mode = stat.S_IMODE(write_path.stat().st_mode)
    temp_path = atomic_sibling_path(write_path, "tmp")
    remove_path(temp_path)
    try:
        temp_path.write_text(text, encoding="utf-8")
        if target_mode is not None:
            temp_path.chmod(target_mode)
        os.replace(temp_path, write_path)
    finally:
        remove_path(temp_path)


def replace_tree_atomically(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temp_path = atomic_sibling_path(destination, "tmp")
    backup_path = atomic_sibling_path(destination, "backup")
    backup_in_use = False
    remove_path(temp_path)
    remove_path(backup_path)
    try:
        shutil.copytree(source, temp_path)
        if destination.exists() or destination.is_symlink():
            os.replace(destination, backup_path)
            backup_in_use = True
            try:
                os.replace(temp_path, destination)
            except OSError:
                os.replace(backup_path, destination)
                backup_in_use = False
                raise
            remove_path(backup_path)
            backup_in_use = False
        else:
            os.replace(temp_path, destination)
    finally:
        remove_path(temp_path)
        if not backup_in_use:
            remove_path(backup_path)


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
                REPO_ROOT
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


def generate_mcp_config(client_path: Path, target_dir: Path) -> dict[str, object]:
    """Build an MCP server config dict with absolute paths."""
    return {
        "mcpServers": {
            "computer-use": {
                "command": str(client_path),
                "args": ["mcp"],
                "env_vars": [
                    "CODEX_COMPUTER_USE_COSMIC_HELPER",
                    "DBUS_SESSION_BUS_ADDRESS",
                    "DESKTOP_SESSION",
                    "DISPLAY",
                    "SKY_CUA_AGENT_CURSOR",
                    BROWSER_SELECTION_ENV,
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
                    "SKY_CUA_PORTAL_EIS",
                    "SKY_CUA_REPO_ROOT",
                    "SKY_CUA_SCREENSHOT_CURSOR",
                    "SKY_CUA_SERVICE_PATH",
                    "SKY_CUA_SERVICE_TCP_ADDR",
                    "SKY_CUA_SERVICE_SOCKET_PATH",
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


def browser_selection_environment() -> dict[str, str]:
    value = os.environ.get(BROWSER_SELECTION_ENV)
    if value is None or not value.strip():
        return {}
    return {BROWSER_SELECTION_ENV: value.strip()}


def install_opencode(target_dir: Path, client_path: Path) -> Path:
    """Update or create opencode.json in the target directory."""
    opencode_config: dict[str, object] = {
        "$schema": "https://opencode.ai/config.json",
        "mcp": {
            "sky_cua": {
                "type": "local",
                "command": [str(client_path), "mcp"],
                "environment": {
                    "SKY_CUA_REPO_ROOT": str(REPO_ROOT),
                    **browser_selection_environment(),
                },
                "enabled": True,
                "timeout": 30000,
            }
        },
    }

    path = target_dir / "opencode.json"
    write_text_atomically(path, json.dumps(opencode_config, indent=2) + "\n")
    return path


def install_claude_desktop(target_dir: Path, client_path: Path) -> Path:
    """Emit a Claude Desktop config snippet and print instructions."""

    # Claude Desktop uses a slightly different shape in ~/Library/Application Support/Claude/claude_desktop_config.json
    # or on Linux: ~/.config/Claude/claude_desktop_config.json
    snippet: dict[str, object] = {
        "mcpServers": {
            "computer-use": {
                "command": str(client_path),
                "args": ["mcp"],
                "env": {
                    "SKY_CUA_REPO_ROOT": str(REPO_ROOT),
                    **browser_selection_environment(),
                },
            }
        }
    }

    path = target_dir / "claude_desktop_config.json"
    write_text_atomically(path, json.dumps(snippet, indent=2) + "\n")
    return path


def install_pi(
    target_dir: Path,
    client_path: Path,
    pi_agent_dir: Path | None = None,
) -> Path:
    """Emit a Pi mcp.json config snippet for merging into ~/.pi/agent/mcp.json.

    Pi does not support the ``env`` field in MCP server configs, so we generate a
    small wrapper script that sets ``SKY_CUA_REPO_ROOT`` and then execs the real
    client binary.
    """
    wrapper_path = target_dir / "pi_mcp_wrapper.sh"
    selection_exports = "".join(
        f"export {name}={shlex.quote(value)}\n"
        for name, value in browser_selection_environment().items()
    )
    wrapper_content = "".join(
        [
            "#!/usr/bin/env bash\n",
            f"export SKY_CUA_REPO_ROOT={shlex.quote(str(REPO_ROOT))}\n",
            selection_exports,
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


def install_sky_cua_skills(skills_dir: Path) -> None:
    skills_dir.mkdir(parents=True, exist_ok=True)
    for skill_name in SKY_CUA_SKILLS:
        source = REPO_ROOT / "skills" / skill_name
        if not source.exists():
            raise FileNotFoundError(f"sky-cua skill source not found: {source}")
        destination = skills_dir / skill_name
        replace_tree_atomically(source, destination)


def install_pi_skills(skills_dir: Path) -> None:
    install_sky_cua_skills(skills_dir)


def install_openclaw(
    target_dir: Path,
    client_path: Path,
    openclaw_dir: Path | None = None,
    openclaw_bin: str = "openclaw",
) -> Path:
    """Register sky-cua with OpenClaw and copy sky-cua skills into its workspace."""
    openclaw_state_dir = (openclaw_dir or DEFAULT_OPENCLAW_DIR).expanduser().resolve()
    openclaw_state_dir.mkdir(parents=True, exist_ok=True)
    server: dict[str, object] = {
        "command": str(client_path),
        "args": ["mcp"],
        "cwd": str(target_dir),
        "env": {
            "SKY_CUA_REPO_ROOT": str(REPO_ROOT),
            **browser_selection_environment(),
        },
        "codex": {"defaultToolsApprovalMode": "approve"},
    }
    snippet = {"mcp": {"servers": {"sky_cua": server}}}
    path = target_dir / "openclaw_mcp.json"
    write_text_atomically(path, json.dumps(snippet, indent=2) + "\n")

    command = [
        openclaw_bin,
        "mcp",
        "set",
        "sky_cua",
        json.dumps(server, separators=(",", ":")),
    ]
    env = os.environ.copy()
    env["OPENCLAW_STATE_DIR"] = str(openclaw_state_dir)
    env["OPENCLAW_CONFIG_PATH"] = str(openclaw_state_dir / "openclaw.json")
    try:
        subprocess.run(command, check=True, env=env, timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS)
    except subprocess.TimeoutExpired as error:
        command_text = shlex.join(command)
        raise TimeoutError(
            "timed out registering sky-cua with OpenClaw after "
            f"{OPENCLAW_MCP_SET_TIMEOUT_SECONDS} seconds: {command_text} "
            f"(OPENCLAW_STATE_DIR={openclaw_state_dir})"
        ) from error
    install_sky_cua_skills(openclaw_state_dir / "workspace" / "skills")
    return path


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


def restart_runtime_processes(target_dir: Path) -> None:
    """Stop installed sky-cua runtime processes so hosts respawn fresh binaries."""
    if sys.platform == "win32":
        stop_windows_cache_processes(target_dir)
    else:
        stop_unix_runtime_processes([target_dir])


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
        print("  3. Run: opencode run 'Use the sky_cua MCP tool list_apps'")
        print("  4. Restart or reload the OpenCode session if it does not reconnect automatically")
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
        print("  3. sky-cua skills were copied into the configured OpenClaw workspace")
        print("  4. Restart or reload active OpenClaw agent sessions if they do not reconnect")
    else:
        print("\nNext steps for generic MCP hosts:")
        print(f"  1. Reference {config_path} in your host's MCP server registry")
        print("  2. Ensure the host forwards desktop session env vars to the spawned process")
        print("     or rely on the runtime's built-in auto-detection")


def main() -> int:
    default_target = Path.home() / ".local" / "share" / "sky-cua"

    parser = argparse.ArgumentParser(description="Install sky-cua as a generic MCP server.")
    parser.add_argument(
        "--target-dir",
        type=Path,
        default=default_target,
        help=f"Installation directory (default: {default_target})",
    )
    parser.add_argument(
        "--host",
        choices=("generic", "opencode", "claude-desktop", "pi", "openclaw"),
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
        "--bin-dir",
        type=Path,
        default=None,
        help="Also symlink/copy binaries into this directory (e.g. ~/.local/bin).",
    )
    parser.add_argument(
        "--restart-runtime",
        action="store_true",
        help=(
            "After installing, stop sky-cua runtime processes rooted under --target-dir "
            "so MCP hosts respawn the updated binaries on the next tool call."
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
    target_dir.mkdir(parents=True, exist_ok=True)

    if args.restart_runtime and sys.platform == "win32":
        restart_runtime_processes(target_dir)

    client_path = install_binaries(target_dir)

    if args.host == "opencode":
        config_path = install_opencode(target_dir, client_path)
    elif args.host == "claude-desktop":
        config_path = install_claude_desktop(target_dir, client_path)
    elif args.host == "pi":
        config_path = install_pi(target_dir, client_path)
    elif args.host == "openclaw":
        config_path = install_openclaw(
            target_dir,
            client_path,
            openclaw_dir=args.openclaw_dir.expanduser().resolve(),
        )
    else:
        config = generate_mcp_config(client_path, target_dir)
        config_path = write_mcp_json(target_dir, config)

    if args.bin_dir:
        link_current_platform_binaries(target_dir, args.bin_dir.expanduser().resolve())

    if args.restart_runtime:
        restart_runtime_processes(target_dir)
        print(f"Stopped installed sky-cua runtime processes rooted under: {target_dir}")

    if args.kwin_effect:
        outcome = deploy_kwin_effect(build_dir=target_dir / "kwin-effect-build")
        if outcome.session_restart_required:
            if outcome.notification_delivered:
                print(
                    "KWin effect updated; the new build activates after the next "
                    "Plasma session restart (a desktop notification was shown)."
                )
            else:
                print(
                    "KWin effect updated; the new build activates after the next "
                    "Plasma session restart. The desktop notification could not "
                    "be delivered - tell the user to restart their session when "
                    "convenient."
                )

    print_next_steps(args.host, target_dir, client_path, config_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
