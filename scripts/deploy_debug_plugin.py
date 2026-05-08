#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

from _plugin_bundle import (
    DEFAULT_CODEX_HOME,
    DIST_PLUGIN_ROOT,
    RELEASE_PLUGIN_ID,
    build_bundle,
    ensure_bundle_structure,
    installed_plugin_root,
    stop_windows_cache_processes,
    update_codex_config,
)
from install_plugin import install_bundle


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
    args = parser.parse_args()

    if not args.no_build:
        build_bundle()

    bundle_root = DIST_PLUGIN_ROOT.resolve()
    ensure_bundle_structure(bundle_root)
    destination = installed_plugin_root(args.codex_home)
    stop_windows_cache_processes(destination)
    install_bundle(bundle_root, destination, args.symlink)
    config_path = args.codex_home / "config.toml"
    update_codex_config(config_path, disabled_plugin_ids=[RELEASE_PLUGIN_ID])
    print(f"installed_path={destination}")
    print(f"config_path={config_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
