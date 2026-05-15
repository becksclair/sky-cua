#!/usr/bin/env python3
from __future__ import annotations

import argparse
import subprocess

from _plugin_bundle import current_runtime_platform, platform_runtime_binary_base_names


def runtime_package_args(platform_id: str) -> list[str]:
    args: list[str] = []
    for package_name in platform_runtime_binary_base_names(platform_id):
        args.extend(("--package", package_name))
    return args


def build_runtime_packages(platform_id: str) -> None:
    subprocess.run(
        ["cargo", "build", "--release", *runtime_package_args(platform_id)],
        check=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build all release runtime packages for one sky-cua platform."
    )
    parser.add_argument(
        "--platform",
        default=current_runtime_platform(),
        help="Runtime platform id: linux-x64, linux-arm64, or windows-x64.",
    )
    args = parser.parse_args()
    build_runtime_packages(args.platform)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
