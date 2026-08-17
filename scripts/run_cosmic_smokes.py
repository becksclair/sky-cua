#!/usr/bin/env python3
"""Run every live smoke that is runnable on the COSMIC desktop, then print a
single pass/fail table with a root-cause line for each failure.

The runner targets the live COSMIC Wayland session: smokes that need an
external agent (Codex/Hermes/OpenClaw), an attached Android device, the Chrome
extension, or a KDE-only compositor contract are excluded. Each smoke spawns
its own isolated daemon, so results are independent of any long-lived system
service.

Smokes open real windows on the shared desktop, so they run sequentially.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class Smoke:
    label: str
    argv: tuple[str, ...]
    timeout_s: int
    # Known environmental root cause, printed next to the extracted error line.
    failure_hint: str | None = None


SMOKES: tuple[Smoke, ...] = (
    Smoke("session_env", ("scripts/live_session_env_smoke.py",), 300),
    Smoke("text_readback", ("scripts/live_text_readback_smoke.py",), 300),
    Smoke("display_screenshot", ("scripts/live_display_screenshot_smoke.py",), 300),
    Smoke("targeted_screenshot", ("scripts/live_targeted_screenshot_smoke.py",), 300),
    Smoke("wayland_pointer", ("scripts/live_wayland_pointer_smoke.py",), 600),
    Smoke("desktop", ("scripts/live_desktop_smoke.py",), 900),
    Smoke("kate", ("scripts/live_kate_smoke.py",), 600),
    Smoke("kwrite", ("scripts/live_kwrite_smoke.py",), 600),
    Smoke("ghostty", ("scripts/live_ghostty_smoke.py",), 600),
    Smoke("agent_cursor_x11_overlay", ("scripts/live_agent_cursor_x11_overlay_smoke.py",), 300),
    Smoke(
        "wayland_layer_shell_overlay",
        ("scripts/live_wayland_layer_shell_overlay_smoke.py",),
        300,
    ),
    Smoke("portal_downgrade", ("scripts/live_portal_downgrade_smoke.py",), 300),
    # The synthetic cursor smoke proves the software-painted fallback; it
    # self-gates by forcing the overlay host off so the assertion is meaningful
    # on compositors with a working real layer-shell host.
    Smoke(
        "agent_cursor_synthetic",
        ("scripts/live_agent_cursor_kde_smoke.py", "--mode", "synthetic", "--allow-non-kde"),
        300,
    ),
)

# Excluded from the COSMIC run because they need external prerequisites rather
# than the desktop itself.
EXCLUDED: tuple[tuple[str, str], ...] = (
    ("agentic_loop", "needs the installed Codex plugin"),
    ("agent_mcp", "needs an external agent binary (--agent)"),
    ("app_server_session_env", "needs the Codex app-server harness"),
    ("app_server_text_readback", "needs the Codex app-server harness"),
    ("chrome_host_client", "needs the Codex Chrome extension"),
    ("codex_cua", "needs the Codex CLI"),
    ("codex_exec_session_env", "needs the Codex CLI"),
    ("codex_exec_text_readback", "needs the Codex CLI"),
    ("fallback_anchor", "fixture for the agentic-loop smoke"),
    ("hermes_mcp", "needs the Hermes agent"),
    ("kdialog", "compat wrapper — identical to live_desktop_smoke"),
    ("openclaw_mcp", "needs the OpenClaw agent"),
    ("phone_companion_setup", "needs an attached Android device"),
    ("phone_use", "needs an attached Android device"),
    ("phone_workflow", "needs an attached Android device"),
)


@dataclass
class Outcome:
    smoke: Smoke
    passed: bool
    duration_s: float
    cause: str | None
    output: str = ""


def has_live_desktop_session() -> bool:
    """True when the process advertises a live desktop display socket.

    The smokes open real windows and drive real pointer input, so they require
    an actual compositor session rather than a headless CI container. Detection
    is based on the session type and its corresponding display variable.
    """
    session_type = os.environ.get("XDG_SESSION_TYPE", "").strip().lower()
    if session_type == "wayland":
        return bool(os.environ.get("WAYLAND_DISPLAY"))
    if session_type in {"x11", "xorg"}:
        return bool(os.environ.get("DISPLAY"))
    return False


def failure_cause(output: str) -> str | None:
    """Extract the last meaningful error line from a smoke's captured output."""
    for line in reversed(output.splitlines()):
        stripped = line.strip()
        if not stripped:
            continue
        if any(token in stripped for token in ("Error", "Traceback", "Assertion")):
            return stripped
    # Fall back to the last non-empty line when no explicit error token exists.
    for line in reversed(output.splitlines()):
        if line.strip():
            return line.strip()
    return None


