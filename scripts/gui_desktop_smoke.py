#!/usr/bin/env python3
"""Scaffold for GUI-enabled desktop matrix smokes.

The real harness will start a profile-specific graphical container. This
script establishes the stable CLI and artifact layout so implementation and
CI wiring can converge without changing callers.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

PROFILES = {"kde", "gnome", "cosmic", "hyprland", "i3"}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True, choices=sorted(PROFILES))
    parser.add_argument("--include-browser-smoke", action="store_true")
    parser.add_argument(
        "--artifacts-root",
        type=Path,
        default=Path("artifacts/gui-desktop-smoke"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    output_dir = args.artifacts_root / args.profile
    output_dir.mkdir(parents=True, exist_ok=True)
    result = {
        "profile": args.profile,
        "include_browser_smoke": args.include_browser_smoke,
        "implemented": False,
        "message": (
            "GUI desktop smoke harness scaffold is present; Docker profile "
            "startup is not implemented yet."
        ),
    }
    (output_dir / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
