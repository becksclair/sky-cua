#!/usr/bin/env python3
from __future__ import annotations

import argparse
import shutil
from pathlib import Path

from _plugin_bundle import (
    REPO_ROOT,
    current_runtime_platform,
    ensure_executable,
    platform_runtime_binary_base_names,
    runtime_binary_source_name,
)


def package_runtime_artifact(platform_id: str, output_root: Path) -> Path:
    binary_names = platform_runtime_binary_base_names(platform_id)
    artifact_root = output_root / platform_id
    if artifact_root.exists():
        shutil.rmtree(artifact_root)
    artifact_root.mkdir(parents=True, exist_ok=True)
    for binary_name in binary_names:
        source_name = runtime_binary_source_name(platform_id, binary_name)
        source = REPO_ROOT / "target" / "release" / source_name
        if not source.exists():
            raise FileNotFoundError(f"runtime binary not found: {source}")
        destination = artifact_root / source_name
        shutil.copy2(source, destination)
        if not destination.name.endswith(".exe"):
            ensure_executable(destination)
    return artifact_root


def main() -> int:
    parser = argparse.ArgumentParser(description="Package one sky-cua runtime artifact.")
    parser.add_argument(
        "--platform",
        default=current_runtime_platform(),
        help="Runtime platform id: linux-x64, linux-arm64, or windows-x64.",
    )
    parser.add_argument(
        "--output-root",
        type=Path,
        default=REPO_ROOT / "artifacts" / "runtime",
        help="Directory that will receive <platform>/ binaries.",
    )
    args = parser.parse_args()
    print(package_runtime_artifact(args.platform, args.output_root.resolve()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