def _captured_text(value: str | bytes | None) -> str:
    """Normalize captured subprocess output to str.

    ``subprocess.run(..., text=True)`` yields ``str`` on success, but
    ``TimeoutExpired`` carries raw ``bytes`` regardless of the ``text`` flag.
    """
    if isinstance(value, bytes):
        return value.decode("utf-8", "replace")
    return value or ""


def run_smoke(smoke: Smoke, scale: float) -> Outcome:
    timeout = max(1.0, smoke.timeout_s * scale)
    env = dict(os.environ)
    start = time.monotonic()
    try:
        completed = subprocess.run(
            [sys.executable, *smoke.argv],
            cwd=REPO_ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        output = _captured_text(exc.stdout) + _captured_text(exc.stderr)
        return Outcome(
            smoke=smoke,
            passed=False,
            duration_s=time.monotonic() - start,
            cause=f"timed out after {timeout:g}s"
            + (f": {failure_cause(output)}" if failure_cause(output) else ""),
            output=output,
        )
    output = _captured_text(completed.stdout) + "\n" + _captured_text(completed.stderr)
    passed = completed.returncode == 0
    return Outcome(
        smoke=smoke,
        passed=passed,
        duration_s=time.monotonic() - start,
        cause=None if passed else failure_cause(output),
        output=output,
    )


def report_payload(
    outcomes: list[Outcome],
    *,
    fail_fast_stopped: bool,
    excluded: tuple[tuple[str, str], ...] | None = None,
) -> dict[str, object]:
    """Build the machine-readable summary consumed by the --json output mode."""
    failed = sum(1 for outcome in outcomes if not outcome.passed)
    results = [
        {
            "smoke": outcome.smoke.label,
            "passed": outcome.passed,
            "duration_s": round(outcome.duration_s, 3),
            "cause": outcome.cause,
            "failure_hint": outcome.smoke.failure_hint,
        }
        for outcome in outcomes
    ]
    payload: dict[str, object] = {
        "suite": "cosmic",
        "passed": failed == 0 and not fail_fast_stopped,
        "total": len(outcomes),
        "failed": failed,
        "fail_fast_stopped": fail_fast_stopped,
        "results": results,
    }
    if excluded is not None:
        payload["excluded"] = [{"label": label, "reason": reason} for label, reason in excluded]
    return payload


def write_junit(outcomes: list[Outcome], path: Path) -> None:
    """Write a JUnit XML report with one <testcase> per smoke so CI can
    annotate failures per smoke."""
    from xml.etree import ElementTree as ET

    failed = sum(1 for outcome in outcomes if not outcome.passed)
    suite = ET.Element(
        "testsuite",
        {
            "name": "cosmic",
            "tests": str(len(outcomes)),
            "failures": str(failed),
            "skipped": "0",
        },
    )
    for outcome in outcomes:
        testcase = ET.SubElement(
            suite,
            "testcase",
            {
                "classname": "cosmic",
                "name": outcome.smoke.label,
                "time": f"{outcome.duration_s:.3f}",
            },
        )
        if not outcome.passed:
            ET.SubElement(
                testcase,
                "failure",
                {"message": outcome.cause or "unknown failure"},
            ).text = outcome.cause or "unknown failure"
    tree = ET.ElementTree(suite)
    path.parent.mkdir(parents=True, exist_ok=True)
    tree.write(path, encoding="utf-8", xml_declaration=True)


def print_table(outcomes: list[Outcome], verbose: bool) -> int:
    print("\nCOSMIC desktop smoke results\n")
    print(f"{'smoke':<28} {'result':<8} {'duration':>10}  root cause")
    print("-" * 88)
    failed = 0
    for outcome in outcomes:
        status = "PASS" if outcome.passed else "FAIL"
        row = f"{outcome.smoke.label:<28} {status:<8} {outcome.duration_s:>9.1f}s"
        if outcome.passed:
            print(f"{row}  —")
        else:
            failed += 1
            print(f"{row}  {outcome.cause or 'unknown failure'}")
            if outcome.smoke.failure_hint:
                print(f"{'':<28} {'':<8} {'':>10}  hint: {outcome.smoke.failure_hint}")
    print("-" * 88)
    print(f"{len(outcomes) - failed} passed, {failed} failed")
    return 1 if failed else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", help="comma-separated smoke labels to run")
    parser.add_argument(
        "--timeout-scale", type=float, default=1.0, help="multiply every smoke timeout"
    )
    parser.add_argument(
        "--skip-build", action="store_true", help="do not rebuild release binaries first"
    )
    parser.add_argument(
        "--verbose", action="store_true", help="print full output for failed smokes"
    )
    parser.add_argument(
        "--dry-run", action="store_true", help="print the plan without running anything"
    )
    parser.add_argument(
        "--fail-fast",
        action="store_true",
        help="stop after the first failing smoke instead of running the rest",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit a machine-readable JSON report on stdout instead of the table",
    )
    parser.add_argument(
        "--skip-if-no-desktop",
        action="store_true",
        help="exit 0 with a skip message when no live desktop session is available",
    )
    parser.add_argument(
        "--junit",
        metavar="PATH",
        help="write a JUnit XML report with one testcase per smoke to PATH",
    )
    args = parser.parse_args()

    if args.skip_if_no_desktop and not has_live_desktop_session():
        if args.junit:
            write_junit([], Path(args.junit))
        if args.json:
            print(
                json.dumps(
                    {
                        "suite": "cosmic",
                        "skipped": True,
                        "reason": "no live desktop session",
                        "passed": True,
                        "total": 0,
                        "failed": 0,
                        "results": [],
                    },
                    indent=2,
                )
            )
        else:
            print("cosmic smoke suite skipped: no live desktop session detected.")
        return 0

    selected = SMOKES
    if args.only:
        wanted = {label.strip() for label in args.only.split(",") if label.strip()}
        selected = tuple(smoke for smoke in SMOKES if smoke.label in wanted)
        unknown = wanted - {smoke.label for smoke in SMOKES}
        if unknown:
            raise SystemExit(f"unknown smoke label(s): {', '.join(sorted(unknown))}")

    excluded = () if args.only else EXCLUDED

    if args.dry_run:
        if args.json:
            print(
                json.dumps(report_payload([], fail_fast_stopped=False, excluded=excluded), indent=2)
            )
            return 0
        for smoke in selected:
            print(f"would run: {smoke.label}  ({' '.join(smoke.argv)})")
        return 0

    # JSON mode keeps stdout clean for CI consumers: progress lines go to
    # stderr instead.
    progress = sys.stderr if args.json else sys.stdout

    if not args.skip_build:
        print("rebuilding release binaries...", file=progress)
        subprocess.run(
            [
                "cargo",
                "build",
                "--release",
                "-p",
                "sky-cua-service",
                "-p",
                "sky-cua-client",
                "-p",
                "sky-cua-cosmic-helper",
            ],
            cwd=REPO_ROOT,
            check=True,
        )

    outcomes: list[Outcome] = []
    fail_fast_stopped = False
    for smoke in selected:
        print(f"\n=== {smoke.label} ===", file=progress)
        outcome = run_smoke(smoke, args.timeout_scale)
        outcomes.append(outcome)
        if outcome.passed:
            print(f"PASS ({outcome.duration_s:.1f}s)", file=progress)
        else:
            print(f"FAIL ({outcome.duration_s:.1f}s): {outcome.cause}", file=progress)
            if args.verbose:
                print(outcome.output, file=progress)
            if args.fail_fast:
                fail_fast_stopped = True
                print("fail-fast: stopping after the first failing smoke.", file=progress)
                break

    if args.junit:
        write_junit(outcomes, Path(args.junit))

    if args.json:
        payload = report_payload(
            outcomes,
            fail_fast_stopped=fail_fast_stopped,
            excluded=excluded,
        )
        print(json.dumps(payload, indent=2))
        return 1 if payload["failed"] else 0

    if excluded:
        print("\nExcluded (external prerequisite):")
        for label, reason in excluded:
            print(f"  {label:<26} {reason}")

    return print_table(outcomes, args.verbose)


if __name__ == "__main__":
    raise SystemExit(main())
