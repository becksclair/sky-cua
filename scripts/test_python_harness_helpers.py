from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from types import ModuleType
from typing import cast

import pytest

import _plugin_bundle as plugin_bundle
import build_plugin
import deploy_release_plugin as release_deploy
import install_mcp_server
import install_plugin
import live_chrome_host_client_smoke
import live_desktop_smoke
import live_portal_downgrade_smoke
import package_runtime_artifact
import publish_marketplace_release
import setup_heliasar_marketplace
from _app_server_harness import build_schema_accept_value, response_contains_computer_use_server
from _plugin_bundle import (
    PLUGIN_CATEGORY,
    RELEASE_PLUGIN_ID,
    REQUIRED_RUNTIME_PLATFORMS,
    all_runtime_binary_names,
    all_runtime_binary_paths,
    bundle_entrypoint_paths,
    codex_config_path,
    current_runtime_platform,
    ensure_apps_feature_disabled,
    ensure_fast_service_tier,
    ensure_plugin_enabled,
    ensure_plugins_feature_enabled,
    executable_name,
    marketplace_manifest_path,
    merge_runtime_artifacts,
    release_plugin_root,
    runtime_binary_names,
    runtime_binary_path,
    runtime_binary_source_name,
    stop_unix_runtime_processes,
    stop_windows_cache_processes,
    update_codex_config,
    update_plugin_manifest_version,
    version_from_tag,
    write_release_marketplace,
)
from _tidal_workflow import tidal_playlist_prompt
from live_app_server_tidal_image_ab import DEFAULT_VARIANTS, playlist_name_for_variant


