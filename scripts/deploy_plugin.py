#!/usr/bin/env python3
"""Deploy the sky-cua plugin locally (fast dev loop).

Install the freshly built bundle into the local Codex payload (`sky-cua@local`),
retarget the computer-use compat plugin at it, and refresh the installed
MCP-server runtime. No git, no Codex `plugin/install`. This updates *what runs*
on this machine, immediately.

To produce a distributable release, use `scripts/package.py`; to install one on
a clean machine, use `install.py`.
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

from _companion import (
    build_and_stage_companion,
    companion_setup_status,
    print_companion_build_outcome,
    print_companion_setup_status,
)
from _install_shared import DEFAULT_LOCAL_INSTALL_DIR, MCP_HOST_CHOICES
from _kwin_effect import deploy_kwin_effect, print_kwin_effect_deploy_outcome
from _plugin_bundle import (
    DEFAULT_CODEX_HOME,
    DIST_PLUGIN_ROOT,
    RETIRED_PLUGIN_IDS,
    build_bundle,
    compat_plugin_targets_payload,
    ensure_bundle_structure,
    installed_plugin_root,
    remove_path,
    stop_unix_runtime_processes,
    stop_windows_cache_processes,
    update_codex_config,
)
from deploy_freshness import write_build_stamp
from install_mcp_server import install_local_mcp_server, refresh_accessibility_bus
from install_plugin import install_bundle, run_browser_preflight


def drop_retired_channel_caches(
    codex_home: Path,
    *,
    stale_roots: list[Path] | None = None,
    stop_unix: bool = True,
) -> None:
    """Best-effort, idempotent removal of stale retired-channel cache payloads.

    Earlier installs cached the retired ``debug`` and ``Heliasar`` channel
    payloads under ``cache/<marketplace>/sky-cua``. Config neutralization (so
    Codex stops launching them) is owned by ``update_codex_config``; this drops
    the orphaned cache trees, stopping any process still running from them, so
    the dev loop does not accumulate dead payloads. Only the sky-cua payload is
    removed, never the marketplace dir, which may hold sibling plugins (e.g.
    ``cache/Heliasar/clawpatch``). A failed removal is cosmetic, so it only warns.
    """
    stale_roots = retired_channel_cache_roots(codex_home) if stale_roots is None else stale_roots
    if stop_unix and sys.platform != "win32":
        stop_unix_runtime_processes(stale_roots)
    for stale_root in stale_roots:
        stop_windows_cache_processes(stale_root)
        try:
            remove_path(stale_root)
        except OSError as exc:
            print(
                f"warning: could not remove stale plugin cache {stale_root}: {exc}",
                file=sys.stderr,
            )


def retired_channel_cache_roots(codex_home: Path) -> list[Path]:
    return [
        codex_home / "plugins" / "cache" / plugin_id.split("@", 1)[1] / "sky-cua"
        for plugin_id in RETIRED_PLUGIN_IDS
        if (codex_home / "plugins" / "cache" / plugin_id.split("@", 1)[1] / "sky-cua").exists()
    ]


def fast_deploy(args: argparse.Namespace) -> int:
    if not args.no_build:
        # Build + stage the Android companion APK before bundling so build_bundle
        # picks up a fresh artifact. Automatic and toolchain-gated: it rebuilds
        # only when the companion sources changed (or --force-companion) and skips
        # gracefully on a host without JDK 21 + the Android SDK. --no-companion
        # opts out entirely. --no-build skips this too (the bundle is reused).
        if not args.no_companion:
            outcome = build_and_stage_companion(force=args.force_companion)
            print_companion_build_outcome(outcome)
        build_bundle()

    bundle_root = DIST_PLUGIN_ROOT.resolve()
    ensure_bundle_structure(bundle_root)

    destination = installed_plugin_root(args.codex_home)
    stale_roots = retired_channel_cache_roots(args.codex_home)
    if sys.platform != "win32":
        refresh_accessibility_bus()
        stop_unix_runtime_processes([*stale_roots, destination])
    drop_retired_channel_caches(args.codex_home, stale_roots=stale_roots, stop_unix=False)
    stop_windows_cache_processes(destination)
    install_bundle(bundle_root, destination, args.symlink)
    run_browser_preflight(destination, args.codex_home)

    config_path = args.codex_home / "config.toml"
    # Compat-first: the preflight above retargets the computer-use compat plugin
    # at this local payload; the channel id stays disabled. When the bundle ships
    # no openai-bundled resources (no compat root), update_codex_config falls back
    # to enabling the local channel id (sky-cua@local) directly.
    update_codex_config(
        config_path,
        compat_enablement=compat_plugin_targets_payload(args.codex_home, destination),
    )

    # Fold in the installed MCP-server refresh so a single command also updates
    # the runtime used by Claude Code and other non-Codex hosts.
    local_install_dir = args.local_install_dir.expanduser().resolve()
    client_path, mcp_config_path = install_local_mcp_server(
        local_install_dir,
        args.local_install_host,
        restart_runtime=True,
        bundle_root=bundle_root,
        refresh_accessibility=False,
    )

    # Stamp the deployed client with the runtime-source fingerprint it was built
    # from, so live tests can detect when the deployed runtime has gone stale
    # relative to the working tree and refuse to run against old binaries.
    stamp_path = write_build_stamp(client_path, deployed_at_ms=int(time.time() * 1000))
    print(f"deploy_stamp={stamp_path}")

    if args.kwin_effect:
        outcome = deploy_kwin_effect(build_dir=destination.parent / "kwin-effect-build")
        print_kwin_effect_deploy_outcome(outcome)

    print(f"installed_path={destination}")
    print(f"config_path={config_path}")
    print(f"local_install_path={client_path}")
    print(f"local_install_config={mcp_config_path}")

    # Device-setup handoff: the deploy staged + bundled the companion but does
    # not install it onto a phone or enable its services (a runtime step). Emit a
    # status the calling agent acts on — which devices are connected, and to ask
    # the user which to set up via phone_connect + phone_install_companion.
    if not args.no_companion:
        print_companion_setup_status(companion_setup_status())
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Deploy the sky-cua plugin locally: a fast install that updates what "
            "runs immediately (sky-cua@local). For a distributable release use "
            "scripts/package.py."
        )
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=DEFAULT_CODEX_HOME,
        help="Codex home directory to install into (default: ~/.codex).",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Install the existing dist/plugin/sky-cua bundle without rebuilding.",
    )
    parser.add_argument(
        "--symlink",
        action="store_true",
        help="Symlink the bundle into the local payload instead of copying it.",
    )
    parser.add_argument(
        "--kwin-effect",
        action="store_true",
        help=(
            "Also build, install (sudo cmake --install), and reload the sky-cua "
            "KWin agent-cursor effect (Linux/KDE only)."
        ),
    )
    parser.add_argument(
        "--no-companion",
        action="store_true",
        help=(
            "Skip the Android phone-companion build/stage lane. By default the "
            "deploy rebuilds the companion APK when its sources changed and the "
            "Android toolchain (JDK 21 + SDK) is present, and bundles it."
        ),
    )
    parser.add_argument(
        "--force-companion",
        action="store_true",
        help=(
            "Rebuild and stage the Android phone-companion APK even when its "
            "sources appear unchanged (requires the Android toolchain)."
        ),
    )
    parser.add_argument(
        "--local-install-dir",
        type=Path,
        default=DEFAULT_LOCAL_INSTALL_DIR,
        help=f"Installed MCP-server runtime to refresh (default: {DEFAULT_LOCAL_INSTALL_DIR}).",
    )
    parser.add_argument(
        "--local-install-host",
        default="claude-code",
        choices=MCP_HOST_CHOICES,
        help="Host config format for the installed MCP-server runtime (default: claude-code).",
    )
    args = parser.parse_args(argv)
    return fast_deploy(args)


if __name__ == "__main__":
    raise SystemExit(main())
