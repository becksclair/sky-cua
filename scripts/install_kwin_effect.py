#!/usr/bin/env python3
"""Build, install, and reload the sky-cua KWin agent-cursor effect locally.

Default action: configure and compile the effect from the current sources,
install it system-wide (sudo cmake --install) under the next generated effect
id, enable it persistently in kwinrc, unload the previous generated/stable id,
then drive the running KWin to the new build. This tool never restarts KWin.

Usage:
  python3 scripts/install_kwin_effect.py
  python3 scripts/install_kwin_effect.py --status
  python3 scripts/install_kwin_effect.py --no-notify   # accepted legacy no-op
"""

from __future__ import annotations

import argparse
import json
import shlex
import sys
from pathlib import Path

from _kwin_effect import (
    REPO_ROOT,
    deploy_kwin_effect,
    effect_status,
    kwin_effect_deploy_failed,
)

DEFAULT_BUILD_DIR = REPO_ROOT / "artifacts" / "kwin-effect-deploy" / "build"


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build, install, and reload the sky-cua KWin agent-cursor effect."
    )
    parser.add_argument(
        "--status",
        action="store_true",
        help="Print effect/session status as JSON and exit (no build, no sudo).",
    )
    parser.add_argument(
        "--no-notify",
        action="store_true",
        help="Skip the desktop notification when an update needs a session restart.",
    )
    parser.add_argument(
        "--prefix",
        type=Path,
        default=Path("/usr"),
        help="CMake install prefix (default: /usr; KWin only discovers system paths).",
    )
    parser.add_argument(
        "--build-dir",
        type=Path,
        default=DEFAULT_BUILD_DIR,
        help=f"CMake build directory (default: {DEFAULT_BUILD_DIR}).",
    )
    parser.add_argument(
        "--sudo-cmd",
        default="sudo",
        help="Privilege-escalation command for the install step (default: sudo).",
    )
    parser.add_argument(
        "--no-enable",
        action="store_true",
        help="Skip enabling the effect persistently in kwinrc (debugging escape hatch).",
    )
    args = parser.parse_args()

    if args.status:
        print(json.dumps(effect_status(), indent=2, sort_keys=True))
        return 0

    try:
        outcome = deploy_kwin_effect(
            build_dir=args.build_dir.resolve(),
            install_prefix=args.prefix,
            sudo_cmd=shlex.split(args.sudo_cmd),
            notify=not args.no_notify,
            enable_persistently=not args.no_enable,
        )
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    if outcome.converged:
        print(
            f"KWin effect {outcome.effect_id} is loaded and current "
            f"(build id {outcome.expected_build_id})"
        )
        return 0

    if outcome.session_restart_required:
        print(
            "KWin effect updated; the new build activates when you restart your "
            "Plasma session (log out and back in) at your convenience.",
        )
        return 0

    if not outcome.loaded:
        if kwin_effect_deploy_failed(outcome):
            print(
                f"KWin effect {outcome.effect_id} did not converge; "
                f"restored {outcome.rollback_effect_id or 'no previous effect'}.",
                file=sys.stderr,
            )
            return 1
        print(
            f"KWin effect {outcome.effect_id} installed and enabled for the next "
            "Plasma session start."
        )
        return 0

    print(
        "KWin effect did not converge; see notes above and "
        "`python3 scripts/install_kwin_effect.py --status`.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
