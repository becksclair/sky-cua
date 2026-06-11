"""Tests for the release plugin deploy flow."""

from __future__ import annotations

import shutil
import sys
import tomllib
from pathlib import Path

import pytest

import deploy_release_plugin as release_deploy
from _plugin_bundle import (
    RELEASE_PLUGIN_ID,
    all_runtime_binary_names,
    codex_config_path,
    current_runtime_platform,
    release_plugin_root,
    runtime_binary_names,
    runtime_binary_path,
)
from _test_support import (
    write_minimal_bundle,
)


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

    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    def fake_install_with_codex(
        _codex_bin: Path, codex_home_arg: Path, _manifest_path: Path
    ) -> None:
        target = release_deploy.release_cache_root(codex_home_arg) / "0.1.0"
        shutil.rmtree(target)
        target.mkdir(parents=True)
        (target / "new-marker.txt").write_text("new", encoding="utf-8")

    def fail_reload(_codex_bin: Path, _codex_home: Path) -> None:
        raise RuntimeError("reload failed")

    monkeypatch.setattr(release_deploy, "install_with_codex", fake_install_with_codex)
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
