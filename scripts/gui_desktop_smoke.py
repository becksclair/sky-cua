#!/usr/bin/env python3
"""Legacy scaffold for GUI-enabled desktop matrix smokes.

The current Linux desktop harness is the Arch testing VM driven by
scripts/run_gui_testing_vm_smoke.py. This script preserves the older CLI and
artifact layout for callers that still reference it.
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
            "GUI desktop smoke scaffold is present; use "
            "scripts/run_gui_testing_vm_smoke.py for current VM-based live proof."
        ),
    }
    (output_dir / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
