#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
from pathlib import Path

from _plugin_bundle import (
    DEFAULT_CODEX_HOME,
    DIST_PLUGIN_ROOT,
    copytree_replace_preserving_platform_binaries,
    ensure_bundle_structure,
    ensure_executable,
    installed_plugin_root,
    remove_path,
    runtime_binary_names,
    update_codex_config,
)


def install_bundle(bundle_root: Path, destination: Path, symlink: bool) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if symlink:
        remove_path(destination)
        os.symlink(bundle_root, destination, target_is_directory=True)
    else:
        copytree_replace_preserving_platform_binaries(bundle_root, destination)
    ensure_bundle_structure(destination)
    for binary_name in runtime_binary_names():
        ensure_executable(destination / "bin" / binary_name)


def main() -> int:
    parser = argparse.ArgumentParser(description="Install the built sky-cua plugin into ~/.codex.")
    parser.add_argument(
        "--bundle-root",
        type=Path,
        default=DIST_PLUGIN_ROOT,
        help="Path to a built bundle (default: dist/plugin/sky-cua).",
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=DEFAULT_CODEX_HOME,
        help="Codex home directory to install into (default: ~/.codex).",
    )
    parser.add_argument(
        "--symlink",
        action="store_true",
        help="Symlink the bundle instead of copying it.",
    )
    args = parser.parse_args()

    bundle_root = args.bundle_root.resolve()
    ensure_bundle_structure(bundle_root)

    destination = installed_plugin_root(args.codex_home)
    install_bundle(bundle_root, destination, args.symlink)

    config_path = args.codex_home / "config.toml"
    update_codex_config(config_path)
    print(f"installed_path={destination}")
    print(f"config_path={config_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
