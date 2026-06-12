#!/usr/bin/env python3
"""One-shot sky-cua installer.

Runs the full setup for a fresh clone: system dependencies, the Rust runtime
build, the Codex Heliasar marketplace install (with the computer-use compat
plugin root), and MCP server registration plus skills for the other supported
agents. Each phase delegates to the existing deploy scripts instead of
duplicating their logic, and the run ends with health checks.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPTS_ROOT = REPO_ROOT / "scripts"
DEFAULT_TARGET_DIR = Path.home() / ".local" / "share" / "sky-cua"

KNOWN_AGENTS = ("codex", "claude-code", "claude-desktop", "opencode", "pi", "openclaw")
NON_CODEX_HOSTS = tuple(agent for agent in KNOWN_AGENTS if agent != "codex")

REQUIRED_COMMANDS = ("cargo", "git")

# Runtime system packages per package manager. Build toolchains (Rust) are
# checked as commands instead because they are usually installed via rustup.
SYSTEM_PACKAGES: Mapping[str, tuple[str, ...]] = {
    "pacman": (
        "at-spi2-core",
        "dbus",
        "gst-plugins-base",
        "gst-plugins-good",
        "gstreamer",
        "libxkbcommon",
        "wayland",
        "xdg-desktop-portal",
        "ydotool",
    ),
    "apt": (
        "at-spi2-core",
        "dbus",
        "gstreamer1.0-plugins-base",
        "gstreamer1.0-plugins-good",
        "libgstreamer1.0-0",
        "libwayland-client0",
        "libxkbcommon0",
        "xdg-desktop-portal",
        "ydotool",
    ),
}

INSTALL_COMMANDS: Mapping[str, tuple[str, ...]] = {
    "pacman": ("sudo", "pacman", "-S", "--needed", "--noconfirm"),
    "apt": ("sudo", "apt-get", "install", "-y"),
}

Runner = Callable[..., "subprocess.CompletedProcess[str]"]


@dataclass
class PhaseResult:
    name: str
    status: str  # "ok" | "skipped" | "failed"
    detail: str = ""

    @property
    def failed(self) -> bool:
        return self.status == "failed"


def run_logged(
    command: list[str], *, cwd: Path | None = None, check: bool = False
) -> subprocess.CompletedProcess[str]:
    print(f"+ {' '.join(command)}")
    return subprocess.run(command, cwd=cwd, check=check, text=True)


def detect_package_manager(which: Callable[[str], str | None] = shutil.which) -> str | None:
    for manager in ("pacman", "apt"):
        probe = "apt-get" if manager == "apt" else manager
        if which(probe):
            return manager
    return None


def missing_packages(
    manager: str,
    installed: Callable[[str], bool],
    packages: Mapping[str, tuple[str, ...]] = SYSTEM_PACKAGES,
) -> list[str]:
    return [package for package in packages.get(manager, ()) if not installed(package)]


def pacman_package_installed(package: str) -> bool:
    return (
        subprocess.run(
            ["pacman", "-Qq", package], capture_output=True, text=True, check=False
        ).returncode
        == 0
    )


def apt_package_installed(package: str) -> bool:
    return (
        subprocess.run(
            ["dpkg", "-s", package], capture_output=True, text=True, check=False
        ).returncode
        == 0
    )


def detect_agents(
    home: Path | None = None, which: Callable[[str], str | None] = shutil.which
) -> dict[str, bool]:
    base = home if home is not None else Path.home()
    return {
        "codex": which("codex") is not None,
        "claude-code": which("claude") is not None or (base / ".claude").is_dir(),
        "claude-desktop": (base / ".config" / "Claude").is_dir()
        or (base / "Library" / "Application Support" / "Claude").is_dir(),
        "opencode": which("opencode") is not None,
        "pi": (base / ".pi" / "agent").is_dir(),
        "openclaw": which("openclaw") is not None or (base / ".openclaw").is_dir(),
    }


def select_agents(requested: str | None, detected: Mapping[str, bool]) -> list[str]:
    """Resolve the agent list: an explicit request wins, otherwise detection."""
    if requested is not None:
        agents: list[str] = []
        for raw in requested.split(","):
            name = raw.strip()
            if not name:
                continue
            if name not in KNOWN_AGENTS:
                known = ", ".join(KNOWN_AGENTS)
                raise ValueError(f"unknown agent {name!r} (known agents: {known})")
            if name not in agents:
                agents.append(name)
        return agents
    return [agent for agent in KNOWN_AGENTS if detected.get(agent, False)]


def missing_required_commands(
    which: Callable[[str], str | None] = shutil.which,
    commands: tuple[str, ...] = REQUIRED_COMMANDS,
) -> list[str]:
    return [command for command in commands if which(command) is None]


def run_system_deps_phase(*, skip: bool, runner: Runner = run_logged) -> PhaseResult:
    name = "system-deps"
    if skip:
        return PhaseResult(name, "skipped", "--skip-system-deps")
    if sys.platform != "linux":
        return PhaseResult(name, "skipped", f"unsupported platform {sys.platform}")

    missing_commands = missing_required_commands()
    if missing_commands:
        return PhaseResult(
            name,
            "failed",
            "missing required commands: "
            + ", ".join(missing_commands)
            + " (install Rust via rustup and git via your package manager)",
        )

    manager = detect_package_manager()
    if manager is None:
        return PhaseResult(
            name,
            "skipped",
            "no supported package manager (pacman/apt); verify runtime libraries manually",
        )

    installed = pacman_package_installed if manager == "pacman" else apt_package_installed
    missing = missing_packages(manager, installed)
    if not missing:
        return PhaseResult(name, "ok", f"{manager}: all runtime packages present")

    command = [*INSTALL_COMMANDS[manager], *missing]
    result = runner(command)
    if result.returncode != 0:
        return PhaseResult(name, "failed", f"package install failed: {' '.join(command)}")
    return PhaseResult(name, "ok", f"{manager}: installed {', '.join(missing)}")


def run_build_phase(*, skip: bool, runner: Runner = run_logged) -> PhaseResult:
    name = "build"
    if skip:
        return PhaseResult(name, "skipped", "--skip-build")
    result = runner([sys.executable, str(SCRIPTS_ROOT / "build_plugin.py")], cwd=REPO_ROOT)
    if result.returncode != 0:
        return PhaseResult(name, "failed", "scripts/build_plugin.py failed")
    return PhaseResult(name, "ok", "bundle staged under dist/plugin/sky-cua")


def run_codex_phase(
    *,
    enabled: bool,
    codex_home: Path,
    marketplace_root: Path | None,
    marketplace_source: str | None,
    runner: Runner = run_logged,
) -> PhaseResult:
    name = "codex"
    if not enabled:
        return PhaseResult(name, "skipped", "codex not selected")
    if shutil.which("codex") is None:
        return PhaseResult(name, "failed", "codex CLI not found on PATH")

    command = [
        sys.executable,
        str(SCRIPTS_ROOT / "setup_heliasar_marketplace.py"),
        "--codex-home",
        str(codex_home),
    ]
    if marketplace_root is not None:
        command.extend(["--marketplace-root", str(marketplace_root)])
    if marketplace_source is not None:
        command.extend(["--marketplace-source", marketplace_source])
    result = runner(command, cwd=REPO_ROOT)
    if result.returncode != 0:
        return PhaseResult(name, "failed", "setup_heliasar_marketplace.py failed")
    return PhaseResult(name, "ok", "marketplace installed; compat plugin enabled")


def run_agent_phase(host: str, *, target_dir: Path, runner: Runner = run_logged) -> PhaseResult:
    name = f"agent:{host}"
    command = [
        sys.executable,
        str(SCRIPTS_ROOT / "install_mcp_server.py"),
        "--host",
        host,
        "--target-dir",
        str(target_dir),
        "--restart-runtime",
    ]
    result = runner(command, cwd=REPO_ROOT)
    if result.returncode != 0:
        return PhaseResult(name, "failed", "install_mcp_server.py failed")
    return PhaseResult(name, "ok", f"MCP server registered for {host}")


def run_kwin_phase(*, enabled: bool, target_dir: Path) -> PhaseResult:
    name = "kwin-effect"
    if not enabled:
        return PhaseResult(name, "skipped", "pass --kwin-effect to install")
    from _kwin_effect import deploy_kwin_effect

    try:
        outcome = deploy_kwin_effect(build_dir=target_dir / "kwin-effect-build")
    except Exception as error:
        return PhaseResult(name, "failed", str(error))
    detail = "installed"
    if outcome.session_restart_required:
        detail = "installed; activates after the next Plasma session restart"
    return PhaseResult(name, "ok", detail)


def run_health_phase(
    *, agents: list[str], target_dir: Path, runner: Runner = run_logged
) -> list[PhaseResult]:
    results: list[PhaseResult] = []

    client = target_dir / "bin" / "sky-cua-client"
    if not client.exists():
        client = REPO_ROOT / "dist" / "plugin" / "sky-cua" / "bin" / "sky-cua-client"
    if client.exists():
        doctor = runner([str(client), "doctor"])
        results.append(PhaseResult("health:doctor", "ok" if doctor.returncode == 0 else "failed"))
    else:
        results.append(PhaseResult("health:doctor", "skipped", "no installed sky-cua-client found"))

    if "codex" in agents and shutil.which("codex"):
        listing = subprocess.run(
            ["codex", "mcp", "list"], capture_output=True, text=True, check=False
        )
        ok = listing.returncode == 0 and "computer-use" in listing.stdout
        results.append(
            PhaseResult(
                "health:codex",
                "ok" if ok else "failed",
                "computer-use MCP server listed"
                if ok
                else "computer-use missing from codex mcp list",
            )
        )

    if "claude-code" in agents and shutil.which("claude"):
        listing = subprocess.run(
            ["claude", "mcp", "list"], capture_output=True, text=True, check=False
        )
        ok = listing.returncode == 0 and "sky-cua" in listing.stdout
        results.append(
            PhaseResult(
                "health:claude-code",
                "ok" if ok else "failed",
                "sky-cua registered" if ok else "sky-cua missing from claude mcp list",
            )
        )

    return results


def print_summary(results: list[PhaseResult]) -> None:
    print("\nInstall summary:")
    width = max(len(result.name) for result in results)
    for result in results:
        line = f"  {result.name.ljust(width)}  {result.status}"
        if result.detail:
            line += f"  ({result.detail})"
        print(line)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="One-shot sky-cua installer: system deps, build, Codex, agents, health."
    )
    parser.add_argument(
        "--agents",
        default=None,
        help=(
            "Comma-separated agents to set up (default: auto-detect). Known: "
            + ", ".join(KNOWN_AGENTS)
        ),
    )
    parser.add_argument(
        "--target-dir",
        type=Path,
        default=DEFAULT_TARGET_DIR,
        help=f"Install directory for non-Codex agents (default: {DEFAULT_TARGET_DIR}).",
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=Path.home() / ".codex",
        help="Codex home directory (default: ~/.codex).",
    )
    parser.add_argument(
        "--marketplace-root",
        type=Path,
        default=None,
        help="Local Heliasar marketplace checkout (passed through to the Codex setup).",
    )
    parser.add_argument(
        "--marketplace-source",
        default=None,
        help="Codex marketplace source (passed through to the Codex setup).",
    )
    parser.add_argument(
        "--kwin-effect",
        action="store_true",
        help="Also build and install the KWin agent-cursor effect (Linux/KDE only).",
    )
    parser.add_argument(
        "--skip-system-deps", action="store_true", help="Skip the system dependency phase."
    )
    parser.add_argument(
        "--skip-build", action="store_true", help="Skip the Rust build/bundle phase."
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the resolved plan without changing anything.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    detected = detect_agents()
    try:
        agents = select_agents(args.agents, detected)
    except ValueError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    if not agents:
        print(
            "No supported agents detected and none requested via --agents; "
            "nothing to set up beyond the build.",
        )

    print(f"Repo root: {REPO_ROOT}")
    print(f"Agents: {', '.join(agents) if agents else '(none)'}")
    if args.dry_run:
        print("Dry run; phases that would execute:")
        phases = ["system-deps", "build"]
        if "codex" in agents:
            phases.append("codex")
        phases.extend(f"agent:{host}" for host in agents if host != "codex")
        phases.append("health")
        for phase in phases:
            print(f"  {phase}")
        return 0

    results: list[PhaseResult] = []

    results.append(run_system_deps_phase(skip=args.skip_system_deps))
    if results[-1].failed:
        print_summary(results)
        return 1

    results.append(run_build_phase(skip=args.skip_build))
    if results[-1].failed:
        print_summary(results)
        return 1

    results.append(
        run_codex_phase(
            enabled="codex" in agents,
            codex_home=args.codex_home.expanduser(),
            marketplace_root=args.marketplace_root,
            marketplace_source=args.marketplace_source,
        )
    )

    for host in agents:
        if host == "codex":
            continue
        results.append(run_agent_phase(host, target_dir=args.target_dir.expanduser()))

    results.append(
        run_kwin_phase(enabled=args.kwin_effect, target_dir=args.target_dir.expanduser())
    )

    results.extend(run_health_phase(agents=agents, target_dir=args.target_dir.expanduser()))

    print_summary(results)
    return 1 if any(result.failed for result in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
