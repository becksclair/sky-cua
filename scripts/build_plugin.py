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
    mcp_config_source,
    remove_path,
    runtime_binary_names,
)

BUNDLE_SOURCE_PATHS = [
    Path(".codex-plugin"),
    Path("resources"),
    Path("skills"),
    Path("docs"),
    Path("README.md"),
]


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


def tracked_bundle_files(source_paths: list[Path] | None = None) -> list[Path]:
    paths = source_paths if source_paths is not None else BUNDLE_SOURCE_PATHS
    result = subprocess.run(
        ["git", "ls-files", "-z", "--", *[str(path) for path in paths]],
        cwd=REPO_ROOT,
        check=True,
        stdout=subprocess.PIPE,
    )
    return [Path(part.decode("utf-8")) for part in result.stdout.split(b"\0") if part]


def copy_tracked_bundle_sources(temp_root: Path) -> None:
    for relative_path in tracked_bundle_files():
        source = REPO_ROOT / relative_path
        destination = temp_root / relative_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def stage_bundle(bundle_root: Path) -> None:
    temp_root = bundle_root.parent / f".{bundle_root.name}.tmp"
    remove_path(temp_root)
    temp_root.mkdir(parents=True, exist_ok=True)

    copy_tracked_bundle_sources(temp_root)
    shutil.copy2(mcp_config_source(), temp_root / ".mcp.json")

    bin_dir = temp_root / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    for binary_name in runtime_binary_names():
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
