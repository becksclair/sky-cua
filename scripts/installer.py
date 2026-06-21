#!/usr/bin/env python3
"""One-shot sky-cua installer.

Sets up sky-cua for every selected agent. Two modes:

- repo: build the Rust runtime from a checkout (cargo + git), then install.
- bundle: install from a prebuilt release bundle (no build, no cargo) - the
  mode the release package's top-level install.py uses on a clean machine.

The Codex setup materializes the computer-use compat plugin from the bundled
preflight (no marketplace). Each phase reuses the existing install helpers, and
the run ends with health checks.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path

from _plugin_bundle import (
    DIST_PLUGIN_ROOT,
    compat_plugin_targets_payload,
    installed_plugin_root,
    stop_unix_runtime_processes,
    stop_windows_cache_processes,
    update_codex_config,
)
from install_mcp_server import install_local_mcp_server
from install_plugin import install_bundle, run_browser_preflight

REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPTS_ROOT = REPO_ROOT / "scripts"
DEFAULT_TARGET_DIR = Path.home() / ".local" / "share" / "sky-cua"
DEFAULT_BUNDLE_ROOT = DIST_PLUGIN_ROOT

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


def resolve_mode(
    mode_arg: str,
    *,
    repo_root: Path = REPO_ROOT,
    which: Callable[[str], str | None] = shutil.which,
) -> str:
    """Resolve `auto` to `repo` or `bundle`.

    Bundle mode when there is no checkout to build from (no `.git`); a release
    package is exactly that shape. A source checkout without cargo must stay in
    repo mode so the system-deps phase reports the missing Rust toolchain
    instead of silently installing a stale prebuilt bundle.
    """
    if mode_arg != "auto":
        return mode_arg
    if not (repo_root / ".git").exists():
        return "bundle"
    _ = which
    return "repo"


def run_system_deps_phase(*, mode: str, skip: bool, runner: Runner = run_logged) -> PhaseResult:
    name = "system-deps"
    if skip:
        return PhaseResult(name, "skipped", "--skip-system-deps")
    if sys.platform != "linux":
        return PhaseResult(name, "skipped", f"unsupported platform {sys.platform}")

    # cargo/git are only needed to build from a checkout; bundle mode ships
    # prebuilt binaries, so it requires only the runtime system libraries.
    if mode == "repo":
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


def run_build_phase(*, mode: str, skip: bool, runner: Runner = run_logged) -> PhaseResult:
    name = "build"
    if mode == "bundle":
        return PhaseResult(name, "skipped", "bundle mode: using prebuilt bundle")
    if skip:
        return PhaseResult(name, "skipped", "--skip-build")
    result = runner([sys.executable, str(SCRIPTS_ROOT / "build_plugin.py")], cwd=REPO_ROOT)
    if result.returncode != 0:
        return PhaseResult(name, "failed", "scripts/build_plugin.py failed")
    return PhaseResult(name, "ok", "bundle staged under dist/plugin/sky-cua")


def run_codex_phase(
    *,
    enabled: bool,
    bundle_root: Path,
    codex_home: Path,
) -> PhaseResult:
    name = "codex"
    if not enabled:
        return PhaseResult(name, "skipped", "codex not selected")
    if not bundle_root.exists():
        return PhaseResult(name, "failed", f"bundle not found at {bundle_root}")

    # Install the payload into the local Codex cache and materialize the
    # computer-use compat plugin from the bundled preflight - no marketplace,
    # no codex CLI plugin/install. Every step that can raise stays inside the
    # try so any failure becomes a failed PhaseResult and the per-phase summary
    # still renders - matching the old subprocess lane, which surfaced failures
    # as a nonzero exit rather than crashing the installer mid-run.
    destination = installed_plugin_root(codex_home)
    try:
        stop_unix_runtime_processes([destination])
        stop_windows_cache_processes(destination)
        install_bundle(bundle_root, destination, symlink=False)
        run_browser_preflight(destination, codex_home)
        # Linux materializes the compat root (compat-first enablement); other
        # platforms have no compat root, so enable the sky-cua@local channel id
        # directly (Windows-Codex compat is not yet implemented).
        compat = compat_plugin_targets_payload(codex_home, destination)
        update_codex_config(codex_home / "config.toml", compat_enablement=compat)
    except Exception as error:
        return PhaseResult(name, "failed", str(error))

    if compat:
        return PhaseResult(name, "ok", "computer-use@openai-bundled compat plugin enabled")
    return PhaseResult(
        name,
        "ok",
        "sky-cua@local channel enabled (channel-id fallback; no compat root)",
    )


def run_agent_phase(
    host: str,
    *,
    bundle_root: Path,
    target_dir: Path,
    claude_config_dir: Path | None = None,
) -> PhaseResult:
    name = f"agent:{host}"
    try:
        install_local_mcp_server(
            target_dir,
            host,
            restart_runtime=True,
            bundle_root=bundle_root,
            claude_config_dir=claude_config_dir if host == "claude-code" else None,
        )
    except Exception as error:
        return PhaseResult(name, "failed", str(error))
    return PhaseResult(name, "ok", f"MCP server registered for {host}")


def run_kwin_phase(*, enabled: bool, target_dir: Path) -> PhaseResult:
    name = "kwin-effect"
    if not enabled:
        return PhaseResult(name, "skipped", "pass --kwin-effect to install")
    from _kwin_effect import deploy_kwin_effect, kwin_effect_deploy_failed

    try:
        outcome = deploy_kwin_effect(build_dir=target_dir / "kwin-effect-build")
    except Exception as error:
        return PhaseResult(name, "failed", str(error))
    if kwin_effect_deploy_failed(outcome):
        return PhaseResult(
            name,
            "failed",
            f"{outcome.effect_id} did not converge; "
            f"restored {outcome.rollback_effect_id or 'no previous effect'}",
        )
    detail = "installed"
    if outcome.session_restart_required:
        detail = "installed; activates after the next Plasma session restart"
    return PhaseResult(name, "ok", detail)


def run_health_phase(
    *,
    agents: list[str],
    target_dir: Path,
    claude_dir: Path | None = None,
    runner: Runner = run_logged,
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

    if "claude-code" in agents:
        # The MCP-list probe needs the claude CLI; the permission policy is a
        # file the installer writes regardless of CLI presence, so attest it
        # whenever claude-code is a target.
        if shutil.which("claude"):
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
        settings_path = (claude_dir or (Path.home() / ".claude")) / "settings.json"
        status, detail = claude_code_permissions_status(settings_path)
        results.append(PhaseResult("health:claude-code-permissions", status, detail))

    return results


def claude_code_permissions_status(settings_path: Path) -> tuple[str, str]:
    """Report whether ~/.claude/settings.json denies built-in computer-use and
    auto-approves sky-cua. Returns a (status, detail) pair for the health summary."""
    if not settings_path.exists():
        return ("skipped", "settings.json not found")
    try:
        settings = json.loads(settings_path.read_text(encoding="utf-8") or "{}")
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        return ("failed", f"settings.json unreadable: {error}")
    permissions = settings.get("permissions", {}) if isinstance(settings, dict) else {}
    if not isinstance(permissions, dict):
        permissions = {}
    deny = permissions.get("deny", [])
    allow = permissions.get("allow", [])
    # Attest the server-scope rule each tuple leads with, not a loose prefix, so
    # a partial hand-edit cannot read as a healthy install.
    denied = isinstance(deny, list) and "mcp__computer-use" in deny
    approved = isinstance(allow, list) and "mcp__sky-cua" in allow
    if denied and approved:
        return ("ok", "built-in computer-use denied, sky-cua auto-approved")
    missing = []
    if not denied:
        missing.append("computer-use deny rule")
    if not approved:
        missing.append("sky-cua allow rule")
    return ("failed", "missing " + ", ".join(missing))


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
        "--claude-config-dir",
        type=Path,
        default=None,
        help=(
            "Claude Code config directory (default: ~/.claude). Used for the "
            "claude-code agent registration and its permission health check."
        ),
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=Path.home() / ".codex",
        help="Codex home directory (default: ~/.codex).",
    )
    parser.add_argument(
        "--mode",
        choices=("auto", "repo", "bundle"),
        default="auto",
        help=(
            "Install mode. 'repo' builds from a checkout (cargo+git); 'bundle' "
            "installs a prebuilt bundle (no build); 'auto' picks bundle when "
            "there is no .git checkout (default: auto)."
        ),
    )
    parser.add_argument(
        "--bundle-root",
        type=Path,
        default=DEFAULT_BUNDLE_ROOT,
        help=f"Prebuilt bundle to install from (default: {DEFAULT_BUNDLE_ROOT}).",
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
        "--skip-health",
        action="store_true",
        help=(
            "Skip installer health checks. Intended for headless package "
            "validation that performs its own degraded assertions."
        ),
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the resolved plan without changing anything.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    mode = resolve_mode(args.mode)
    bundle_root = args.bundle_root.expanduser().resolve()

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

    print(f"Mode: {mode}" + (f" (bundle: {bundle_root})" if mode == "bundle" else ""))
    print(f"Agents: {', '.join(agents) if agents else '(none)'}")
    if args.dry_run:
        print("Dry run; phases that would execute:")
        phases = ["system-deps"]
        if mode == "repo":
            phases.append("build")
        if "codex" in agents:
            phases.append("codex")
        phases.extend(f"agent:{host}" for host in agents if host != "codex")
        if not args.skip_health:
            phases.append("health")
        for phase in phases:
            print(f"  {phase}")
        return 0

    results: list[PhaseResult] = []

    results.append(run_system_deps_phase(mode=mode, skip=args.skip_system_deps))
    if results[-1].failed:
        print_summary(results)
        return 1

    results.append(run_build_phase(mode=mode, skip=args.skip_build))
    if results[-1].failed:
        print_summary(results)
        return 1

    results.append(
        run_codex_phase(
            enabled="codex" in agents,
            bundle_root=bundle_root,
            codex_home=args.codex_home.expanduser(),
        )
    )

    # install_claude_code writes settings.json to the resolved config dir, so the
    # health check must read the same resolved path or it would attest the wrong file.
    claude_config_dir = (
        args.claude_config_dir.expanduser().resolve()
        if args.claude_config_dir is not None
        else None
    )

    for host in agents:
        if host == "codex":
            continue
        results.append(
            run_agent_phase(
                host,
                bundle_root=bundle_root,
                target_dir=args.target_dir.expanduser(),
                claude_config_dir=claude_config_dir,
            )
        )

    if mode == "bundle" and args.kwin_effect:
        results.append(
            PhaseResult("kwin-effect", "skipped", "requires a source checkout (repo mode)")
        )
    else:
        results.append(
            run_kwin_phase(enabled=args.kwin_effect, target_dir=args.target_dir.expanduser())
        )

    if args.skip_health:
        results.append(PhaseResult("health", "skipped", "--skip-health"))
    else:
        results.extend(
            run_health_phase(
                agents=agents,
                target_dir=args.target_dir.expanduser(),
                claude_dir=claude_config_dir,
            )
        )

    print_summary(results)
    return 1 if any(result.failed for result in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
