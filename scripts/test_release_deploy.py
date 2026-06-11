"""Tests for the release plugin deploy flow."""

from __future__ import annotations

import os
import shutil
import sys
import time
import tomllib
from pathlib import Path

import pytest

import deploy_release_plugin as release_deploy
from _plugin_bundle import (
    COMPUTER_USE_COMPAT_PLUGIN_ID,
    PLUGIN_ID,
    RELEASE_PLUGIN_ID,
    all_runtime_binary_names,
    codex_config_path,
    compat_plugin_available,
    current_runtime_platform,
    release_plugin_root,
    runtime_binary_names,
    runtime_binary_path,
    update_codex_config,
)
from _test_support import (
    write_minimal_bundle,
)


def write_fake_compat_root(codex_home: Path) -> Path:
    compat_latest = codex_home / "plugins" / "cache" / "openai-bundled" / "computer-use" / "latest"
    compat_latest.mkdir(parents=True)
    (compat_latest / ".mcp.json").write_text(
        '{"mcpServers": {"computer-use": {"command": "/payload/bin/sky-cua-client"}}}',
        encoding="utf-8",
    )
    return compat_latest


def test_compat_plugin_available_requires_materialized_mcp_config(tmp_path: Path) -> None:
    codex_home = tmp_path / "codex-home"
    assert compat_plugin_available(codex_home) is False

    write_fake_compat_root(codex_home)
    assert compat_plugin_available(codex_home) is True


def test_update_codex_config_compat_enablement_owns_computer_use_toggles(
    tmp_path: Path,
) -> None:
    config_path = tmp_path / "config.toml"
    config_path.write_text(
        f'[plugins."{RELEASE_PLUGIN_ID}"]\nenabled = true\n',
        encoding="utf-8",
    )

    update_codex_config(config_path, compat_enablement=True)

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["plugins"][COMPUTER_USE_COMPAT_PLUGIN_ID]["enabled"] is True
    assert parsed["plugins"][RELEASE_PLUGIN_ID]["enabled"] is False
    assert parsed["plugins"][PLUGIN_ID]["enabled"] is False


def test_release_install_preserves_existing_other_platform_binaries(tmp_path: Path) -> None:
    marketplace_root = tmp_path / "marketplace"
    source = tmp_path / "bundle"
    destination = release_plugin_root(marketplace_root)
    current_binaries = runtime_binary_names()
    current_runtime_paths = [
        runtime_binary_path(current_runtime_platform(), name.removesuffix(".exe"))
        for name in current_binaries
    ]
    other_binaries = [
        name
        for name in all_runtime_binary_names()
        if Path("bin") / name not in current_runtime_paths
    ]
    write_minimal_bundle(
        source,
        binaries=[path.as_posix().removeprefix("bin/") for path in current_runtime_paths],
    )
    write_minimal_bundle(destination, binaries=other_binaries)

    installed = release_deploy.install_release_bundle(source, marketplace_root)

    assert installed == destination
    for _binary_name, runtime_path in zip(current_binaries, current_runtime_paths, strict=True):
        assert (destination / runtime_path).read_text(
            encoding="utf-8"
        ) == runtime_path.as_posix().removeprefix("bin/")
    for binary_name in other_binaries:
        assert (destination / "bin" / binary_name).read_text(encoding="utf-8") == binary_name


