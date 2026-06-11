#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import tempfile
import tomllib
from pathlib import Path

from _codex_app_server import CodexAppServerClient
from _plugin_bundle import (
    DEFAULT_CODEX_HOME,
    DEFAULT_MARKETPLACE_ROOT,
    DIST_PLUGIN_ROOT,
    PLUGIN_ID,
    PLUGIN_NAME,
    RELEASE_MARKETPLACE_NAME,
    RELEASE_PLUGIN_ID,
    build_bundle,
    copytree_replace,
    copytree_replace_preserving_platform_binaries,
    ensure_bundle_structure,
    marketplace_manifest_path,
    release_plugin_root,
    remove_path,
    stop_unix_runtime_processes,
    stop_windows_cache_processes,
    update_codex_config,
    write_release_marketplace,
)


def app_server_client(codex_bin: Path, codex_home: Path) -> CodexAppServerClient:
    env = os.environ.copy()
    env["CODEX_HOME"] = str(codex_home)
    return CodexAppServerClient(
        [str(codex_bin), "app-server", "--listen", "stdio://"],
        env=env,
    )


def resolve_codex_bin(codex_bin: Path | None) -> Path:
    if codex_bin is not None:
        return codex_bin
    discovered = shutil.which("codex")
    if discovered is None:
        raise FileNotFoundError("codex executable not found on PATH; pass --codex-bin")
    return Path(discovered)


def install_with_codex(codex_bin: Path, codex_home: Path, marketplace_path: Path) -> None:
    client = app_server_client(codex_bin, codex_home)
    try:
        client.initialize(client_name="sky-cua-release-deploy", client_version="0")
        client.request(
            "plugin/install",
            {
                "marketplacePath": str(marketplace_path.resolve()),
                "pluginName": PLUGIN_NAME,
            },
        )
    finally:
        client.close()


def release_cache_root(codex_home: Path) -> Path:
    return codex_home / "plugins" / "cache" / RELEASE_MARKETPLACE_NAME / PLUGIN_NAME


def reload_mcp_servers(codex_bin: Path, codex_home: Path) -> None:
    client = app_server_client(codex_bin, codex_home)
    try:
        client.initialize(client_name="sky-cua-release-deploy", client_version="0")
        client.request("config/mcpServer/reload")
    finally:
        client.close()


def install_release_bundle(bundle_root: Path, marketplace_root: Path) -> Path:
    source = bundle_root.resolve()
    ensure_bundle_structure(source)
    destination = release_plugin_root(marketplace_root)
    destination.parent.mkdir(parents=True, exist_ok=True)
    copytree_replace_preserving_platform_binaries(source, destination)
    ensure_bundle_structure(destination)
    return destination


def plugin_version(bundle_root: Path) -> str:
    metadata_path = bundle_root / ".codex-plugin" / "plugin.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    version = metadata.get("version")
    if not isinstance(version, str) or not version:
        raise ValueError(f"{metadata_path} is missing a string version")
    return version


def install_release_cache(bundle_root: Path, codex_home: Path) -> Path:
    source = bundle_root.resolve()
    ensure_bundle_structure(source)
    destination = release_cache_root(codex_home) / plugin_version(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    copytree_replace(source, destination)
    ensure_bundle_structure(destination)
    return destination


def config_has_release_marketplace(config_text: str) -> bool:
    try:
        parsed = tomllib.loads(config_text)
    except tomllib.TOMLDecodeError:
        return (
            f"[marketplaces.{RELEASE_MARKETPLACE_NAME}]" in config_text
            or f'[marketplaces."{RELEASE_MARKETPLACE_NAME}"]' in config_text
        )
    marketplaces = parsed.get("marketplaces")
    return isinstance(marketplaces, dict) and RELEASE_MARKETPLACE_NAME in marketplaces


def is_cache_backup_access_denied(error: RuntimeError) -> bool:
    message = str(error)
    return "failed to back up plugin cache entry" in message and "Access is denied" in message


def snapshot_path(path: Path, backup_root: Path, label: str) -> Path | None:
    if not path.exists() and not path.is_symlink():
        return None
    backup_path = backup_root / label
    if path.is_dir() and not path.is_symlink():
        shutil.copytree(path, backup_path)
    else:
        backup_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, backup_path)
    return backup_path


