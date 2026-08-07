#!/usr/bin/env python3
"""Deploy the sky-cua plugin locally (fast dev loop).

Install the freshly built bundle into the local Codex payload (`sky-cua@local`),
retarget the computer-use compat plugin at it, and refresh the installed
MCP-server runtime. No git, no Codex `plugin/install`. This updates *what runs*
on this machine, immediately.

To produce or install the standalone distribution, use `install.py build` or
`install.py install` from the repository root.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
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
from _install_shared import enabled_skill_names as durable_enabled_skill_names
from _kwin_effect import (
    deploy_kwin_effect,
    installed_effect_ids,
    kwin_effect_deploy_failed,
    kwin_effect_up_to_date,
    print_kwin_effect_deploy_outcome,
)
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

CODEX_BROWSER_CLIENT_RELATIVE_PATH = Path(
    "plugins/openai-bundled/plugins/browser-use/scripts/browser-client.mjs"
)
CODEX_BROWSER_PLUGIN_MANIFEST_RELATIVE_PATH = Path(
    "plugins/openai-bundled/plugins/browser-use/.codex-plugin/plugin.json"
)
CODEX_RELEASE_MODULE_NAME = "sky-cua-release.cjs"
CODEX_RUNTIME_OVERRIDE_ENV_NAMES = (
    "CODEX_BROWSER_USE_MODULE_SEARCH_ROOT",
    "CODEX_BROWSER_USE_NODE_PATH",
    "CODEX_CUA_NODE_LOCK_ROOT",
    "CODEX_CUA_NODE_ROOT",
    "CODEX_ELECTRON_RESOURCES_PATH",
    "CODEX_NODE_REPL_LEGACY_FALLBACK",
    "CODEX_NODE_REPL_PATH",
    "CUA_NODE_BROWSER_CLIENT_PATH",
    "NODE_REPL_NODE_MODULE_DIRS",
    "NODE_REPL_NODE_PATH",
    "NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S",
    "PLAYWRIGHT_BROWSERS_PATH",
    "SKY_CUA_DOCUMENTATION_ROOT",
    "SKY_CUA_DOCUMENTATION_ROUTING_INVENTORY",
    "SKY_CUA_DOCUMENTATION_ROUTING_INVENTORY_SHA256",
    "SKY_CUA_RELEASE_CANONICAL_BROWSER_SHA256",
    "SKY_CUA_RELEASE_ID",
    "SKY_CUA_RELEASE_MANIFEST_SHA256",
    "SKY_CUA_RELEASE_PRODUCER_COMMIT",
    "SKY_CUA_RELEASE_ROOT",
)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _is_sha256(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def _codex_resources_root(explicit: Path | None) -> Path | None:
    if explicit is not None:
        candidates: list[Path | None] = [explicit]
    else:
        candidates = [
            Path(value) if (value := os.environ.get("CODEX_ELECTRON_RESOURCES_PATH")) else None,
            Path("/opt/chatgpt-desktop/resources"),
            Path("/opt/codex-desktop/resources"),
        ]
    for candidate in candidates:
        if candidate is None:
            continue
        resolved = candidate.expanduser().resolve()
        runner = resolved.parent / "ChatGPT"
        if (
            (resolved / "browser-use-cache-sync.cjs").is_file()
            and (resolved / CODEX_RELEASE_MODULE_NAME).is_file()
            and (resolved / CODEX_BROWSER_PLUGIN_MANIFEST_RELATIVE_PATH).is_file()
            and (resolved / CODEX_BROWSER_CLIENT_RELATIVE_PATH).is_file()
            and runner.is_file()
            and os.access(runner, os.X_OK)
        ):
            return resolved
    return None


def _packaged_codex_runtime_env() -> dict[str, str]:
    env = os.environ.copy()
    for name in CODEX_RUNTIME_OVERRIDE_ENV_NAMES:
        env.pop(name, None)
    env["ELECTRON_RUN_AS_NODE"] = "1"
    return env


def sync_and_verify_codex_browser_client(
    codex_home: Path,
    *,
    resources_root: Path | None = None,
) -> Path | None:
    """Materialize Codex's packaged Browser plugin and verify its trust tuple.

    Codex Desktop owns the cache publisher and consumes sky-cua's verified release
    projection. sky-cua invokes that publisher during local deployment and checks
    that the release, packaged projection, cache-latest bytes, and resolved runtime
    identities agree; it never rewrites Codex's resolver or Browser projection.
    """
    if sys.platform != "linux":
        return None

    resolved_resources = _codex_resources_root(resources_root)
    if resolved_resources is None:
        raise RuntimeError(
            "Packaged Codex Desktop sky-cua consumer resources were not found; "
            "pass --codex-resources-root"
        )

    command_env = _packaged_codex_runtime_env()
    node = resolved_resources.parent / "ChatGPT"
    sync_script = resolved_resources / "browser-use-cache-sync.cjs"
    resolved_env = subprocess.run(
        [
            str(node),
            str(sync_script),
            "resolve-env",
            f"--resources-root={resolved_resources}",
        ],
        check=True,
        capture_output=True,
        env=command_env,
        text=True,
    )
    try:
        runtime_env = json.loads(resolved_env.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError("Codex sky-cua resolver returned invalid JSON") from exc
    expected_hash = runtime_env.get("SKY_CUA_RELEASE_CANONICAL_BROWSER_SHA256")
    trusted_hash = runtime_env.get("NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S")
    release_root_value = runtime_env.get("SKY_CUA_RELEASE_ROOT")
    release_id = runtime_env.get("SKY_CUA_RELEASE_ID")
    manifest_sha256 = runtime_env.get("SKY_CUA_RELEASE_MANIFEST_SHA256")
    if (
        not _is_sha256(expected_hash)
        or trusted_hash != expected_hash
        or not _is_sha256(release_id)
        or not _is_sha256(manifest_sha256)
        or not isinstance(release_root_value, str)
        or not Path(release_root_value).is_absolute()
        or Path(release_root_value).resolve().name != release_id
    ):
        raise RuntimeError("Codex sky-cua resolver omitted a consistent verified release identity")

    packaged_plugin_manifest_path = resolved_resources / CODEX_BROWSER_PLUGIN_MANIFEST_RELATIVE_PATH
    packaged_client = resolved_resources / CODEX_BROWSER_CLIENT_RELATIVE_PATH
    if _sha256(packaged_client) != expected_hash:
        raise RuntimeError(
            "Codex Browser packaged bytes do not match the verified sky-cua release trust hash"
        )

    sync = subprocess.run(
        [
            str(node),
            str(sync_script),
            "sync-cache",
            f"--resources-root={resolved_resources}",
            f"--codex-home={codex_home.expanduser().resolve()}",
            "--require-resources",
            "--json",
        ],
        check=True,
        capture_output=True,
        env=command_env,
        text=True,
    )
    try:
        sync_result = json.loads(sync.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError("Codex Browser cache sync returned invalid JSON") from exc

    latest_link_value = sync_result.get("latestLink")
    if not isinstance(latest_link_value, str):
        raise RuntimeError("Codex Browser cache sync omitted latestLink")
    latest_root = Path(latest_link_value)
    cached_client = latest_root / "scripts/browser-client.mjs"
    cached_plugin_manifest_path = latest_root / ".codex-plugin/plugin.json"

    try:
        packaged_plugin_manifest = json.loads(
            packaged_plugin_manifest_path.read_text(encoding="utf-8")
        )
        cached_plugin_manifest = json.loads(cached_plugin_manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError("Codex Browser packaged or cached manifest is invalid") from exc

    expected_version = packaged_plugin_manifest.get("version")
    if (
        not isinstance(expected_version, str)
        or not expected_version
        or cached_plugin_manifest.get("version") != expected_version
        or sync_result.get("version") != expected_version
    ):
        raise RuntimeError("Codex Browser cache version does not match packaged resources")
    if _sha256(cached_client) != expected_hash:
        raise RuntimeError(
            "Codex Browser cache-latest bytes do not match the verified sky-cua release trust hash"
        )

    print(f"codex_browser_version={expected_version}")
    print(f"codex_browser_client={cached_client.resolve()}")
    print(f"codex_browser_trusted_sha256={expected_hash}")
    print(f"codex_sky_cua_release_id={release_id}")
    print(f"codex_sky_cua_manifest_sha256={manifest_sha256}")
    return cached_client.resolve()


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
    # Verify the external Codex consumer contract before building, stopping
    # runtimes, or replacing config. A pinned-consumer mismatch is expected to
    # fail closed, but it must not leave a partially applied local deployment.
    sync_and_verify_codex_browser_client(
        args.codex_home,
        resources_root=getattr(args, "codex_resources_root", None),
    )

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
        # The AT-SPI registry reset (pkill at-spi2-registryd + restart
        # at-spi-dbus-bus) is a heavy hammer: it wipes every running app's
        # accessibility registration, not just sky-cua's. Chromium apps
        # re-register lazily on the next query, but GTK apps register eagerly at
        # startup and go dark until relaunched — so a deploy silently breaks
        # semantic targeting of any GTK app running across it (a live-smoke flake
        # generator). sky-cua already self-heals a wedged AT-SPI connection on
        # reconnect (reset_accessibility_connection + retry), so the reset is
        # opt-in rather than default. Pass --refresh-accessibility when the
        # registry is genuinely wedged.
        if getattr(args, "refresh_accessibility", False):
            refresh_accessibility_bus()
            print(
                "note: reset the user AT-SPI registry; GTK apps running across "
                "this deploy must be relaunched to re-register their trees.",
            )
        stop_unix_runtime_processes([*stale_roots, destination])
    drop_retired_channel_caches(args.codex_home, stale_roots=stale_roots, stop_unix=False)
    stop_windows_cache_processes(destination)
    install_bundle(bundle_root, destination, args.symlink)
    run_browser_preflight(destination, args.codex_home)

    config_path = args.codex_home / "config.toml"
    enabled_skills = durable_enabled_skill_names()
    # Compat-first: the preflight above retargets the computer-use compat plugin
    # at this local payload; the channel id stays disabled. When the bundle ships
    # no openai-bundled resources (no compat root), update_codex_config falls back
    # to enabling the local channel id (sky-cua@local) directly.
    update_codex_config(
        config_path,
        compat_enablement=compat_plugin_targets_payload(args.codex_home, destination),
        enabled_skill_names=enabled_skills,
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
        browser_eval=getattr(args, "browser_eval", None),
        model_supports_images=getattr(args, "model_supports_images", None),
        reap_all_runtime=True,
    )

    # Stamp the deployed client with the runtime-source fingerprint it was built
    # from, so live tests can detect when the deployed runtime has gone stale
    # relative to the working tree and refuse to run against old binaries.
    stamp_path = write_build_stamp(client_path, deployed_at_ms=int(time.time() * 1000))
    print(f"deploy_stamp={stamp_path}")

    # Keep the KWin agent-cursor effect ABI-fresh and actually loaded. Run when
    # forced with --kwin-effect, or automatically when the effect is already
    # installed (the operator opted in before, so a deploy must not leave it
    # broken after a KWin update silently unloads a stale-ABI build). The
    # up-to-date fast path avoids a gratuitous sudo prompt when the loaded
    # effect already matches the current source.
    want_effect = not args.no_kwin_effect and (args.kwin_effect or bool(installed_effect_ids()))
    if want_effect:
        if not args.kwin_effect and kwin_effect_up_to_date():
            print("kwin-effect: already loaded and up to date; skipping rebuild")
        else:
            outcome = deploy_kwin_effect(build_dir=destination.parent / "kwin-effect-build")
            print_kwin_effect_deploy_outcome(outcome)
            if kwin_effect_deploy_failed(outcome):
                print(
                    f"KWin effect {outcome.effect_id} did not load; "
                    f"restored {outcome.rollback_effect_id or 'no previous effect'}. "
                    "The agent cursor will not hide the system cursor until this is fixed.",
                    file=sys.stderr,
                )
                return 1

    print(f"installed_path={destination}")
    print(f"config_path={config_path}")
    print(f"local_install_path={client_path}")
    print(f"local_install_config={mcp_config_path}")

    # Device-setup handoff: the deploy staged + bundled the companion but does
    # not install it onto a phone or enable its services (a runtime step). Emit a
    # status the calling agent acts on — which devices are connected, and to ask
    # the user which to set up via phone_connect + phone_install_companion.
    if not args.no_companion and "phone-use" in enabled_skills:
        print_companion_setup_status(companion_setup_status())
    elif not args.no_companion:
        print("phone surface disabled; skipping phone companion setup handoff")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Deploy the sky-cua plugin locally: a fast install that updates what "
            "runs immediately (sky-cua@local). For a distributable release use "
            "python3 install.py build."
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
            "Force build, install (sudo cmake --install), and reload the sky-cua "
            "KWin agent-cursor effect (Linux/KDE only). Runs automatically when "
            "the effect is already installed; this forces it on first install."
        ),
    )
    parser.add_argument(
        "--no-kwin-effect",
        action="store_true",
        help="Skip the KWin agent-cursor effect step even if it is already installed.",
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
    parser.add_argument(
        "--codex-resources-root",
        type=Path,
        default=None,
        help=(
            "Codex Desktop resources root containing sky-cua-release.cjs and "
            "browser-use-cache-sync.cjs (auto-detected by default)."
        ),
    )
    parser.add_argument(
        "--browser-eval",
        choices=("on", "off"),
        default=None,
        help="Persist browser_eval availability for the refreshed local MCP install.",
    )
    parser.add_argument(
        "--model-supports-images",
        choices=("true", "false"),
        default=None,
        help="Persist an explicit model image-capability override for the refreshed local MCP install.",
    )
    parser.add_argument(
        "--refresh-accessibility",
        action="store_true",
        help=(
            "Reset the user AT-SPI registry (pkill at-spi2-registryd + restart "
            "at-spi-dbus-bus) before reconnecting. Off by default because it wipes "
            "every running app's accessibility registration; GTK apps must be "
            "relaunched afterwards. Use only when AT-SPI is genuinely wedged."
        ),
    )
    args = parser.parse_args(argv)
    return fast_deploy(args)


if __name__ == "__main__":
    raise SystemExit(main())
