#!/usr/bin/env python3
from __future__ import annotations

import argparse
import filecmp
import os
import subprocess
from pathlib import Path

import _install_shared
from _plugin_bundle import (
    DEFAULT_CODEX_HOME,
    DEFAULT_MARKETPLACE_ROOT,
    DIST_PLUGIN_ROOT,
    PLUGIN_ID,
    PLUGIN_NAME,
    RELEASE_MARKETPLACE_NAME,
    RELEASE_PLUGIN_ID,
    build_bundle,
    current_runtime_platform,
    merge_runtime_artifacts,
    platform_runtime_binary_base_names,
    runtime_binary_path,
    runtime_binary_source_name,
    update_codex_config,
    update_plugin_manifest_version,
    version_from_tag,
    write_release_marketplace,
)
from deploy_release_plugin import (
    install_release_bundle,
    install_with_codex,
    plugin_version,
    reload_mcp_servers,
    resolve_codex_bin,
)
from install_mcp_server import install_local_mcp_server

DEFAULT_LOCAL_INSTALL_DIR = Path.home() / ".local" / "share" / "sky-cua"

DEFAULT_MARKETPLACE_SOURCE = "becksclair/heliasar-marketplace"


def run(
    command: list[str], *, cwd: Path | None = None, check: bool = True
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, check=check, text=True)


def git_has_head(repo_root: Path) -> bool:
    return (
        subprocess.run(
            ["git", "rev-parse", "--verify", "HEAD"],
            cwd=repo_root,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        ).returncode
        == 0
    )


