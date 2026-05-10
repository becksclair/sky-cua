#!/usr/bin/env python3
from __future__ import annotations

import argparse
import subprocess
from pathlib import Path

from _plugin_bundle import (
    DEFAULT_CODEX_HOME,
    DEFAULT_MARKETPLACE_ROOT,
    PLUGIN_ID,
    RELEASE_MARKETPLACE_NAME,
    RELEASE_PLUGIN_ID,
    marketplace_manifest_path,
    update_codex_config,
)
from deploy_release_plugin import install_with_codex, reload_mcp_servers, resolve_codex_bin
from publish_marketplace_release import DEFAULT_MARKETPLACE_SOURCE


def run(
    command: list[str], *, cwd: Path | None = None, check: bool = True
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, check=check, text=True)


def ensure_marketplace_checkout(marketplace_root: Path, marketplace_source: str) -> None:
    if (marketplace_root / ".git").exists():
        run(["git", "pull", "--ff-only"], cwd=marketplace_root)
        return
    marketplace_root.parent.mkdir(parents=True, exist_ok=True)
    clone_url = marketplace_source
    if "/" in marketplace_source and not marketplace_source.startswith(
        ("git@", "http://", "https://", "ssh://")
    ):
        clone_url = f"https://github.com/{marketplace_source}.git"
    run(["git", "clone", clone_url, str(marketplace_root)])


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Clone/add the Heliasar Codex marketplace and install sky-cua."
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
        help="Codex marketplace source to add (default: becksclair/heliasar-marketplace).",
    )
    parser.add_argument(
        "--codex-bin",
        type=Path,
        default=None,
        help="Codex executable used for marketplace add and plugin install.",
    )
    args = parser.parse_args()

    marketplace_root = args.marketplace_root.expanduser().resolve()
    codex_bin = resolve_codex_bin(args.codex_bin)

    ensure_marketplace_checkout(marketplace_root, args.marketplace_source)
    add = run(
        [str(codex_bin), "plugin", "marketplace", "add", args.marketplace_source],
        check=False,
    )
    if add.returncode != 0:
        run([str(codex_bin), "plugin", "marketplace", "upgrade", RELEASE_MARKETPLACE_NAME])

    manifest_path = marketplace_manifest_path(marketplace_root)
    install_with_codex(codex_bin, args.codex_home, manifest_path)
    update_codex_config(
        args.codex_home / "config.toml",
        plugin_id=RELEASE_PLUGIN_ID,
        disabled_plugin_ids=[PLUGIN_ID],
    )
    reload_mcp_servers(codex_bin, args.codex_home)

    print(f"marketplace_root={marketplace_root}")
    print(f"marketplace_manifest={manifest_path}")
    print(f"plugin_id={RELEASE_PLUGIN_ID}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
