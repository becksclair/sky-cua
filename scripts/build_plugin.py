#!/usr/bin/env python3
from __future__ import annotations

import argparse
import shutil
import subprocess
from pathlib import Path

from _plugin_bundle import (
    DIST_PLUGIN_ROOT,
    REPO_ROOT,
    ensure_bundle_structure,
    ensure_executable,
    remove_path,
)


def build_release_binaries() -> None:
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--package",
            "sky-cua-client",
            "--package",
            "sky-cua-service",
        ],
        cwd=REPO_ROOT,
        check=True,
    )


def stage_bundle(bundle_root: Path) -> None:
    temp_root = bundle_root.parent / f".{bundle_root.name}.tmp"
    remove_path(temp_root)
    temp_root.mkdir(parents=True, exist_ok=True)

    for directory in [".codex-plugin", "resources", "skills", "docs"]:
        shutil.copytree(REPO_ROOT / directory, temp_root / directory)
    for file_name in [".mcp.json", "README.md"]:
        shutil.copy2(REPO_ROOT / file_name, temp_root / file_name)

    bin_dir = temp_root / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    for binary_name in ["sky-cua-client", "sky-cua-service"]:
        source = REPO_ROOT / "target" / "release" / binary_name
        destination = bin_dir / binary_name
        shutil.copy2(source, destination)
        ensure_executable(destination)

    ensure_bundle_structure(temp_root)
    remove_path(bundle_root)
    temp_root.replace(bundle_root)


def main() -> int:
    parser = argparse.ArgumentParser(description="Build a distributable sky-cua plugin bundle.")
    parser.add_argument(
        "--dist-root",
        type=Path,
        default=DIST_PLUGIN_ROOT,
        help="Bundle output directory (default: dist/plugin/sky-cua).",
    )
    args = parser.parse_args()

    build_release_binaries()
    args.dist_root.parent.mkdir(parents=True, exist_ok=True)
    stage_bundle(args.dist_root)
    print(args.dist_root)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