def load_chrome_preflight() -> ModuleType:
    module_path = Path(__file__).resolve().parents[1] / "resources" / "chrome_preflight.py"
    spec = importlib.util.spec_from_file_location("chrome_preflight", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
    expected = [f"sky-cua-client{suffix}", f"sky-cua-service{suffix}"]
    if suffix == "":
        expected.append("sky-cua-cosmic-helper")
        expected.append("sky-cua-chrome-host")

    assert runtime_binary_names() == expected


def test_all_runtime_binary_names_include_linux_and_windows_binaries() -> None:
    assert all_runtime_binary_names() == [
        "runtimes/linux-x64/sky-cua-client",
        "runtimes/linux-x64/sky-cua-service",
        "runtimes/linux-x64/sky-cua-cosmic-helper",
        "runtimes/linux-x64/sky-cua-chrome-host",
        "runtimes/linux-arm64/sky-cua-client",
        "runtimes/linux-arm64/sky-cua-service",
        "runtimes/linux-arm64/sky-cua-cosmic-helper",
        "runtimes/linux-arm64/sky-cua-chrome-host",
        "sky-cua-client.exe",
        "sky-cua-service.exe",
    ]


def test_runtime_binary_paths_map_platform_variants() -> None:
    assert runtime_binary_path("linux-x64", "sky-cua-client") == Path(
        "bin/runtimes/linux-x64/sky-cua-client"
    )
    assert runtime_binary_path("linux-arm64", "sky-cua-service") == Path(
        "bin/runtimes/linux-arm64/sky-cua-service"
    )
    assert runtime_binary_path("linux-x64", "sky-cua-cosmic-helper") == Path(
        "bin/runtimes/linux-x64/sky-cua-cosmic-helper"
    )
    assert runtime_binary_path("linux-arm64", "sky-cua-chrome-host") == Path(
        "bin/runtimes/linux-arm64/sky-cua-chrome-host"
    )
    assert runtime_binary_path("windows-x64", "sky-cua-client") == Path("bin/sky-cua-client.exe")


def test_runtime_binary_source_names_reject_invalid_platform_or_binary() -> None:
    with pytest.raises(ValueError, match="unknown runtime platform"):
        runtime_binary_source_name("linux-riscv64", "sky-cua-client")
    with pytest.raises(ValueError, match="unknown runtime binary"):
        runtime_binary_source_name("windows-x64", "sky-cua-cosmic-helper")


def test_bundle_entrypoint_paths_always_include_unix_launchers(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(sys, "platform", "win32")

    assert Path("bin/sky-cua-client") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-service") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-client.exe") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-service.exe") in bundle_entrypoint_paths()


def test_x11_click_target_falls_back_to_native_root_window() -> None:
    snapshot = {
        "snapshot_id": "snapshot-root-only",
        "elements": [
            {
                "bounds": {"height": 52.0, "width": 128.0, "x": 895.0, "y": 526.0},
                "element_index": 0,
                "parent_index": None,
                "role": "window",
                "state_flags": ["native_window_fallback", "physical_target"],
            }
        ],
    }

    target = live_desktop_smoke.pick_x11_click_target(snapshot)

    assert target["element_index"] == 0
    assert live_desktop_smoke.x11_click_arguments(snapshot, target) == {
        "x": 959.0,
        "y": 565.52,
    }


def test_x11_click_target_prefers_lowest_leaf_region_when_available() -> None:
    snapshot = {
        "snapshot_id": "snapshot-with-leaves",
        "elements": [
            {
                "bounds": {"height": 80.0, "width": 160.0, "x": 0.0, "y": 0.0},
                "element_index": 0,
                "parent_index": None,
                "role": "window",
                "state_flags": ["native_window_fallback"],
            },
            {
                "bounds": {"height": 20.0, "width": 80.0, "x": 10.0, "y": 10.0},
                "element_index": 1,
                "parent_index": 0,
                "role": "x11_leaf_region",
            },
            {
                "bounds": {"height": 16.0, "width": 64.0, "x": 20.0, "y": 44.0},
                "element_index": 2,
                "parent_index": 0,
                "role": "x11_action_region",
            },
        ],
    }

    target = live_desktop_smoke.pick_x11_click_target(snapshot)

    assert target["element_index"] == 2
    assert live_desktop_smoke.x11_click_arguments(snapshot, target) == {
        "snapshot_id": "snapshot-with-leaves",
        "element_index": 2,
    }
    live_desktop_smoke.require_x11_action_region_hints(snapshot, "X11")


def test_x11_action_region_hints_reject_root_only_snapshot() -> None:
    snapshot = {
        "snapshot_id": "snapshot-root-only",
        "elements": [
            {
                "bounds": {"height": 52.0, "width": 128.0, "x": 895.0, "y": 526.0},
                "element_index": 0,
                "parent_index": None,
                "role": "window",
                "state_flags": ["native_window_fallback", "physical_target"],
            }
        ],
    }

    with pytest.raises(RuntimeError, match="did not recover any child X11 regions"):
        live_desktop_smoke.require_x11_action_region_hints(snapshot, "X11")


def test_portal_downgrade_accepts_restored_session_diagnostic() -> None:
    diagnostics: list[dict[str, object]] = [
        {"code": "PipeWireStreamFailed"},
        {"code": "CaptureBackendDowngraded"},
        {"code": "PortalSessionRestored"},
    ]

    assert live_portal_downgrade_smoke.has_portal_session_diagnostic(diagnostics)
    assert live_portal_downgrade_smoke.diagnostic_codes(diagnostics) >= {
        "PipeWireStreamFailed",
        "CaptureBackendDowngraded",
    }


def test_portal_downgrade_summary_accepts_restored_session_text() -> None:
    summary = "Reused a persisted RemoteDesktop approval token for the combined portal session."

    assert live_portal_downgrade_smoke.summary_mentions_portal_session(summary)


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

    helper_exe = (
        cache_root / "plugin-backup-old" / "sky-cua" / "0.1.0" / "bin" / "sky-cua-cosmic-helper"
    )
    helper_proc = proc_root / "789"
    helper_proc.mkdir()
    (helper_proc / "cmdline").write_bytes(str(helper_exe).encode() + b"\0")
    (helper_proc / "exe").symlink_to(helper_exe)
    (helper_proc / "cwd").symlink_to(helper_exe.parent.parent)

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
    assert (789, plugin_bundle.SIGTERM) in calls
    assert all(pid != 456 for pid, _signal in calls)


def test_stop_windows_cache_processes_uses_powershell_string_escaping(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    class FakeWindowsPath:
        def resolve(self) -> str:
            return r"C:\Users\O'Brien\sky-cua"

    commands: list[list[str]] = []

    def fake_run(command: list[str], *, check: bool) -> subprocess.CompletedProcess[str]:
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

    monkeypatch.setattr(plugin_bundle.sys, "platform", "win32")
    monkeypatch.setattr(plugin_bundle.subprocess, "run", fake_run)

    stop_windows_cache_processes(cast(Path, FakeWindowsPath()))

    script = commands[0][-1]
    assert "$cacheRoot = 'C:\\Users\\O''Brien\\sky-cua';" in script
    assert "C:\\\\Users" not in script


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


def test_worktree_bundle_dirs_include_untracked_runtime_resources() -> None:
    assert Path("resources/chrome-extension") in build_plugin.WORKTREE_BUNDLE_DIRS
    assert (
        Path("skills/computer-use-workflows/references/apps") in build_plugin.WORKTREE_BUNDLE_DIRS
    )


def write_minimal_bundle(root: Path, *, binaries: list[str]) -> None:
    write_minimal_bundle_sources(root)
    (root / "bin").mkdir(parents=True, exist_ok=True)
    for binary_name in binaries:
        relative_name = binary_name
        if binary_name in runtime_binary_names():
            relative_name = (
                runtime_binary_path(current_runtime_platform(), binary_name.removesuffix(".exe"))
                .as_posix()
                .removeprefix("bin/")
            )
        binary_path = root / "bin" / relative_name
        binary_path.parent.mkdir(parents=True, exist_ok=True)
        binary_path.write_text(binary_name, encoding="utf-8")


def write_minimal_bundle_sources(root: Path) -> None:
    (root / ".codex-plugin").mkdir(parents=True)
    (root / ".codex-plugin" / "plugin.json").write_text(
        json.dumps({"version": "0.1.0"}),
        encoding="utf-8",
    )
    (root / ".mcp.json").write_text("{}", encoding="utf-8")
    (root / "bin").mkdir(parents=True, exist_ok=True)
    (root / "bin" / "sky-cua-client").write_text("#!/bin/sh\n", encoding="utf-8")
    (root / "bin" / "sky-cua-service").write_text("#!/bin/sh\n", encoding="utf-8")
    (root / "bin" / "sky-cua-browser-preflight").write_text("#!/bin/sh\n", encoding="utf-8")
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
        Path("bin/sky-cua-client"),
        Path("bin/sky-cua-service"),
        Path("bin/sky-cua-browser-preflight"),
        Path("skills/computer-use-workflows/SKILL.md"),
        Path("resources/app-instructions/index.json"),
    ]


def test_stage_bundle_preserves_existing_other_platform_binaries(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    bundle_root = tmp_path / "dist" / "plugin" / "sky-cua"
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

    for binary_name, runtime_path in zip(current_binaries, current_runtime_paths, strict=True):
        assert (bundle_root / runtime_path).read_text(encoding="utf-8") == (f"fresh {binary_name}")
    for binary_name in other_binaries:
        assert (bundle_root / "bin" / binary_name).read_text(encoding="utf-8") == binary_name


def test_stage_bundle_uses_repo_bins_for_other_platform_on_clean_bundle(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    bundle_root = tmp_path / "dist" / "plugin" / "sky-cua"
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
    write_minimal_bundle(bundle_root, binaries=[])
    write_minimal_bundle_sources(tmp_path)
    (tmp_path / "target" / "release").mkdir(parents=True)
    for binary_name in current_binaries:
        (tmp_path / "target" / "release" / binary_name).write_text(
            f"fresh {binary_name}",
            encoding="utf-8",
        )
    (tmp_path / "bin").mkdir(exist_ok=True)
    for binary_name in other_binaries:
        binary_path = tmp_path / "bin" / binary_name
        binary_path.parent.mkdir(parents=True, exist_ok=True)
        binary_path.write_text(f"repo {binary_name}", encoding="utf-8")

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)
    monkeypatch.setattr(
        build_plugin,
        "tracked_bundle_files",
        tracked_minimal_bundle_files,
    )

    build_plugin.stage_bundle(bundle_root)

    for binary_name, runtime_path in zip(current_binaries, current_runtime_paths, strict=True):
        assert (bundle_root / runtime_path).read_text(encoding="utf-8") == (f"fresh {binary_name}")
    for binary_name in other_binaries:
        assert (bundle_root / "bin" / binary_name).read_text(encoding="utf-8") == (
            f"repo {binary_name}"
        )


def test_browser_preflight_links_browser_use_into_bundled_marketplace(tmp_path: Path) -> None:
    chrome_preflight = load_chrome_preflight()
    source_plugin = tmp_path / "source" / "plugins" / "openai-bundled" / "plugins" / "browser-use"
    (source_plugin / ".codex-plugin").mkdir(parents=True)
    (source_plugin / ".codex-plugin" / "plugin.json").write_text(
        json.dumps({"version": "1.2.3"}),
        encoding="utf-8",
    )
    (source_plugin / "scripts").mkdir()
    (source_plugin / "scripts" / "browser-client.mjs").write_text(
        "client",
        encoding="utf-8",
    )
    codex_home = tmp_path / "codex-home"

    chrome_preflight.sync_browser_use_plugin(source_plugin.parents[1], codex_home)

    cache_root = codex_home / "plugins" / "cache" / "openai-bundled" / "browser-use"
    assert (cache_root / "latest").readlink() == Path("1.2.3")
    plugin_link = (
        codex_home / ".tmp" / "bundled-marketplaces" / "openai-bundled" / "plugins" / "browser-use"
    )
    assert plugin_link.readlink() == cache_root / "latest"


def test_browser_preflight_replaces_read_only_cached_plugin_tree(tmp_path: Path) -> None:
    chrome_preflight = load_chrome_preflight()
    source = tmp_path / "source"
    destination = tmp_path / "destination"
    source.mkdir()
    (source / "fresh.txt").write_text("fresh", encoding="utf-8")
    destination.mkdir()
    (destination / "stale.txt").write_text("stale", encoding="utf-8")
    destination.chmod(0o500)

    try:
        chrome_preflight.copytree_replace(source, destination)
    finally:
        destination.chmod(0o700)

    assert (destination / "fresh.txt").read_text(encoding="utf-8") == "fresh"
    assert not (destination / "stale.txt").exists()


def test_browser_use_node_repl_is_staged_from_upstream_resources(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    true_binary = shutil.which("true")
    if true_binary is None:
        pytest.skip("true binary is not available")

    source_root = tmp_path / "upstream" / "resources" / "plugins" / "openai-bundled"
    marketplace = source_root / ".agents" / "plugins" / "marketplace.json"
    marketplace.parent.mkdir(parents=True)
    marketplace.write_text(json.dumps({"plugins": []}), encoding="utf-8")
    for plugin_name in ("browser-use", "chrome"):
        plugin = source_root / "plugins" / plugin_name
        (plugin / ".codex-plugin").mkdir(parents=True)
        (plugin / ".codex-plugin" / "plugin.json").write_text(
            json.dumps({"name": plugin_name, "version": "1.0.0"}),
            encoding="utf-8",
        )
        (plugin / "scripts").mkdir()
        (plugin / "scripts" / "browser-client.mjs").write_text("client", encoding="utf-8")
    shutil.copy2(true_binary, source_root.parents[1] / "node_repl")

    monkeypatch.setattr(build_plugin, "bundled_resource_root", lambda: source_root)
    monkeypatch.setattr(build_plugin, "install_bundled_chrome_host", lambda _root: None)
    temp_root = tmp_path / "bundle"

    build_plugin.stage_openai_bundled_plugins(temp_root)

    staged = temp_root / "resources" / "node_repl"
    assert staged.exists()
    assert staged.stat().st_mode & 0o111
    assert staged.read_bytes() == Path(true_binary).read_bytes()


def test_browser_use_node_repl_installer_rejects_incompatible_ldd(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = tmp_path / "node_repl"
    destination = tmp_path / "out" / "node_repl"
    source.write_text("fake", encoding="utf-8")

    monkeypatch.setattr(build_plugin, "node_repl_ldd_compatible", lambda _path: False)

    assert not build_plugin.install_browser_use_node_repl(source, destination)
    assert not destination.exists()


def test_browser_use_node_repl_non_elf_patch_is_noop(tmp_path: Path) -> None:
    chrome_preflight = load_chrome_preflight()
    node_repl = tmp_path / "node_repl"
    node_repl.write_bytes(b"not an elf")

    assert chrome_preflight.patch_browser_use_node_repl_glibc_pidfd_symbols(node_repl) is False
    assert node_repl.read_bytes() == b"not an elf"


def test_browser_preflight_validates_node_repl_without_installing_to_codex_home(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    chrome_preflight = load_chrome_preflight()
    source_root = tmp_path / "upstream" / "resources" / "plugins" / "openai-bundled"
    source_root.mkdir(parents=True)
    source_node_repl = source_root.parents[1] / "node_repl"
    source_node_repl.write_text("fake", encoding="utf-8")
    calls: list[tuple[Path, Path]] = []

    def fake_install(source: Path, destination: Path) -> bool:
        calls.append((source, destination))
        return True

    monkeypatch.setattr(chrome_preflight, "install_browser_use_node_repl", fake_install)

    chrome_preflight.validate_browser_use_node_repl(source_root)

    assert calls
    assert calls[0][0] == source_node_repl
    assert calls[0][1].name == "node_repl"


def test_browser_preflight_adds_coupled_plugins_to_marketplace(tmp_path: Path) -> None:
    chrome_preflight = load_chrome_preflight()
    marketplace_path = (
        tmp_path
        / "codex-home"
        / ".tmp"
        / "bundled-marketplaces"
        / "openai-bundled"
        / ".agents"
        / "plugins"
        / "marketplace.json"
    )
    marketplace_path.parent.mkdir(parents=True)
    marketplace_path.write_text(
        json.dumps({"name": "openai-bundled", "plugins": [{"name": "chrome"}]}),
        encoding="utf-8",
    )

    chrome_preflight.ensure_marketplace_entries(tmp_path / "codex-home")

    manifest = json.loads(marketplace_path.read_text(encoding="utf-8"))
    names = {plugin["name"] for plugin in manifest["plugins"]}
    assert {"chrome", "browser-use", "computer-use"} <= names
    computer_use = next(
        plugin for plugin in manifest["plugins"] if plugin["name"] == "computer-use"
    )
    assert computer_use["source"]["path"] == "./plugins/computer-use"


def test_browser_preflight_update_config_enables_browser_plugins_only(tmp_path: Path) -> None:
    chrome_preflight = load_chrome_preflight()
    codex_home = tmp_path / "codex-home"

    chrome_preflight.update_codex_config(codex_home)

    parsed = tomllib.loads((codex_home / "config.toml").read_text(encoding="utf-8"))
    assert parsed["features"]["plugins"] is True
    assert parsed["plugins"]["chrome@openai-bundled"]["enabled"] is True
    assert parsed["plugins"]["browser-use@openai-bundled"]["enabled"] is True
    assert parsed["plugins"]["computer-use@openai-bundled"]["enabled"] is False


def test_browser_preflight_rejects_uppercase_native_host_name(tmp_path: Path) -> None:
    chrome_preflight = load_chrome_preflight()
    plugin_root = tmp_path / "chrome"
    scripts = plugin_root / "scripts"
    scripts.mkdir(parents=True)
    (scripts / "extension-id.json").write_text(
        json.dumps(
            {
                "extensionId": "abcdefghijklmnopabcdefghijklmnop",
                "extensionHostName": "Com.OpenAI.CodexExtension",
            }
        ),
        encoding="utf-8",
    )

    with pytest.raises(RuntimeError, match="invalid Chrome native host name"):
        chrome_preflight.read_chrome_extension_metadata(plugin_root)


def test_browser_preflight_computer_use_compat_plugin_preserves_env_allowlist(
    tmp_path: Path,
) -> None:
    chrome_preflight = load_chrome_preflight()
    sky_root = tmp_path / "sky-cua"
    source_root = sky_root / "resources" / "plugins" / "openai-bundled"
    source_root.mkdir(parents=True)
    (sky_root / ".codex-plugin").mkdir()
    (sky_root / ".codex-plugin" / "plugin.json").write_text(
        json.dumps({"version": "1.2.3"}),
        encoding="utf-8",
    )
    (sky_root / ".mcp.json").write_text(
        json.dumps(
            {
                "mcpServers": {
                    "computer-use": {
                        "command": "./bin/sky-cua-client",
                        "args": ["mcp"],
                        "env_vars": ["DISPLAY", "SKY_CUA_COSMIC_HELPER"],
                        "cwd": ".",
                    }
                }
            }
        ),
        encoding="utf-8",
    )
    codex_home = tmp_path / "codex-home"

    chrome_preflight.sync_computer_use_compat_plugin(source_root, codex_home)

    compat_mcp_path = (
        codex_home
        / "plugins"
        / "cache"
        / "openai-bundled"
        / "computer-use"
        / "1.2.3-sky-cua"
        / ".mcp.json"
    )
    cache_root = compat_mcp_path.parents[1]
    compat_mcp = json.loads(compat_mcp_path.read_text(encoding="utf-8"))
    server = compat_mcp["mcpServers"]["computer-use"]
    assert server["env_vars"] == ["DISPLAY", "SKY_CUA_COSMIC_HELPER"]
    assert server["command"] == str((sky_root / "bin" / "sky-cua-client").resolve())
    assert server["cwd"] == str(sky_root.resolve())
    assert (cache_root / "latest").readlink() == Path("1.2.3-sky-cua")
    assert (
        codex_home / ".tmp" / "bundled-marketplaces" / "openai-bundled" / "plugins" / "computer-use"
    ).readlink() == cache_root / "latest"


def test_bundled_resource_root_accepts_upstream_codex_resource_root(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    upstream_root = tmp_path / "codex-app" / "resources"
    bundled_root = upstream_root / "plugins" / "openai-bundled"
    bundled_root.mkdir(parents=True)
    monkeypatch.setenv("SKY_CUA_UPSTREAM_CODEX_RESOURCES", str(upstream_root))
    monkeypatch.delenv("SKY_CUA_OPENAI_BUNDLED_RESOURCE_ROOT", raising=False)

    assert build_plugin.bundled_resource_root() == bundled_root


def test_bundled_resource_root_accepts_legacy_openai_bundled_root(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    bundled_root = tmp_path / "openai-bundled"
    monkeypatch.delenv("SKY_CUA_UPSTREAM_CODEX_RESOURCES", raising=False)
    monkeypatch.setenv("SKY_CUA_OPENAI_BUNDLED_RESOURCE_ROOT", str(bundled_root))

    assert build_plugin.bundled_resource_root() == bundled_root


def test_build_stage_marketplace_entries_include_coupled_plugins(tmp_path: Path) -> None:
    marketplace_path = tmp_path / "marketplace.json"
    marketplace_path.write_text(
        json.dumps({"name": "openai-bundled", "plugins": [{"name": "chrome"}]}),
        encoding="utf-8",
    )

    build_plugin.ensure_openai_bundled_marketplace_entries(marketplace_path)

    manifest = json.loads(marketplace_path.read_text(encoding="utf-8"))
    names = {plugin["name"] for plugin in manifest["plugins"]}
    assert {"chrome", "browser-use", "computer-use"} <= names
    browser_use = next(plugin for plugin in manifest["plugins"] if plugin["name"] == "browser-use")
    assert browser_use["source"]["path"] == "./plugins/browser-use"


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


def test_merge_runtime_artifacts_requires_all_platforms(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    artifacts_root = tmp_path / "artifacts"
    write_minimal_bundle_sources(bundle_root)
    for platform_id in REQUIRED_RUNTIME_PLATFORMS:
        platform_root = artifacts_root / platform_id
        platform_root.mkdir(parents=True)
        for binary_name in plugin_bundle.platform_runtime_binary_base_names(platform_id):
            source_name = runtime_binary_source_name(platform_id, binary_name)
            (platform_root / source_name).write_text(
                f"{platform_id}/{source_name}",
                encoding="utf-8",
            )

    merge_runtime_artifacts(bundle_root, artifacts_root)

    for relative_path in all_runtime_binary_paths():
        assert (bundle_root / relative_path).exists()
    linux_x64_host_path = plugin_bundle.chrome_extension_host_path("linux-x64")
    linux_arm64_host_path = plugin_bundle.chrome_extension_host_path("linux-arm64")
    assert linux_x64_host_path is not None
    assert linux_arm64_host_path is not None
    assert (bundle_root / linux_x64_host_path).read_text(
        encoding="utf-8"
    ) == "linux-x64/sky-cua-chrome-host"
    assert (bundle_root / linux_arm64_host_path).read_text(
        encoding="utf-8"
    ) == "linux-arm64/sky-cua-chrome-host"


def test_merge_runtime_artifacts_fails_when_variant_is_missing(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    artifacts_root = tmp_path / "artifacts"
    write_minimal_bundle_sources(bundle_root)
    for platform_id in REQUIRED_RUNTIME_PLATFORMS:
        platform_root = artifacts_root / platform_id
        platform_root.mkdir(parents=True)
        for binary_name in plugin_bundle.platform_runtime_binary_base_names(platform_id):
            if platform_id == "linux-arm64" and binary_name == "sky-cua-service":
                continue
            (platform_root / runtime_binary_source_name(platform_id, binary_name)).write_text(
                "binary",
                encoding="utf-8",
            )

    with pytest.raises(FileNotFoundError, match="linux-arm64"):
        merge_runtime_artifacts(bundle_root, artifacts_root)


def test_package_runtime_artifact_uses_platform_binary_contract(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    release_root = repo_root / "target" / "release"
    release_root.mkdir(parents=True)
    output_root = tmp_path / "artifacts"
    stale_linux_root = output_root / "linux-x64"
    stale_linux_root.mkdir(parents=True)
    (stale_linux_root / "stale-binary").write_text("stale", encoding="utf-8")

    for binary_name in plugin_bundle.platform_runtime_binary_base_names("linux-x64"):
        (release_root / runtime_binary_source_name("linux-x64", binary_name)).write_text(
            binary_name,
            encoding="utf-8",
        )
    (release_root / "sky-cua-cosmic-helper.exe").write_text(
        "windows should not package helper",
        encoding="utf-8",
    )

    monkeypatch.setattr(package_runtime_artifact, "REPO_ROOT", repo_root)

    linux_root = package_runtime_artifact.package_runtime_artifact("linux-x64", output_root)

    assert sorted(path.name for path in linux_root.iterdir()) == [
        "sky-cua-chrome-host",
        "sky-cua-client",
        "sky-cua-cosmic-helper",
        "sky-cua-service",
    ]
    assert not (linux_root / "stale-binary").exists()

    for binary_name in plugin_bundle.platform_runtime_binary_base_names("windows-x64"):
        (release_root / runtime_binary_source_name("windows-x64", binary_name)).write_text(
            binary_name,
            encoding="utf-8",
        )

    windows_root = package_runtime_artifact.package_runtime_artifact("windows-x64", output_root)

    assert sorted(path.name for path in windows_root.iterdir()) == [
        "sky-cua-client.exe",
        "sky-cua-service.exe",
    ]


def test_package_runtime_artifact_rejects_invalid_platform_before_cleanup(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    output_root = tmp_path / "artifacts"
    escaped = tmp_path / "escaped"
    escaped.mkdir()
    sentinel = escaped / "sentinel"
    sentinel.write_text("keep", encoding="utf-8")
    monkeypatch.setattr(package_runtime_artifact, "REPO_ROOT", repo_root)

    with pytest.raises(ValueError, match="unknown runtime platform"):
        package_runtime_artifact.package_runtime_artifact("../escaped", output_root)

    assert sentinel.read_text(encoding="utf-8") == "keep"


def test_install_bundle_uses_runtime_binary_paths(tmp_path: Path) -> None:
    source = tmp_path / "source"
    destination = tmp_path / "installed"
    write_minimal_bundle(source, binaries=runtime_binary_names())

    install_plugin.install_bundle(source, destination, symlink=False)

    platform_id = current_runtime_platform()
    for binary_name in plugin_bundle.platform_runtime_binary_base_names(platform_id):
        binary_path = destination / runtime_binary_path(platform_id, binary_name)
        assert binary_path.exists()
        if not binary_path.name.endswith(".exe"):
            assert binary_path.stat().st_mode & 0o111


def test_install_plugin_skips_browser_preflight_on_non_linux(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    destination = tmp_path / "installed"
    preflight = destination / "resources" / "chrome_preflight.py"
    preflight.parent.mkdir(parents=True)
    preflight.write_text("raise SystemExit(99)", encoding="utf-8")
    calls: list[list[str]] = []

    def fake_run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(install_plugin.sys, "platform", "win32")
    monkeypatch.setattr(install_plugin.subprocess, "run", fake_run)

    install_plugin.run_browser_preflight(destination, tmp_path / "codex-home")

    assert calls == []


def test_generic_mcp_install_copies_all_current_platform_binaries(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    release_root = repo_root / "target" / "release"
    release_root.mkdir(parents=True)
    target_dir = tmp_path / "installed"
    platform_id = install_mcp_server.current_platform()

    for binary_name in install_mcp_server.platform_runtime_binary_base_names(platform_id):
        source_name = runtime_binary_source_name(platform_id, binary_name)
        (release_root / source_name).write_text(binary_name, encoding="utf-8")

    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    client_path = install_mcp_server.install_binaries(target_dir)

    assert client_path == target_dir / install_mcp_server.entrypoint_path(
        platform_id, "sky-cua-client"
    )
    for binary_name in install_mcp_server.platform_runtime_binary_base_names(platform_id):
        binary_path = target_dir / install_mcp_server.entrypoint_path(platform_id, binary_name)
        assert binary_path.exists()
        if not binary_path.name.endswith(".exe"):
            assert binary_path.stat().st_mode & 0o111


def test_generic_mcp_bin_links_use_platform_entrypoint_names(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    bin_dir = tmp_path / "bin"
    for name in install_mcp_server.platform_runtime_binary_base_names("windows-x64"):
        binary = target_dir / install_mcp_server.entrypoint_path("windows-x64", name)
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_text(name, encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "current_platform", lambda: "windows-x64")

    install_mcp_server.link_current_platform_binaries(target_dir, bin_dir)

    assert (bin_dir / "sky-cua-client.exe").readlink() == target_dir / "bin" / "sky-cua-client.exe"
    assert (
        bin_dir / "sky-cua-service.exe"
    ).readlink() == target_dir / "bin" / "sky-cua-service.exe"
    assert not (bin_dir / "sky-cua-client").exists()


def test_generic_mcp_bin_links_copy_when_symlinks_are_unavailable(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    bin_dir = tmp_path / "bin"
    binary = target_dir / install_mcp_server.entrypoint_path("windows-x64", "sky-cua-client")
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text("client", encoding="utf-8")
    service = target_dir / install_mcp_server.entrypoint_path("windows-x64", "sky-cua-service")
    service.write_text("service", encoding="utf-8")

    def fake_symlink_to(self: Path, target: Path) -> None:
        _ = self, target
        raise OSError("symlinks unavailable")

    monkeypatch.setattr(install_mcp_server, "current_platform", lambda: "windows-x64")
    monkeypatch.setattr(Path, "symlink_to", fake_symlink_to)

    install_mcp_server.link_current_platform_binaries(target_dir, bin_dir)

    assert (bin_dir / "sky-cua-client.exe").read_text(encoding="utf-8") == "client"
    assert (bin_dir / "sky-cua-service.exe").read_text(encoding="utf-8") == "service"


def test_version_from_tag_updates_plugin_manifest(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle_sources(bundle_root)

    version = version_from_tag("v1.2.3")
    update_plugin_manifest_version(bundle_root, version)

    manifest = json.loads((bundle_root / ".codex-plugin" / "plugin.json").read_text())
    assert manifest["version"] == "1.2.3"
    with pytest.raises(ValueError, match=r"vX\.Y\.Z"):
        version_from_tag("release-1.2.3")
    with pytest.raises(ValueError, match=r"vX\.Y\.Z"):
        version_from_tag("v1.2.3-rc.1")


def test_current_tag_normalizes_full_tag_ref(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GITHUB_REF_NAME", "refs/tags/v1.2.3")

    assert publish_marketplace_release.current_tag() == "v1.2.3"


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
        _ = command, cwd, check, env, text
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


def test_mcp_config_allows_runtime_override_env_vars() -> None:
    mcp_config = json.loads((plugin_bundle.REPO_ROOT / ".mcp.json").read_text(encoding="utf-8"))
    env_vars = set(mcp_config["mcpServers"]["computer-use"]["env_vars"])

    assert "SKY_CUA_COSMIC_HELPER" in env_vars
    assert "CODEX_COMPUTER_USE_COSMIC_HELPER" in env_vars
    assert "SKY_CUA_REPO_ROOT" in env_vars
    assert "SKY_CUA_SERVICE_PATH" in env_vars


def test_chrome_preflight_default_env_allowlist_matches_primary_mcp_config() -> None:
    chrome_preflight = load_chrome_preflight()
    mcp_config = json.loads((plugin_bundle.REPO_ROOT / ".mcp.json").read_text(encoding="utf-8"))
    env_vars = mcp_config["mcpServers"]["computer-use"]["env_vars"]

    assert env_vars == chrome_preflight.DEFAULT_COMPUTER_USE_ENV_VARS


def test_bundled_chrome_extension_cursor_overlay_contract() -> None:
    extension_dir = live_chrome_host_client_smoke.FALLBACK_EXTENSION_DIR
    manifest = json.loads((extension_dir / "manifest.json").read_text(encoding="utf-8"))
    content_script = (extension_dir / "content-scripts" / "codex.js").read_text(encoding="utf-8")
    background = (extension_dir / "background.js").read_text(encoding="utf-8")

    assert (extension_dir / "images" / "cursor-chat.png").exists()
    assert any(
        "images/cursor-chat.png" in entry.get("resources", [])
        for entry in manifest["web_accessible_resources"]
    )
    assert "codex-agent-overlay" in content_script
    assert "pointer-events:none" in content_script
    assert "images/cursor-chat.png" in content_script
    assert "AGENT_CURSOR_STATE" in content_script
    assert "GET_AGENT_CURSOR_STATE" in content_script
    assert "AGENT_CURSOR_ARRIVED" in content_script
    assert "async moveMouse" in background
    assert "waitForArrival" in background
    assert "createCursorArrivalWaiter" in background
    assert "AGENT_CURSOR_ARRIVED" in background


def write_cursor_diff_fixture(path: Path, points: list[tuple[int, int]]) -> None:
    from PIL import Image, ImageDraw

    image = Image.new("RGB", (400, 300), "white")
    draw = ImageDraw.Draw(image)
    for x, y in points:
        draw.rectangle((x - 4, y - 4, x + 4, y + 4), fill="black")
    image.save(path)


def write_cursor_rectangle_fixture(path: Path, rectangles: list[tuple[int, int, int, int]]) -> None:
    from PIL import Image, ImageDraw

    image = Image.new("RGB", (400, 300), "white")
    draw = ImageDraw.Draw(image)
    for x, y, width, height in rectangles:
        draw.rectangle((x, y, x + width - 1, y + height - 1), fill="black")
    image.save(path)


def test_cursor_diff_accepts_localized_cursor_change(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    write_cursor_diff_fixture(after, [(100, 80)])

    result = live_chrome_host_client_smoke.assert_localized_cursor_diff(
        before,
        after,
        target_x_css=100,
        target_y_css=80,
        device_pixel_ratio=1,
    )

    assert result["ok"] is True
    assert result["near_changed_pixels"] == 81


def test_cursor_diff_accepts_compact_prior_cursor_disappearing(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [(280, 220)])
    write_cursor_diff_fixture(after, [(100, 80)])

    result = live_chrome_host_client_smoke.assert_localized_cursor_diff(
        before,
        after,
        target_x_css=100,
        target_y_css=80,
        device_pixel_ratio=1,
    )

    assert result["ok"] is True
    assert result["near_changed_pixels"] == 81
    assert result["outside_changed_pixels"] == 81


def test_cursor_diff_accepts_full_size_prior_cursor_disappearing(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_rectangle_fixture(before, [(260, 210, 46, 48)])
    write_cursor_diff_fixture(after, [(100, 80)])

    result = live_chrome_host_client_smoke.assert_localized_cursor_diff(
        before,
        after,
        target_x_css=100,
        target_y_css=80,
        device_pixel_ratio=1,
    )

    assert result["ok"] is True
    assert result["near_changed_pixels"] == 81
    assert result["outside_changed_pixels"] == 2208


def test_cursor_diff_rejects_missing_visible_change(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    write_cursor_diff_fixture(after, [])

    with pytest.raises(AssertionError, match="enough changed pixels"):
        live_chrome_host_client_smoke.assert_localized_cursor_diff(
            before,
            after,
            target_x_css=100,
            target_y_css=80,
            device_pixel_ratio=1,
        )


def test_cursor_diff_rejects_far_away_change(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    write_cursor_diff_fixture(after, [(320, 240)])

    with pytest.raises(AssertionError, match="enough changed pixels"):
        live_chrome_host_client_smoke.assert_localized_cursor_diff(
            before,
            after,
            target_x_css=100,
            target_y_css=80,
            device_pixel_ratio=1,
        )


def test_cursor_diff_rejects_broad_unrelated_change(tmp_path: Path) -> None:
    from PIL import Image, ImageDraw

    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    image = Image.new("RGB", (400, 300), "white")
    draw = ImageDraw.Draw(image)
    draw.rectangle((96, 76, 104, 84), fill="black")
    draw.rectangle((250, 20, 390, 260), fill="black")
    image.save(after)

    with pytest.raises(AssertionError, match="outside the cursor region"):
        live_chrome_host_client_smoke.assert_localized_cursor_diff(
            before,
            after,
            target_x_css=100,
            target_y_css=80,
            device_pixel_ratio=1,
        )


def test_cursor_diff_scales_css_coordinates_by_device_pixel_ratio(tmp_path: Path) -> None:
    before = tmp_path / "before.png"
    after = tmp_path / "after.png"
    write_cursor_diff_fixture(before, [])
    write_cursor_diff_fixture(after, [(200, 160)])

    result = live_chrome_host_client_smoke.assert_localized_cursor_diff(
        before,
        after,
        target_x_css=100,
        target_y_css=80,
        device_pixel_ratio=2,
    )

    assert result["ok"] is True
    assert result["target_pixel"] == {"x": 200, "y": 160}


def test_chrome_host_smoke_accepts_same_origin_web_redirects() -> None:
    assert live_chrome_host_client_smoke.same_requested_origin(
        "http://www.example.com/article",
        "https://example.com/article",
    )


def test_chrome_host_smoke_rejects_unexpected_navigation_origin() -> None:
    assert not live_chrome_host_client_smoke.same_requested_origin(
        "https://example.com/article",
        "chrome-error://chromewebdata/",
    )
    assert not live_chrome_host_client_smoke.same_requested_origin(
        "https://example.com/article",
        "https://other.example.com/article",
    )


def test_chrome_host_smoke_accepts_successful_turn_ended_response() -> None:
    stderr = (
        "[com.openai.codexextension] received unmatched Chrome response "
        "id=native-turn-ended:smoke-session:smoke-turn "
        'payload={"jsonrpc":"2.0","id":"native-turn-ended:smoke-session:smoke-turn"}'
    )

    response = live_chrome_host_client_smoke.turn_ended_response_from_stderr(stderr)

    assert live_chrome_host_client_smoke.turn_ended_response_was_successful(response)


def test_chrome_host_smoke_rejects_turn_ended_error_response() -> None:
    stderr = (
        "[com.openai.codexextension] received unmatched Chrome response "
        "id=native-turn-ended:smoke-session:smoke-turn "
        'payload={"jsonrpc":"2.0","id":"native-turn-ended:smoke-session:smoke-turn",'
        '"error":{"code":-32601,"message":"No handler registered for method"}}'
    )

    response = live_chrome_host_client_smoke.turn_ended_response_from_stderr(stderr)

    assert response is not None
    assert not live_chrome_host_client_smoke.turn_ended_response_was_successful(response)


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
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

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