def restore_snapshot(path: Path, backup_path: Path | None) -> None:
    remove_path(path)
    if backup_path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    if backup_path.is_dir() and not backup_path.is_symlink():
        shutil.copytree(backup_path, path)
    else:
        shutil.copy2(backup_path, path)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build and deploy sky-cua as a local marketplace release plugin."
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
        help="Local marketplace root (default: ~/projects/heliasar-marketplace).",
    )
    parser.add_argument(
        "--bundle-root",
        type=Path,
        default=DIST_PLUGIN_ROOT,
        help="Built bundle to deploy (default: dist/plugin/sky-cua).",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Deploy the existing bundle without rebuilding first.",
    )
    parser.add_argument(
        "--codex-bin",
        type=Path,
        default=None,
        help="Codex executable used for plugin/install (default: first codex on PATH).",
    )
    parser.add_argument(
        "--skip-codex-install",
        action="store_true",
        help="Only stage the marketplace/config; do not call codex app-server plugin/install.",
    )
    args = parser.parse_args()

    if not args.no_build:
        build_bundle()

    release_root = release_plugin_root(args.marketplace_root)
    manifest_path = marketplace_manifest_path(args.marketplace_root)
    backup_dir = Path(tempfile.mkdtemp(prefix="sky-cua-release-deploy-"))
    plugin_backup = snapshot_path(release_root, backup_dir, "marketplace-plugin")
    manifest_backup = snapshot_path(manifest_path, backup_dir, "marketplace.json")
    config_path = args.codex_home / "config.toml"
    cache_root = release_cache_root(args.codex_home)
    cache_backup = snapshot_path(cache_root, backup_dir, "release-cache")
    codex_bin = resolve_codex_bin(args.codex_bin) if not args.skip_codex_install else None
    codex_install = "skipped"
    cache_path: Path | None = None
    deploy_complete = False
    installed_path = release_root
    previous_config = config_path.read_text() if config_path.exists() else None
    configure_local_marketplace = not config_has_release_marketplace(previous_config or "")
    try:
        stop_unix_runtime_processes([cache_root.parent, release_root])
        if not args.skip_codex_install:
            assert codex_bin is not None
            installed_path = install_release_bundle(args.bundle_root, args.marketplace_root)
            manifest_path = write_release_marketplace(args.marketplace_root)
            update_codex_config(
                config_path,
                plugin_id=RELEASE_PLUGIN_ID,
                plugin_enabled=False,
                marketplace_root=args.marketplace_root if configure_local_marketplace else None,
            )
            stop_windows_cache_processes(cache_root)
            try:
                install_with_codex(codex_bin, args.codex_home, manifest_path)
                codex_install = "ok"
            except RuntimeError as error:
                if not is_cache_backup_access_denied(error):
                    raise
                cache_path = install_release_cache(installed_path, args.codex_home)
                codex_install = "direct-cache-fallback"
            update_codex_config(
                config_path,
                plugin_id=RELEASE_PLUGIN_ID,
                disabled_plugin_ids=[PLUGIN_ID],
                marketplace_root=args.marketplace_root if configure_local_marketplace else None,
            )
            reload_mcp_servers(codex_bin, args.codex_home)
            deploy_complete = True
        else:
            installed_path = install_release_bundle(args.bundle_root, args.marketplace_root)
            manifest_path = write_release_marketplace(args.marketplace_root)
            update_codex_config(
                config_path,
                plugin_id=RELEASE_PLUGIN_ID,
                disabled_plugin_ids=[PLUGIN_ID],
                marketplace_root=args.marketplace_root if configure_local_marketplace else None,
            )
            deploy_complete = True
    except Exception:
        if previous_config is None:
            config_path.unlink(missing_ok=True)
        else:
            config_path.write_text(previous_config)
        restore_snapshot(cache_root, cache_backup)
        restore_snapshot(release_root, plugin_backup)
        restore_snapshot(manifest_path, manifest_backup)
        raise

    if deploy_complete:
        shutil.rmtree(backup_dir, ignore_errors=True)

    print(f"marketplace_root={args.marketplace_root}")
    print(f"marketplace_manifest={manifest_path}")
    print(f"installed_path={installed_path}")
    print(f"plugin_id={RELEASE_PLUGIN_ID}")
    print(f"config_path={config_path}")
    if not args.skip_codex_install:
        print(f"codex_install={codex_install}")
    if cache_path is not None:
        print(f"cache_path={cache_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