@pytest.mark.parametrize(
    "marketplace_header",
    ["[marketplaces.Heliasar]", '[marketplaces."Heliasar"]'],
)
def test_release_deploy_preserves_existing_git_marketplace_source(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    marketplace_header: str,
) -> None:
    codex_home = tmp_path / "codex-home"
    marketplace_root = tmp_path / "marketplace"
    bundle_root = tmp_path / "bundle"
    config_path = codex_home / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text(
        "\n".join(
            [
                marketplace_header,
                'source_type = "git"',
                'source = "https://github.com/becksclair/heliasar-marketplace.git"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    monkeypatch.setattr(release_deploy, "install_with_codex", lambda *_args: None)
    monkeypatch.setattr(release_deploy, "reload_mcp_servers", lambda *_args: None)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "deploy_release_plugin.py",
            "--no-build",
            "--codex-home",
            str(codex_home),
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
            "--codex-bin",
            "codex",
        ],
    )

    assert release_deploy.main() == 0

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["marketplaces"]["Heliasar"]["source_type"] == "git"
    assert parsed["marketplaces"]["Heliasar"]["source"] == (
        "https://github.com/becksclair/heliasar-marketplace.git"
    )
    assert parsed["plugins"][RELEASE_PLUGIN_ID]["enabled"] is True
    assert parsed["plugins"]["sky-cua@debug"]["enabled"] is False


def test_release_deploy_configures_local_marketplace_when_missing(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    codex_home = tmp_path / "codex-home"
    marketplace_root = tmp_path / "marketplace"
    bundle_root = tmp_path / "bundle"
    config_path = codex_home / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text(
        '[plugins."sky-cua@debug"]\nenabled = false\n',
        encoding="utf-8",
    )
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    monkeypatch.setattr(release_deploy, "install_with_codex", lambda *_args: None)
    monkeypatch.setattr(release_deploy, "reload_mcp_servers", lambda *_args: None)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "deploy_release_plugin.py",
            "--no-build",
            "--codex-home",
            str(codex_home),
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
            "--codex-bin",
            "codex",
        ],
    )

    assert release_deploy.main() == 0

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["marketplaces"]["Heliasar"]["source_type"] == "local"
    assert parsed["marketplaces"]["Heliasar"]["source"] == codex_config_path(marketplace_root)
    assert parsed["plugins"][RELEASE_PLUGIN_ID]["enabled"] is True
    assert parsed["plugins"]["sky-cua@debug"]["enabled"] is False


def test_release_deploy_prefers_compat_plugin_when_root_is_materialized(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    codex_home = tmp_path / "codex-home"
    marketplace_root = tmp_path / "marketplace"
    bundle_root = tmp_path / "bundle"
    config_path = codex_home / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text(
        f'[plugins."{RELEASE_PLUGIN_ID}"]\nenabled = true\n',
        encoding="utf-8",
    )
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())
    write_fake_compat_root(codex_home)

    refresh_calls: list[tuple[Path, Path]] = []
    monkeypatch.setattr(release_deploy, "install_with_codex", lambda *_args: None)
    monkeypatch.setattr(release_deploy, "reload_mcp_servers", lambda *_args: None)
    monkeypatch.setattr(
        release_deploy,
        "refresh_compat_plugin",
        lambda payload_root, home: refresh_calls.append((payload_root, home)),
    )
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "deploy_release_plugin.py",
            "--no-build",
            "--codex-home",
            str(codex_home),
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
            "--codex-bin",
            "codex",
        ],
    )

    assert release_deploy.main() == 0

    assert refresh_calls == [
        (release_plugin_root(marketplace_root), codex_home),
    ]
    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["plugins"][COMPUTER_USE_COMPAT_PLUGIN_ID]["enabled"] is True
    assert parsed["plugins"][RELEASE_PLUGIN_ID]["enabled"] is False
    assert parsed["plugins"][PLUGIN_ID]["enabled"] is False


def test_release_payload_root_prefers_codex_cache_payload(tmp_path: Path) -> None:
    codex_home = tmp_path / "codex-home"
    installed_path = tmp_path / "marketplace-payload"
    cache_payload = release_deploy.release_cache_root(codex_home) / "0.1.0"
    (cache_payload / "resources").mkdir(parents=True)
    (cache_payload / "resources" / "chrome_preflight.py").write_text("", encoding="utf-8")

    assert release_deploy.release_payload_root(codex_home, "0.1.0", installed_path) == cache_payload
    assert (
        release_deploy.release_payload_root(codex_home, "0.2.0", installed_path) == installed_path
    )


