#!/usr/bin/env python3
"""Install sky-cua as a generic MCP server for any MCP-compatible host.

This script copies the built runtime binaries to a stable installation directory
and emits a host-ready MCP server configuration with absolute paths.

Supported host targets:
  - opencode        OpenCode via opencode.json
  - claude-desktop  Claude Desktop via claude_desktop_config.json
  - generic         Raw .mcp.json for manual wiring (default)

Usage:
  python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host opencode
"""

from __future__ import annotations

import argparse
import json
import shutil
import stat
import sys
from pathlib import Path

from _plugin_bundle import (
    LINUX_ARM64,
    LINUX_X64,
    WINDOWS_X64,
    current_runtime_platform,
    platform_runtime_binary_base_names,
    runtime_binary_path,
    runtime_binary_source_name,
)

# Relative to the script location, which lives under scripts/
REPO_ROOT = Path(__file__).resolve().parents[1]


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
    shutil.copy2(src, dst)
    if sys.platform != "win32" and not dst.name.endswith(".exe"):
        mode = dst.stat().st_mode
        dst.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


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
    path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    return path


def install_opencode(target_dir: Path, client_path: Path) -> Path:
    """Update or create opencode.json in the target directory."""
    opencode_config: dict[str, object] = {
        "$schema": "https://opencode.ai/config.json",
        "mcp": {
            "sky_cua": {
                "type": "local",
                "command": [str(client_path), "mcp"],
                "environment": {},
                "enabled": True,
                "timeout": 30000,
            }
        },
    }

    path = target_dir / "opencode.json"
    path.write_text(json.dumps(opencode_config, indent=2) + "\n", encoding="utf-8")
    return path


def install_claude_desktop(target_dir: Path, client_path: Path) -> Path:
    """Emit a Claude Desktop config snippet and print instructions."""

    # Claude Desktop uses a slightly different shape in ~/Library/Application Support/Claude/claude_desktop_config.json
    # or on Linux: ~/.config/Claude/claude_desktop_config.json
    snippet = {
        "mcpServers": {
            "computer-use": {
                "command": str(client_path),
                "args": ["mcp"],
                "env": {},
            }
        }
    }

    path = target_dir / "claude_desktop_config.json"
    path.write_text(json.dumps(snippet, indent=2) + "\n", encoding="utf-8")
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


def print_next_steps(host: str, target_dir: Path, client_path: Path, config_path: Path) -> None:
    print(f"\nInstalled sky-cua MCP server to: {target_dir}")
    print(f"Client binary: {client_path}")
    print(f"Config written: {config_path}")

    if host == "opencode":
        print("\nNext steps for OpenCode:")
        print(f"  1. Copy {config_path} to your opencode project root as opencode.json")
        print("     OR merge the 'mcp.sky_cua' section into your existing opencode.json")
        print("  2. Run: opencode mcp list")
        print("  3. Run: opencode run 'Use the sky_cua MCP tool list_apps'")
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
        choices=("generic", "opencode", "claude-desktop"),
        default="generic",
        help="Host-specific MCP config format to emit.",
    )
    parser.add_argument(
        "--bin-dir",
        type=Path,
        default=None,
        help="Also symlink/copy binaries into this directory (e.g. ~/.local/bin).",
    )
    args = parser.parse_args()

    target_dir = args.target_dir.resolve()
    target_dir.mkdir(parents=True, exist_ok=True)

    client_path = install_binaries(target_dir)

    if args.host == "opencode":
        config_path = install_opencode(target_dir, client_path)
    elif args.host == "claude-desktop":
        config_path = install_claude_desktop(target_dir, client_path)
    else:
        config = generate_mcp_config(client_path, target_dir)
        config_path = write_mcp_json(target_dir, config)

    if args.bin_dir:
        link_current_platform_binaries(target_dir, args.bin_dir.expanduser().resolve())

    print_next_steps(args.host, target_dir, client_path, config_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
