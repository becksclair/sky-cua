from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

import _plugin_bundle as plugin_bundle
import build_plugin
import deploy_release_plugin as release_deploy
import publish_marketplace_release
import setup_heliasar_marketplace
from _app_server_harness import build_schema_accept_value, response_contains_computer_use_server
from _plugin_bundle import (
    PLUGIN_CATEGORY,
    RELEASE_PLUGIN_ID,
    all_runtime_binary_names,
    codex_config_path,
    ensure_apps_feature_disabled,
    ensure_fast_service_tier,
    ensure_plugin_enabled,
    ensure_plugins_feature_enabled,
    executable_name,
    marketplace_manifest_path,
    release_plugin_root,
    runtime_binary_names,
    stop_unix_runtime_processes,
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


def test_all_runtime_binary_names_include_linux_and_windows_binaries() -> None:
    assert all_runtime_binary_names() == [
        "sky-cua-client",
        "sky-cua-service",
        "sky-cua-client.exe",
        "sky-cua-service.exe",
    ]


def test_stop_unix_runtime_processes_targets_deleted_cache_process(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    if sys.platform == "win32":
        pytest.skip("Unix process cleanup is not used on Windows")

    cache_root = tmp_path / "codex" / "plugins" / "cache" / "Heliasar"
    deleted_exe = cache_root / "plugin-backup-old" / "sky-cua" / "0.1.0" / "bin" / "sky-cua-client"
    deleted_exe.parent.mkdir(parents=True)

    proc_root = tmp_path / "proc"
    match_proc = proc_root / "123"
    match_proc.mkdir(parents=True)
    (match_proc / "cmdline").write_bytes(str(deleted_exe).encode() + b"\0mcp")
    (match_proc / "exe").symlink_to(f"{deleted_exe} (deleted)")
    (match_proc / "cwd").symlink_to(f"{deleted_exe.parent.parent} (deleted)")

    ignored_proc = proc_root / "456"
    ignored_proc.mkdir()
    (ignored_proc / "cmdline").write_bytes(b"/usr/bin/sky-cua-client\0mcp")
    (ignored_proc / "exe").symlink_to("/usr/bin/sky-cua-client")
    (ignored_proc / "cwd").symlink_to("/usr/bin")

    terminated: set[int] = set()
    calls: list[tuple[int, int]] = []

    def fake_kill(pid: int, signal: int) -> None:
        calls.append((pid, signal))
        if signal == plugin_bundle.SIGTERM:
            terminated.add(pid)
        if signal == 0 and pid in terminated:
            raise ProcessLookupError

    monkeypatch.setattr(plugin_bundle.os, "kill", fake_kill)

    stop_unix_runtime_processes([cache_root], proc_root=proc_root)

    assert (123, plugin_bundle.SIGTERM) in calls
    assert all(pid != 456 for pid, _signal in calls)


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


def test_bundle_source_paths_include_standard_optional_plugin_roots() -> None:
    assert Path(".codex-plugin") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path(".app.json") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path("assets") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path("hooks") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path("skills") in build_plugin.BUNDLE_SOURCE_PATHS


def write_minimal_bundle(root: Path, *, binaries: list[str]) -> None:
    write_minimal_bundle_sources(root)
    (root / "bin").mkdir(parents=True)
    for binary_name in binaries:
        (root / "bin" / binary_name).write_text(binary_name, encoding="utf-8")


def write_minimal_bundle_sources(root: Path) -> None:
    (root / ".codex-plugin").mkdir(parents=True)
    (root / ".codex-plugin" / "plugin.json").write_text(
        json.dumps({"version": "0.1.0"}),
        encoding="utf-8",
    )
    (root / ".mcp.json").write_text("{}", encoding="utf-8")
    (root / "skills" / "computer-use-workflows").mkdir(parents=True)
    (root / "skills" / "computer-use-workflows" / "SKILL.md").write_text(
        "skill",
        encoding="utf-8",
    )
    (root / "resources" / "app-instructions").mkdir(parents=True)
    (root / "resources" / "app-instructions" / "index.json").write_text(
        "{}",
        encoding="utf-8",
    )


def tracked_minimal_bundle_files() -> list[Path]:
    return [
        Path(".codex-plugin/plugin.json"),
        Path("skills/computer-use-workflows/SKILL.md"),
        Path("resources/app-instructions/index.json"),
    ]


def test_stage_bundle_preserves_existing_other_platform_binaries(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    bundle_root = tmp_path / "dist" / "plugin" / "sky-cua"
    current_binaries = runtime_binary_names()
    other_binaries = [name for name in all_runtime_binary_names() if name not in current_binaries]
    write_minimal_bundle(bundle_root, binaries=other_binaries)
    write_minimal_bundle_sources(tmp_path)
    target_release = tmp_path / "target" / "release"
    target_release.mkdir(parents=True)
    for binary_name in current_binaries:
        (target_release / binary_name).write_text(f"fresh {binary_name}", encoding="utf-8")

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)
    monkeypatch.setattr(
        build_plugin,
        "tracked_bundle_files",
        tracked_minimal_bundle_files,
    )

    build_plugin.stage_bundle(bundle_root)

    for binary_name in current_binaries:
        assert (bundle_root / "bin" / binary_name).read_text(encoding="utf-8") == (
            f"fresh {binary_name}"
        )
    for binary_name in other_binaries:
        assert (bundle_root / "bin" / binary_name).read_text(encoding="utf-8") == binary_name


def test_stage_bundle_uses_repo_bins_for_other_platform_on_clean_bundle(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    bundle_root = tmp_path / "dist" / "plugin" / "sky-cua"
    current_binaries = runtime_binary_names()
    other_binaries = [name for name in all_runtime_binary_names() if name not in current_binaries]
    write_minimal_bundle(bundle_root, binaries=[])
    write_minimal_bundle_sources(tmp_path)
    (tmp_path / "target" / "release").mkdir(parents=True)
    for binary_name in current_binaries:
        (tmp_path / "target" / "release" / binary_name).write_text(
            f"fresh {binary_name}",
            encoding="utf-8",
        )
    (tmp_path / "bin").mkdir()
    for binary_name in other_binaries:
        (tmp_path / "bin" / binary_name).write_text(f"repo {binary_name}", encoding="utf-8")

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)
    monkeypatch.setattr(
        build_plugin,
        "tracked_bundle_files",
        tracked_minimal_bundle_files,
    )

    build_plugin.stage_bundle(bundle_root)

    for binary_name in current_binaries:
        assert (bundle_root / "bin" / binary_name).read_text(encoding="utf-8") == (
            f"fresh {binary_name}"
        )
    for binary_name in other_binaries:
        assert (bundle_root / "bin" / binary_name).read_text(encoding="utf-8") == (
            f"repo {binary_name}"
        )


def test_release_install_preserves_existing_other_platform_binaries(tmp_path: Path) -> None:
    marketplace_root = tmp_path / "marketplace"
    source = tmp_path / "bundle"
    destination = release_plugin_root(marketplace_root)
    current_binaries = runtime_binary_names()
    other_binaries = [name for name in all_runtime_binary_names() if name not in current_binaries]
    write_minimal_bundle(source, binaries=current_binaries)
    write_minimal_bundle(destination, binaries=other_binaries)

    installed = release_deploy.install_release_bundle(source, marketplace_root)

    assert installed == destination
    for binary_name in current_binaries:
        assert (destination / "bin" / binary_name).read_text(encoding="utf-8") == binary_name
    for binary_name in other_binaries:
        assert (destination / "bin" / binary_name).read_text(encoding="utf-8") == binary_name


def test_build_release_binaries_retries_windows_sccache_shim_failure(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[dict[str, str] | None] = []

    def fake_run(
        _command: list[str],
        *,
        cwd: Path,
        check: bool,
        env: dict[str, str] | None = None,
        text: bool,
        capture_output: bool,
    ) -> subprocess.CompletedProcess[str]:
        assert _command == build_plugin.CARGO_BUILD_COMMAND
        assert cwd == build_plugin.REPO_ROOT
        assert check is False
        assert text is True
        assert capture_output is True
        calls.append(env)
        if len(calls) == 1:
            return subprocess.CompletedProcess(
                _command,
                1,
                stdout="",
                stderr="Shim: Could not create process with command 'sccache rustc'.",
            )
        return subprocess.CompletedProcess(_command, 0, stdout="built\n", stderr="")

    monkeypatch.setattr(build_plugin.sys, "platform", "win32")
    monkeypatch.setenv("RUSTC_WRAPPER", "sccache")
    monkeypatch.setattr(build_plugin.subprocess, "run", fake_run)

    build_plugin.build_release_binaries()

    assert len(calls) == 2
    assert calls[0] is None
    assert calls[1] is not None
    assert calls[1]["RUSTC_WRAPPER"] == ""
    assert calls[1]["RUSTC_WORKSPACE_WRAPPER"] == ""


def test_build_release_binaries_does_not_retry_unrelated_failure(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls = 0

    def fake_run(
        command: list[str],
        *,
        cwd: Path,
        check: bool,
        env: dict[str, str] | None = None,
        text: bool,
        capture_output: bool,
    ) -> subprocess.CompletedProcess[str]:
        nonlocal calls
        assert capture_output is True
        calls += 1
        return subprocess.CompletedProcess(command, 101, stdout="", stderr="ordinary cargo error")

    monkeypatch.setattr(build_plugin.sys, "platform", "win32")
    monkeypatch.setattr(build_plugin.subprocess, "run", fake_run)

    with pytest.raises(subprocess.CalledProcessError):
        build_plugin.build_release_binaries()

    assert calls == 1


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
    assert manifest["plugins"][0]["policy"] == {
        "installation": "AVAILABLE",
        "authentication": "ON_INSTALL",
    }
    assert manifest["plugins"][0]["category"] == PLUGIN_CATEGORY
    assert release_plugin_root(marketplace_root) == marketplace_root / "plugins" / "sky-cua"
    assert "[marketplaces.Heliasar]" in config
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
                "[marketplaces.Heliasar]",
                'source = "old"',
                'source_type = "local"',
                "",
            ]
        )
    )
    marketplace_root = Path(r"C:\Users\bex\projects\heliasar-marketplace")

    update_codex_config(
        config_path,
        plugin_id=RELEASE_PLUGIN_ID,
        marketplace_root=marketplace_root,
    )
    config = config_path.read_text()

    parsed = tomllib.loads(config)
    assert parsed["marketplaces"]["Heliasar"]["source"] == codex_config_path(marketplace_root)
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


def test_release_deploy_preserves_existing_git_marketplace_source(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    codex_home = tmp_path / "codex-home"
    marketplace_root = tmp_path / "marketplace"
    bundle_root = tmp_path / "bundle"
    config_path = codex_home / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text(
        "\n".join(
            [
                "[marketplaces.Heliasar]",
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


def test_plugin_manifest_tracks_scaffold_metadata_contract() -> None:
    manifest_path = plugin_bundle.REPO_ROOT / ".codex-plugin" / "plugin.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    interface = manifest["interface"]

    assert manifest["name"] == plugin_bundle.PLUGIN_NAME
    assert manifest["mcpServers"] == "./.mcp.json"
    assert manifest["skills"] == "./skills/"
    assert manifest["homepage"].startswith("https://")
    assert manifest["repository"].startswith("https://")
    assert "computer-use" in manifest["keywords"]
    assert interface["category"] == PLUGIN_CATEGORY
    assert interface["capabilities"] == ["Interactive", "Read", "Write"]
    assert interface["websiteURL"].startswith("https://")
    assert interface["privacyPolicyURL"].startswith("https://")
    assert interface["termsOfServiceURL"].startswith("https://")
    assert (plugin_bundle.REPO_ROOT / interface["composerIcon"]).exists()
    assert (plugin_bundle.REPO_ROOT / interface["logo"]).exists()


def test_publish_marketplace_detects_staged_plugin_changes(tmp_path: Path) -> None:
    repo_root = tmp_path / "marketplace"
    plugin_root = repo_root / "plugins" / plugin_bundle.PLUGIN_NAME
    plugin_root.mkdir(parents=True)
    (repo_root / ".agents" / "plugins").mkdir(parents=True)
    (repo_root / ".agents" / "plugins" / "marketplace.json").write_text(
        "{}",
        encoding="utf-8",
    )
    (plugin_root / "README.md").write_text("initial\n", encoding="utf-8")
    subprocess.run(["git", "init", "-b", "main"], cwd=repo_root, check=True)
    subprocess.run(
        ["git", "-c", "user.name=test", "-c", "user.email=test@example.invalid", "add", "."],
        cwd=repo_root,
        check=True,
    )
    subprocess.run(
        [
            "git",
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
        cwd=repo_root,
        check=True,
        stdout=subprocess.DEVNULL,
    )

    (plugin_root / "README.md").write_text("updated\n", encoding="utf-8")

    assert publish_marketplace_release.git_has_head(repo_root)
    assert publish_marketplace_release.git_has_changes(repo_root)


def test_publish_marketplace_preflights_git_repo_before_writing(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    marketplace_root = tmp_path / "marketplace"
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    monkeypatch.setattr(
        sys,
        "argv",
        [
            "publish_marketplace_release.py",
            "--no-build",
            "--skip-codex-install",
            "--no-push",
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
        ],
    )

    with pytest.raises(RuntimeError, match="published git repository"):
        publish_marketplace_release.main()

    assert not marketplace_root.exists()


def test_setup_marketplace_checkout_expands_github_short_source(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(
        command: list[str], *, cwd: Path | None = None, check: bool = True
    ) -> subprocess.CompletedProcess[str]:
        del cwd, check
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(setup_heliasar_marketplace, "run", fake_run)

    setup_heliasar_marketplace.ensure_marketplace_checkout(
        tmp_path / "heliasar-marketplace",
        "becksclair/heliasar-marketplace",
    )

    assert commands == [
        [
            "git",
            "clone",
            "https://github.com/becksclair/heliasar-marketplace.git",
            str(tmp_path / "heliasar-marketplace"),
        ]
    ]