def test_latest_release_payload_orders_by_version_not_mtime(tmp_path: Path) -> None:
    codex_home = tmp_path / "codex-home"
    assert release_deploy.latest_release_payload(codex_home) is None

    cache_root = release_deploy.release_cache_root(codex_home)
    # copytree-based installs preserve source mtimes, so a freshly installed
    # higher version can carry an OLDER mtime than a stale cache entry.
    newest_version = cache_root / "0.10.0"
    newest_version.mkdir(parents=True)
    past = time.time() - 60
    os.utime(newest_version, (past, past))
    stale_touched = cache_root / "0.9.0"
    stale_touched.mkdir()
    junk = cache_root / "not-a-version"
    junk.mkdir()

    assert release_deploy.latest_release_payload(codex_home) == newest_version


def test_update_codex_config_channel_enable_disables_compat_id(tmp_path: Path) -> None:
    config_path = tmp_path / "config.toml"
    config_path.write_text(
        f'[plugins."{COMPUTER_USE_COMPAT_PLUGIN_ID}"]\nenabled = true\n',
        encoding="utf-8",
    )

    update_codex_config(config_path, plugin_id=RELEASE_PLUGIN_ID, compat_enablement=False)

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["plugins"][RELEASE_PLUGIN_ID]["enabled"] is True
    assert parsed["plugins"][COMPUTER_USE_COMPAT_PLUGIN_ID]["enabled"] is False


def test_update_codex_config_disable_only_call_leaves_compat_id_alone(tmp_path: Path) -> None:
    config_path = tmp_path / "config.toml"
    config_path.write_text(
        f'[plugins."{COMPUTER_USE_COMPAT_PLUGIN_ID}"]\nenabled = true\n',
        encoding="utf-8",
    )

    update_codex_config(config_path, plugin_id=RELEASE_PLUGIN_ID, plugin_enabled=False)

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["plugins"][RELEASE_PLUGIN_ID]["enabled"] is False
    assert parsed["plugins"][COMPUTER_USE_COMPAT_PLUGIN_ID]["enabled"] is True


def test_release_deploy_restores_cache_when_reload_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    codex_home = tmp_path / "codex-home"
    marketplace_root = tmp_path / "marketplace"
    bundle_root = tmp_path / "bundle"
    cache_version = release_deploy.release_cache_root(codex_home) / "0.1.0"
    cache_version.mkdir(parents=True)
    (cache_version / "old-marker.txt").write_text("old", encoding="utf-8")
    compat_latest = write_fake_compat_root(codex_home)
    previous_compat_mcp = (compat_latest / ".mcp.json").read_text(encoding="utf-8")

    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    def fake_install_with_codex(
        _codex_bin: Path, codex_home_arg: Path, _manifest_path: Path
    ) -> None:
        target = release_deploy.release_cache_root(codex_home_arg) / "0.1.0"
        shutil.rmtree(target)
        target.mkdir(parents=True)
        (target / "new-marker.txt").write_text("new", encoding="utf-8")

    def fake_refresh_compat_plugin(_payload_root: Path, codex_home_arg: Path) -> None:
        # Simulate the preflight retargeting the compat root at the payload
        # the rollback is about to delete.
        mcp_path = (
            codex_home_arg
            / "plugins"
            / "cache"
            / "openai-bundled"
            / "computer-use"
            / "latest"
            / ".mcp.json"
        )
        mcp_path.write_text(
            '{"mcpServers": {"computer-use": {"command": "/deleted/payload"}}}',
            encoding="utf-8",
        )

    def fail_reload(_codex_bin: Path, _codex_home: Path) -> None:
        raise RuntimeError("reload failed")

    monkeypatch.setattr(release_deploy, "install_with_codex", fake_install_with_codex)
    monkeypatch.setattr(release_deploy, "refresh_compat_plugin", fake_refresh_compat_plugin)
    monkeypatch.setattr(release_deploy, "reload_mcp_servers", fail_reload)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "deploy_release_plugin.py",
            "--no-build",
            "--codex-home",
            str(codex_home),
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
            "--codex-bin",
            "codex",
        ],
    )

    with pytest.raises(RuntimeError, match="reload failed"):
        release_deploy.main()

    assert (cache_version / "old-marker.txt").read_text(encoding="utf-8") == "old"
    assert not (cache_version / "new-marker.txt").exists()
    restored_compat_mcp = (compat_latest / ".mcp.json").read_text(encoding="utf-8")
    assert restored_compat_mcp == previous_compat_mcp
