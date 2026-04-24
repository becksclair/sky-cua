#!/usr/bin/env python3
from __future__ import annotations

import argparse
import subprocess
from pathlib import Path

from _plugin_bundle import INSTALLED_PLUGIN_ROOT, REPO_ROOT


def default_bundle_root() -> Path:
    if INSTALLED_PLUGIN_ROOT.exists():
        return INSTALLED_PLUGIN_ROOT
    return REPO_ROOT


def main() -> int:
    parser = argparse.ArgumentParser(description="Clear persisted sky-cua portal restore tokens.")
    parser.add_argument(
        "--bundle-root",
        type=Path,
        default=default_bundle_root(),
        help="Bundle root to use (defaults to installed plugin if present, else repo root).",
    )
    args = parser.parse_args()

    bundle_root = args.bundle_root.resolve()
    client = bundle_root / "bin" / "sky-cua-client"
    subprocess.run([str(client), "clear-portal-tokens"], cwd=bundle_root, check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
