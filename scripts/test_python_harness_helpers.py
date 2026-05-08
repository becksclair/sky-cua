from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

import build_plugin
import deploy_release_plugin as release_deploy
from _app_server_harness import build_schema_accept_value, response_contains_computer_use_server
from _plugin_bundle import (
    RELEASE_PLUGIN_ID,
    codex_config_path,
    ensure_apps_feature_disabled,
    ensure_fast_service_tier,
    ensure_plugin_enabled,
    ensure_plugins_feature_enabled,
    executable_name,
    marketplace_manifest_path,
    release_plugin_root,
    runtime_binary_names,
    update_codex_config,
    write_release_marketplace,
)
from _tidal_workflow import tidal_playlist_prompt
from live_app_server_tidal_image_ab import DEFAULT_VARIANTS, playlist_name_for_variant


def test_codex_config_helpers_update_existing_sections() -> None:
    config = "\n".join(
        [
            'service_tier = "flex"',
            "",
            "[features]",
            "plugins = false",
            "apps = true",
            "",
            '[plugins."sky-cua@debug"]',
            "enabled = false",
            "",
            "[profiles.default]",
            'service_tier = "flex"',
            "",
        ]
    )

    config = ensure_plugins_feature_enabled(config)
    config = ensure_apps_feature_disabled(config)
    config = ensure_fast_service_tier(config)
    config = ensure_plugin_enabled(config)

    assert 'service_tier = "fast"' in config
    assert "plugins = true" in config
    assert "apps = false" in config
    assert "enabled = true" in config
    assert "[profiles.default]\n" in config
    assert 'profiles.default]\nservice_tier = "flex"' not in config


def test_runtime_binary_names_match_host_platform() -> None:
    suffix = ".exe" if executable_name("tool").endswith(".exe") else ""

    assert runtime_binary_names() == [
        f"sky-cua-client{suffix}",
        f"sky-cua-service{suffix}",
    ]


def test_build_bundle_inputs_are_selected_from_git_index(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    subprocess.run(["git", "init"], cwd=tmp_path, check=True, stdout=subprocess.DEVNULL)
    (tmp_path / "README.md").write_text("tracked readme\n", encoding="utf-8")
    (tmp_path / "docs").mkdir()
    (tmp_path / "docs" / "kept.md").write_text("tracked doc\n", encoding="utf-8")
    (tmp_path / "docs" / "local-only.md").write_text("untracked doc\n", encoding="utf-8")
    subprocess.run(
        ["git", "add", "README.md", "docs/kept.md"],
        cwd=tmp_path,
        check=True,
        stdout=subprocess.DEVNULL,
    )

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)

    assert build_plugin.tracked_bundle_files([Path("README.md"), Path("docs")]) == [
        Path("README.md"),
        Path("docs/kept.md"),
    ]


def test_release_marketplace_helpers_use_local_marketplace_shape(tmp_path: Path) -> None:
    marketplace_root = tmp_path / "marketplace"
    config_path = tmp_path / "codex-home" / "config.toml"

    manifest_path = write_release_marketplace(marketplace_root)
    manifest = json.loads(manifest_path.read_text())
    update_codex_config(
        config_path,
        plugin_id=RELEASE_PLUGIN_ID,
        disabled_plugin_ids=["sky-cua@debug"],
        marketplace_root=marketplace_root,
    )
    config = config_path.read_text()

    assert manifest_path == marketplace_manifest_path(marketplace_root)
    assert manifest_path.exists()
    assert manifest["plugins"][0]["source"] == {
        "source": "local",
        "path": "./plugins/sky-cua",
    }
    assert release_plugin_root(marketplace_root) == marketplace_root / "plugins" / "sky-cua"
    assert "[marketplaces.sky-cua-local]" in config
    assert str(marketplace_root.resolve()).replace("\\", "\\\\") in config
    assert f'[plugins."{RELEASE_PLUGIN_ID}"]' in config
    assert "enabled = true" in config
    assert '[plugins."sky-cua@debug"]\nenabled = false' in config


