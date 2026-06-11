#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

from _kwin_effect import deploy_kwin_effect
from _plugin_bundle import (
    DEFAULT_CODEX_HOME,
    DIST_PLUGIN_ROOT,
    RELEASE_PLUGIN_ID,
    build_bundle,
    compat_plugin_available,
    ensure_bundle_structure,
    installed_plugin_root,
    stop_unix_runtime_processes,
    stop_windows_cache_processes,
    update_codex_config,
)
from install_plugin import install_bundle, run_browser_preflight


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build and deploy the sky-cua debug plugin into the Codex cache."
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=DEFAULT_CODEX_HOME,
        help="Codex home directory to install into (default: ~/.codex).",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Install the existing dist/plugin/sky-cua bundle without rebuilding.",
    )
    parser.add_argument(
        "--symlink",
        action="store_true",
        help="Symlink the bundle into the debug cache instead of copying it.",
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

    if not args.no_build:
        build_bundle()

    bundle_root = DIST_PLUGIN_ROOT.resolve()
    ensure_bundle_structure(bundle_root)
    destination = installed_plugin_root(args.codex_home)
    stop_unix_runtime_processes([destination])
    stop_windows_cache_processes(destination)
    install_bundle(bundle_root, destination, args.symlink)
    run_browser_preflight(destination, args.codex_home)
    config_path = args.codex_home / "config.toml"
    # Compat-first: the preflight above retargets the computer-use compat
    # plugin at this debug payload; channel ids stay disabled. When the
    # bundle ships no openai-bundled resources (no compat root), fall back to
    # enabling the debug channel id directly.
    update_codex_config(
        config_path,
        disabled_plugin_ids=[RELEASE_PLUGIN_ID],
        compat_enablement=compat_plugin_available(args.codex_home),
    )
    print(f"installed_path={destination}")
    print(f"config_path={config_path}")

    # This lane only stops the Codex cache runtime. The installed-MCP runtime
    # used by Claude Code and other hosts is restarted through
    # `install_mcp_server.py --restart-runtime`.
    if args.kwin_effect:
        outcome = deploy_kwin_effect(build_dir=destination.parent / "kwin-effect-build")
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
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