def git_has_changes(repo_root: Path) -> bool:
    result = subprocess.run(
        ["git", "status", "--porcelain", "--", ".agents", "plugins"],
        cwd=repo_root,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return bool(result.stdout.strip())


def commit_marketplace(repo_root: Path, version: str) -> bool:
    run(["git", "add", ".agents/plugins/marketplace.json", f"plugins/{PLUGIN_NAME}"], cwd=repo_root)
    if not git_has_changes(repo_root):
        return False
    message = f"Update {PLUGIN_NAME} plugin to {version}"
    run(["git", "commit", "-m", message], cwd=repo_root)
    return True


def configure_marketplace(codex_bin: Path, marketplace_source: str) -> None:
    upgrade = run(
        [str(codex_bin), "plugin", "marketplace", "upgrade", RELEASE_MARKETPLACE_NAME],
        check=False,
    )
    if upgrade.returncode == 0:
        return
    run([str(codex_bin), "plugin", "marketplace", "add", marketplace_source])
    run([str(codex_bin), "plugin", "marketplace", "upgrade", RELEASE_MARKETPLACE_NAME])


def stale_bundle_binaries(bundle_root: Path, release_dir: Path) -> list[str]:
    """Return current-platform bundle binaries whose content differs from target/release.

    Guards --no-build publishes: a rebuilt target/release with an unrebuilt
    bundle means the publish would silently ship old code. Binaries missing
    on either side are skipped so artifact-only flows (CI) stay unaffected.
    """
    platform_id = current_runtime_platform()
    stale: list[str] = []
    for name in platform_runtime_binary_base_names(platform_id):
        bundle_binary = bundle_root / runtime_binary_path(platform_id, name)
        release_binary = release_dir / runtime_binary_source_name(platform_id, name)
        if not bundle_binary.exists() or not release_binary.exists():
            continue
        if not filecmp.cmp(bundle_binary, release_binary, shallow=False):
            stale.append(name)
    return stale


def current_tag() -> str:
    for name in ["GITHUB_REF_NAME", "GITEA_REF_NAME", "CI_COMMIT_TAG"]:
        value = os.environ.get(name, "").strip()
        if value:
            return value.removeprefix("refs/tags/")
    result = subprocess.run(
        ["git", "describe", "--tags", "--exact-match"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return result.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build, publish, and install the sky-cua release plugin through Heliasar."
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=DEFAULT_CODEX_HOME,
        help="Codex home directory whose config.toml should enable the release plugin.",
    )
    parser.add_argument(
        "--marketplace-root",
        type=Path,
        default=DEFAULT_MARKETPLACE_ROOT,
        help="Local Heliasar marketplace checkout (default: ~/projects/heliasar-marketplace).",
    )
    parser.add_argument(
        "--marketplace-source",
        default=DEFAULT_MARKETPLACE_SOURCE,
        help="Codex marketplace source for first-time setup (default: becksclair/heliasar-marketplace).",
    )
    parser.add_argument(
        "--bundle-root",
        type=Path,
        default=DIST_PLUGIN_ROOT,
        help="Built bundle to publish (default: dist/plugin/sky-cua).",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Publish the existing bundle without rebuilding first.",
    )
    parser.add_argument(
        "--allow-stale-bundle",
        action="store_true",
        help="With --no-build, publish even when bundle binaries differ from target/release.",
    )
    parser.add_argument(
        "--runtime-artifacts",
        type=Path,
        default=None,
        help="Directory containing linux-x64, linux-arm64, and windows-x64 runtime artifacts.",
    )
    parser.add_argument(
        "--version-from-tag",
        action="store_true",
        help="Set plugin.json version from the current vX.Y.Z tag before publishing.",
    )
    parser.add_argument(
        "--no-push",
        action="store_true",
        help="Commit the marketplace but do not push it.",
    )
    parser.add_argument(
        "--skip-codex-install",
        action="store_true",
        help="Only stage/commit/push the marketplace; do not configure or install in Codex.",
    )
    parser.add_argument(
        "--skip-local-install",
        action="store_true",
        help="Do not refresh the local MCP-server install or restart its runtime.",
    )
    parser.add_argument(
        "--local-install-dir",
        type=Path,
        default=DEFAULT_LOCAL_INSTALL_DIR,
        help=f"Local MCP-server install to refresh (default: {DEFAULT_LOCAL_INSTALL_DIR}).",
    )
    parser.add_argument(
        "--local-install-host",
        default="claude-code",
        choices=("generic", "opencode", "claude-code", "claude-desktop", "pi", "openclaw"),
        help="Host config format for the local MCP-server install (default: claude-code).",
    )
    parser.add_argument(
        "--codex-bin",
        type=Path,
        default=None,
        help="Codex executable used for marketplace upgrade and plugin install.",
    )
    args = parser.parse_args()

    if not args.no_build:
        build_bundle()

    bundle_root = args.bundle_root.resolve()
    if args.no_build and not args.allow_stale_bundle:
        stale = stale_bundle_binaries(bundle_root, _install_shared.REPO_ROOT / "target" / "release")
        if stale:
            raise RuntimeError(
                f"bundle binaries differ from target/release ({', '.join(stale)}); "
                "rerun without --no-build (or scripts/build_plugin.py first), "
                "or pass --allow-stale-bundle to publish the bundle as-is"
            )
    if args.runtime_artifacts is not None:
        merge_runtime_artifacts(bundle_root, args.runtime_artifacts.resolve())
    if args.version_from_tag:
        update_plugin_manifest_version(bundle_root, version_from_tag(current_tag()))

    marketplace_root = args.marketplace_root.expanduser().resolve()
    if not (marketplace_root / ".git").exists() or not git_has_head(marketplace_root):
        raise RuntimeError(
            f"{marketplace_root} must be a published git repository before running this script"
        )

    installed_path = install_release_bundle(bundle_root, marketplace_root)
    manifest_path = write_release_marketplace(marketplace_root)
    version = plugin_version(bundle_root)

    committed = commit_marketplace(marketplace_root, version)
    if not args.no_push:
        run(["git", "push", "origin", "main"], cwd=marketplace_root)

    if not args.skip_codex_install:
        codex_bin = resolve_codex_bin(args.codex_bin)
        configure_marketplace(codex_bin, args.marketplace_source)
        install_with_codex(codex_bin, args.codex_home, manifest_path)
        update_codex_config(
            args.codex_home / "config.toml",
            plugin_id=RELEASE_PLUGIN_ID,
            disabled_plugin_ids=[PLUGIN_ID],
        )
        reload_mcp_servers(codex_bin, args.codex_home)

    # Keep the local MCP-server install (and the shared daemon it backs) in
    # lockstep with the published release so the two channels cannot drift.
    # Skipped alongside --skip-codex-install, which marks repo-only runs (CI).
    refresh_local_install = not args.skip_local_install and not args.skip_codex_install
    local_install_dir = args.local_install_dir.expanduser().resolve()
    if refresh_local_install:
        install_local_mcp_server(
            local_install_dir,
            args.local_install_host,
            restart_runtime=True,
            bundle_root=bundle_root,
        )

    print(f"marketplace_root={marketplace_root}")
    print(f"marketplace_manifest={manifest_path}")
    print(f"installed_path={installed_path}")
    print(f"plugin_id={RELEASE_PLUGIN_ID}")
    print(f"committed={str(committed).lower()}")
    print(f"pushed={str(not args.no_push).lower()}")
    if refresh_local_install:
        print(f"local_install={local_install_dir}")
        print(f"local_install_host={args.local_install_host}")
    else:
        print("local_install=skipped")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