def test_codex_config_upsert_preserves_windows_backslashes(tmp_path: Path) -> None:
    config_path = tmp_path / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text(
        "\n".join(
            [
                "[marketplaces.sky-cua-local]",
                'source = "old"',
                'source_type = "local"',
                "",
            ]
        )
    )
    marketplace_root = Path(r"C:\Users\bex\.agents\sky-cua-marketplace")

    update_codex_config(
        config_path,
        plugin_id=RELEASE_PLUGIN_ID,
        marketplace_root=marketplace_root,
    )
    config = config_path.read_text()

    parsed = tomllib.loads(config)
    assert parsed["marketplaces"]["sky-cua-local"]["source"] == codex_config_path(marketplace_root)
    assert "C:\\\\Users\\\\bex" in config


def test_update_codex_config_can_stage_disabled_plugin_before_install(tmp_path: Path) -> None:
    config_path = tmp_path / "codex-home" / "config.toml"

    update_codex_config(
        config_path,
        plugin_id=RELEASE_PLUGIN_ID,
        plugin_enabled=False,
        disabled_plugin_ids=["sky-cua@debug"],
        marketplace_root=tmp_path / "marketplace",
    )
    parsed = tomllib.loads(config_path.read_text())

    assert parsed["plugins"][RELEASE_PLUGIN_ID]["enabled"] is False
    assert parsed["plugins"]["sky-cua@debug"]["enabled"] is False
    assert parsed["features"]["plugins"] is True


def test_codex_config_upsert_updates_crlf_sections_without_duplicate_tables(tmp_path: Path) -> None:
    config_path = tmp_path / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text(
        '[features]\r\nplugins = false\r\n\r\n[plugins."sky-cua@debug"]\r\nenabled = false\r\n'
    )

    update_codex_config(config_path)
    config = config_path.read_text()
    parsed = tomllib.loads(config)

    assert config.count("[features]") == 1
    assert config.count('[plugins."sky-cua@debug"]') == 1
    assert parsed["features"]["plugins"] is True
    assert parsed["plugins"]["sky-cua@debug"]["enabled"] is True


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

    (bundle_root / ".codex-plugin").mkdir(parents=True)
    (bundle_root / ".codex-plugin" / "plugin.json").write_text(
        json.dumps({"version": "0.1.0"}),
        encoding="utf-8",
    )
    (bundle_root / ".mcp.json").write_text("{}", encoding="utf-8")
    (bundle_root / "skills" / "computer-use-workflows").mkdir(parents=True)
    (bundle_root / "skills" / "computer-use-workflows" / "SKILL.md").write_text(
        "skill",
        encoding="utf-8",
    )
    (bundle_root / "resources" / "app-instructions").mkdir(parents=True)
    (bundle_root / "resources" / "app-instructions" / "index.json").write_text(
        "{}",
        encoding="utf-8",
    )
    (bundle_root / "bin").mkdir(parents=True)
    for binary_name in runtime_binary_names():
        (bundle_root / "bin" / binary_name).write_text("binary", encoding="utf-8")

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


def test_build_schema_accept_value_prefers_required_fields_and_enums() -> None:
    value = build_schema_accept_value(
        {
            "type": "object",
            "required": ["decision", "count", "flags"],
            "properties": {
                "decision": {"type": "string", "enum": ["accept", "decline"]},
                "count": {"type": "integer"},
                "flags": {
                    "type": "array",
                    "minItems": 2,
                    "items": {"type": "boolean"},
                },
                "optional": {"type": "string"},
            },
        }
    )

    assert value == {
        "decision": "accept",
        "count": 1,
        "flags": [True, True],
    }


def test_response_contains_computer_use_server_accepts_common_shapes() -> None:
    assert response_contains_computer_use_server(
        {"result": {"servers": [{"name": "computer-use"}]}}
    )
    assert response_contains_computer_use_server({"result": {"data": [{"name": "computer-use"}]}})
    assert response_contains_computer_use_server(
        {"result": {"items": [{"server": "computer-use"}]}}
    )
    assert not response_contains_computer_use_server({"result": {"servers": []}})


def test_tidal_prompt_uses_custom_playlist_name() -> None:
    prompt = tidal_playlist_prompt(
        app_server=True, playlist_name="Codex Favorites AB test webp-q85"
    )

    assert "Codex Favorites AB test webp-q85" in prompt


def test_tidal_ab_playlist_names_are_variant_scoped() -> None:
    names = [playlist_name_for_variant("20260424T120000Z", variant) for variant in DEFAULT_VARIANTS]

    assert len(names) == len(set(names))
    assert all(name.startswith("Codex Favorites AB 20260424T120000Z ") for name in names)
