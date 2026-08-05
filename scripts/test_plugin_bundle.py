"""Tests for plugin bundle build, staging, and browser preflight helpers."""

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

import _chrome_bridge
import _plugin_bundle as plugin_bundle
import _portable_elf as portable_elf
import build_plugin
import live_agent_cursor_kde_smoke
from _plugin_bundle import (
    PLUGIN_ID,
    all_runtime_binary_names,
    bundle_entrypoint_paths,
    compat_plugin_targets_payload,
    current_runtime_platform,
    ensure_apps_feature_disabled,
    ensure_fast_service_tier,
    ensure_plugins_feature_enabled,
    runtime_binary_names,
    runtime_binary_path,
    set_plugin_enabled,
    stop_unix_runtime_processes,
    stop_windows_cache_processes,
    update_codex_config,
    update_plugin_manifest_version,
    version_from_tag,
)
from _test_support import (
    tracked_minimal_bundle_files,
    write_minimal_bundle,
    write_minimal_bundle_sources,
)


def load_chrome_preflight() -> ModuleType:
    module_path = Path(__file__).resolve().parents[1] / "resources" / "chrome_preflight.py"
    spec = importlib.util.spec_from_file_location("chrome_preflight", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def project_browser_client_fixture(client: Path, projection_root: Path) -> None:
    for plugin_name in ("browser-use", "chrome"):
        destination = (
            projection_root
            / "openai-bundled"
            / "plugins"
            / plugin_name
            / "scripts"
            / "browser-client.mjs"
        )
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(client, destination)


def test_codex_config_helpers_update_existing_sections() -> None:
    config = "\n".join(
        [
            'service_tier = "flex"',
            "",
            "[features]",
            "plugins = false",
            "apps = true",
            "",
            '[plugins."sky-cua@local"]',
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
    config = set_plugin_enabled(config, PLUGIN_ID, enabled=True)

    assert 'service_tier = "fast"' in config
    assert "plugins = true" in config
    assert "apps = false" in config
    assert "enabled = true" in config
    assert "[profiles.default]\n" in config
    assert 'profiles.default]\nservice_tier = "flex"' not in config


def test_bundle_entrypoint_paths_always_include_unix_launchers(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(sys, "platform", "win32")

    assert Path("bin/sky-cua-client") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-service") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-overlay-host") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-client.exe") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-service.exe") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-overlay-host.exe") in bundle_entrypoint_paths()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix launcher contract")
def test_unix_launcher_runs_bundled_runtime_from_relocated_bundle(tmp_path: Path) -> None:
    repo_root = Path(__file__).resolve().parents[1]
    bundle_bin = tmp_path / "bundle" / "bin"
    runtime_dir = bundle_bin / "runtimes" / current_runtime_platform()
    runtime_dir.mkdir(parents=True)
    shutil.copy(repo_root / "bin" / "sky-cua-client", bundle_bin / "sky-cua-client")
    fake_runtime = runtime_dir / "sky-cua-client"
    fake_runtime.write_text('#!/bin/sh\necho "bundled-runtime $@"\n', encoding="utf-8")
    fake_runtime.chmod(0o755)

    link_dir = tmp_path / "materialized"
    link_dir.mkdir()
    absolute_link = link_dir / "sky-cua-client"
    absolute_link.symlink_to(bundle_bin / "sky-cua-client")
    chained_link = link_dir / "sky-cua-client-chained"
    chained_link.symlink_to(Path("sky-cua-client"))

    for entrypoint in (bundle_bin / "sky-cua-client", absolute_link, chained_link):
        result = subprocess.run(
            [str(entrypoint), "mcp"],
            cwd=tmp_path,
            capture_output=True,
            text=True,
            check=True,
            timeout=30,
        )
        assert result.stdout.strip() == "bundled-runtime mcp"


def test_remove_path_retries_transient_non_empty_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target = tmp_path / "bundle.tmp"
    target.mkdir()
    (target / "file.txt").write_text("content", encoding="utf-8")
    calls = 0
    real_rmtree = shutil.rmtree

    def flaky_rmtree(path: Path) -> None:
        nonlocal calls
        calls += 1
        if calls == 1:
            raise OSError(plugin_bundle.errno.ENOTEMPTY, "Directory not empty", str(path))
        real_rmtree(path)

    monkeypatch.setattr(plugin_bundle.shutil, "rmtree", flaky_rmtree)
    monkeypatch.setattr(plugin_bundle.time, "sleep", lambda _seconds: None)

    plugin_bundle.remove_path(target)

    assert calls == 2
    assert not target.exists()


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

    overlay_exe = (
        cache_root / "plugin-backup-old" / "sky-cua" / "0.1.0" / "bin" / "sky-cua-overlay-host"
    )
    overlay_proc = proc_root / "790"
    overlay_proc.mkdir()
    (overlay_proc / "cmdline").write_bytes(str(overlay_exe).encode() + b"\0serve")
    (overlay_proc / "exe").symlink_to(overlay_exe)
    (overlay_proc / "cwd").symlink_to(overlay_exe.parent.parent)

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
    assert (790, plugin_bundle.SIGTERM) in calls
    assert all(pid != 456 for pid, _signal in calls)


def test_stop_unix_runtime_processes_match_all_reaps_off_path_zombies(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    if sys.platform == "win32":
        pytest.skip("Unix process cleanup is not used on Windows")

    proc_root = tmp_path / "proc"

    # A stale overlay host from a dev build outside any install root — exactly
    # the zombie a path-scoped stop misses and match_all must reap.
    zombie_exe = "/home/dev/projects/sky-cua/target/release/sky-cua-overlay-host"
    zombie = proc_root / "111"
    zombie.mkdir(parents=True)
    (zombie / "cmdline").write_bytes(zombie_exe.encode() + b"\0serve")
    (zombie / "exe").symlink_to(zombie_exe)
    (zombie / "cwd").symlink_to("/home/dev/projects/sky-cua")

    # A non-sky-cua process must never be signalled.
    other = proc_root / "222"
    other.mkdir()
    (other / "cmdline").write_bytes(b"/usr/bin/firefox\0")
    (other / "exe").symlink_to("/usr/bin/firefox")
    (other / "cwd").symlink_to("/usr/bin")

    calls: list[tuple[int, int]] = []
    terminated: set[int] = set()

    def fake_kill(pid: int, signal: int) -> None:
        calls.append((pid, signal))
        if signal == plugin_bundle.SIGTERM:
            terminated.add(pid)
        if signal == 0 and pid in terminated:
            raise ProcessLookupError

    monkeypatch.setattr(plugin_bundle.os, "kill", fake_kill)
    # The fake /proc entries are owned by the test user, matching getuid().
    monkeypatch.setattr(plugin_bundle.os, "getuid", lambda: zombie.stat().st_uid)

    # No search roots at all: only match_all can find the off-path zombie.
    stop_unix_runtime_processes([], proc_root=proc_root, match_all_paths=True)

    assert (111, plugin_bundle.SIGTERM) in calls
    assert all(pid != 222 for pid, _signal in calls)


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


def test_build_bundle_inputs_omit_tracked_worktree_deletions(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    subprocess.run(["git", "init"], cwd=tmp_path, check=True, stdout=subprocess.DEVNULL)
    tracked = tmp_path / "docs" / "retired.md"
    tracked.parent.mkdir()
    tracked.write_text("retired\n", encoding="utf-8")
    subprocess.run(
        ["git", "add", "docs/retired.md"],
        cwd=tmp_path,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    tracked.unlink()

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)

    assert build_plugin.tracked_bundle_files([Path("docs")]) == []


def test_build_bundle_inputs_fall_back_to_worktree_without_git(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    (tmp_path / "README.md").write_text("readme\n", encoding="utf-8")
    (tmp_path / "docs").mkdir()
    (tmp_path / "docs" / "guide.md").write_text("doc\n", encoding="utf-8")
    (tmp_path / "docs" / "nested").mkdir()
    (tmp_path / "docs" / "nested" / "feature.md").write_text("feature\n", encoding="utf-8")

    def fail_git(*_args: object, **_kwargs: object) -> subprocess.CompletedProcess[bytes]:
        raise subprocess.CalledProcessError(128, ["git", "ls-files"])

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)
    monkeypatch.setattr(build_plugin.subprocess, "run", fail_git)

    assert build_plugin.tracked_bundle_files([Path("README.md"), Path("docs")]) == [
        Path("README.md"),
        Path("docs/guide.md"),
        Path("docs/nested/feature.md"),
    ]


def test_worktree_fallback_excludes_gitignored_bytecode_and_companion_apk(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resources = tmp_path / "resources"
    (resources / "__pycache__").mkdir(parents=True)
    (resources / "__pycache__" / "mod.cpython-313.pyc").write_bytes(b"\x00")
    (resources / "stray.pyc").write_bytes(b"\x00")
    (resources / "kept.toml").write_text("kept\n", encoding="utf-8")
    (resources / "android").mkdir()
    (resources / "android" / "phone-companion.apk").write_bytes(b"\x00")
    (resources / "android" / "phone-companion.json").write_text("{}", encoding="utf-8")

    def fail_git(*_args: object, **_kwargs: object) -> subprocess.CompletedProcess[bytes]:
        raise subprocess.CalledProcessError(128, ["git", "ls-files"])

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)
    monkeypatch.setattr(build_plugin.subprocess, "run", fail_git)

    # The no-git fallback must mirror `git ls-files`: only the tracked source
    # survives. Bytecode caches and the separately-staged companion APK dir are
    # gitignored and must not leak into the bundle.
    assert build_plugin.tracked_bundle_files([Path("resources")]) == [
        Path("resources/kept.toml"),
    ]


def test_bundle_source_paths_include_standard_optional_plugin_roots() -> None:
    assert Path(".claude-plugin") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path(".codex-plugin") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path(".app.json") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path("assets") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path("hooks") in build_plugin.BUNDLE_SOURCE_PATHS
    assert Path("skills") in build_plugin.BUNDLE_SOURCE_PATHS


def test_worktree_bundle_dirs_include_untracked_runtime_resources() -> None:
    assert (
        Path("docs/operations/testing-vm-desktop-smokes.md") in build_plugin.WORKTREE_BUNDLE_FILES
    )
    assert Path(".claude-plugin/plugin.json") in build_plugin.WORKTREE_BUNDLE_FILES
    assert Path(".claude-plugin/marketplace.json") in build_plugin.WORKTREE_BUNDLE_FILES
    assert Path("resources/chrome-extension") in build_plugin.WORKTREE_BUNDLE_DIRS
    assert Path("resources/kwin") in build_plugin.WORKTREE_BUNDLE_DIRS
    assert Path("skills/computer-use") in build_plugin.WORKTREE_BUNDLE_DIRS
    assert Path("skills/browser-use") in build_plugin.WORKTREE_BUNDLE_DIRS


def test_copy_tracked_bundle_sources_rejects_unexpected_missing_files(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setattr(build_plugin, "REPO_ROOT", repo_root)
    monkeypatch.setattr(
        build_plugin,
        "tracked_bundle_files",
        lambda: [
            Path("README.md"),
            Path("skills/computer-use-workflows/SKILL.md"),
        ],
    )

    with pytest.raises(FileNotFoundError, match="tracked bundle source is missing"):
        build_plugin.copy_tracked_bundle_sources(tmp_path / "bundle")


def test_copy_tracked_bundle_sources_allows_retired_skill_paths(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setattr(build_plugin, "REPO_ROOT", repo_root)
    monkeypatch.setattr(
        build_plugin,
        "tracked_bundle_files",
        lambda: [
            Path("skills/computer-use-workflows/SKILL.md"),
            Path("skills/sky-cua-isolated-daemon/SKILL.md"),
            Path("skills/sky-cua-plugin-release/SKILL.md"),
            Path("resources/kwin/effects/sky-cua-agent-cursor/metadata.json"),
            Path("resources/kwin/effects/sky-cua-agent-cursor/qml/main.qml"),
        ],
    )

    build_plugin.copy_tracked_bundle_sources(tmp_path / "bundle")


def test_copy_worktree_bundle_dirs_includes_kwin_effect_resources(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    effect_dir = repo_root / "resources" / "kwin" / "effects" / "sky-cua-agent-cursor"
    effect_dir.mkdir(parents=True)
    (effect_dir / "metadata.json.in").write_text(
        json.dumps(
            {
                "KPackageStructure": "KWin/Effect",
                "KPlugin": {"Id": "@SKY_CUA_EFFECT_ID@"},
            }
        ),
        encoding="utf-8",
    )
    bundle_root = tmp_path / "bundle"

    monkeypatch.setattr(build_plugin, "REPO_ROOT", repo_root)

    build_plugin.copy_worktree_bundle_dirs(bundle_root)

    copied = (
        bundle_root / "resources" / "kwin" / "effects" / "sky-cua-agent-cursor" / "metadata.json.in"
    )
    assert json.loads(copied.read_text(encoding="utf-8"))["KPlugin"]["Id"] == (
        "@SKY_CUA_EFFECT_ID@"
    )


def test_cargo_target_root_honors_shared_absolute_cache(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    shared = tmp_path / "shared-target"
    monkeypatch.setenv("CARGO_TARGET_DIR", str(shared))

    assert build_plugin.cargo_target_root() == shared


def test_copy_worktree_bundle_dirs_excludes_chrome_maps_and_metadata(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    extension_dir = repo_root / "resources" / "chrome-extension" / "codex" / "1.2.0_0"
    assets_dir = extension_dir / "assets"
    metadata_dir = extension_dir / "_metadata"
    assets_dir.mkdir(parents=True)
    metadata_dir.mkdir()
    (extension_dir / "manifest.json").write_text(
        json.dumps({"name": "ChatGPT", "version": "1.2.0", "key": "current-key"}),
        encoding="utf-8",
    )
    older_extension = extension_dir.parent / "1.1.5_0"
    older_extension.mkdir()
    (older_extension / "manifest.json").write_text(
        json.dumps({"name": "Codex", "version": "1.1.5", "key": "old-key"}),
        encoding="utf-8",
    )
    unrelated_extension = extension_dir.parent / "9.0.0_0"
    unrelated_extension.mkdir()
    (unrelated_extension / "manifest.json").write_text(
        json.dumps({"name": "Unrelated", "version": "9.0.0", "key": "foreign-key"}),
        encoding="utf-8",
    )
    mismatched_extension = extension_dir.parent / "9.1.0_0"
    mismatched_extension.mkdir()
    (mismatched_extension / "manifest.json").write_text(
        json.dumps({"name": "ChatGPT", "version": "9.1.1", "key": "mismatched"}),
        encoding="utf-8",
    )
    (assets_dir / "sidepanel.js").write_text("runtime", encoding="utf-8")
    (assets_dir / "sidepanel.js.map").write_text("source map", encoding="utf-8")
    (metadata_dir / "verified_contents.json").write_text("{}", encoding="utf-8")
    bundle_root = tmp_path / "bundle"

    monkeypatch.setattr(build_plugin, "REPO_ROOT", repo_root)

    build_plugin.copy_worktree_bundle_dirs(bundle_root)

    bundled_extension = bundle_root / extension_dir.relative_to(repo_root)
    assert (bundled_extension / "manifest.json").exists()
    assert (bundled_extension / "assets" / "sidepanel.js").exists()
    assert not (bundled_extension / "assets" / "sidepanel.js.map").exists()
    assert not (bundled_extension / "_metadata").exists()
    assert not (bundle_root / older_extension.relative_to(repo_root)).exists()
    assert not (bundle_root / unrelated_extension.relative_to(repo_root)).exists()
    assert not (bundle_root / mismatched_extension.relative_to(repo_root)).exists()
    assert (assets_dir / "sidepanel.js.map").exists()
    assert metadata_dir.exists()


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
    for binary_name in other_binaries:
        if Path(binary_name).name in {"sky-cua-client", "sky-cua-client.exe"}:
            (bundle_root / "bin" / binary_name).with_name(
                Path(binary_name).name + plugin_bundle.BUILD_STAMP_SUFFIX
            ).write_text(
                '{"source_fingerprint":"preserved"}\n',
                encoding="utf-8",
            )
    write_minimal_bundle_sources(tmp_path)
    target_release = tmp_path / "target" / "release"
    target_release.mkdir(parents=True)
    for binary_name in current_binaries:
        binary_path = target_release / binary_name
        binary_path.write_text(f"fresh {binary_name}", encoding="utf-8")
        if binary_name == "sky-cua-client":
            binary_path.with_name(binary_path.name + ".buildstamp.json").write_text(
                '{"source_fingerprint":"fresh"}\n',
                encoding="utf-8",
            )

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)
    monkeypatch.setattr(
        build_plugin,
        "tracked_bundle_files",
        tracked_minimal_bundle_files,
    )

    build_plugin.stage_bundle(bundle_root)

    for binary_name, runtime_path in zip(current_binaries, current_runtime_paths, strict=True):
        assert (bundle_root / runtime_path).read_text(encoding="utf-8") == (f"fresh {binary_name}")
        if binary_name == "sky-cua-client":
            assert (
                bundle_root / runtime_path.with_name(runtime_path.name + ".buildstamp.json")
            ).read_text(encoding="utf-8") == '{"source_fingerprint":"fresh"}\n'
    for binary_name in other_binaries:
        assert (bundle_root / "bin" / binary_name).read_text(encoding="utf-8") == binary_name
        if Path(binary_name).name in {"sky-cua-client", "sky-cua-client.exe"}:
            assert (bundle_root / "bin" / binary_name).with_name(
                Path(binary_name).name + plugin_bundle.BUILD_STAMP_SUFFIX
            ).read_text(encoding="utf-8") == '{"source_fingerprint":"preserved"}\n'


def test_copytree_replace_preserving_platform_binaries_keeps_build_stamp_sidecars(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "source"
    destination_root = tmp_path / "installed"
    source_root.mkdir()
    destination_root.mkdir()
    (source_root / "manifest.json").write_text('{"fresh":true}\n', encoding="utf-8")

    preserved_path = destination_root / plugin_bundle.all_runtime_binary_paths()[0]
    preserved_path.parent.mkdir(parents=True)
    preserved_path.write_text("preserved binary", encoding="utf-8")
    preserved_stamp = preserved_path.with_name(
        preserved_path.name + plugin_bundle.BUILD_STAMP_SUFFIX
    )
    preserved_stamp.write_text('{"source_fingerprint":"preserved"}\n', encoding="utf-8")

    plugin_bundle.copytree_replace_preserving_platform_binaries(source_root, destination_root)

    restored_path = destination_root / plugin_bundle.all_runtime_binary_paths()[0]
    assert (destination_root / "manifest.json").read_text(encoding="utf-8") == '{"fresh":true}\n'
    assert restored_path.read_text(encoding="utf-8") == "preserved binary"
    assert (
        restored_path.with_name(restored_path.name + plugin_bundle.BUILD_STAMP_SUFFIX).read_text(
            encoding="utf-8"
        )
        == '{"source_fingerprint":"preserved"}\n'
    )


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
        if binary_name == "sky-cua-client":
            binary_path.with_name(binary_path.name + ".buildstamp.json").write_text(
                '{"source_fingerprint":"repo"}\n',
                encoding="utf-8",
            )

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
        if binary_name == "sky-cua-client":
            assert (bundle_root / "bin" / f"{binary_name}.buildstamp.json").read_text(
                encoding="utf-8"
            ) == '{"source_fingerprint":"repo"}\n'


def test_merge_runtime_artifacts_copies_build_stamp_sidecars(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    artifacts_root = tmp_path / "artifacts"

    for platform_id in plugin_bundle.REQUIRED_RUNTIME_PLATFORMS:
        platform_root = artifacts_root / platform_id
        platform_root.mkdir(parents=True)
        for binary_name in plugin_bundle.platform_runtime_binary_base_names(platform_id):
            source_name = plugin_bundle.runtime_binary_source_name(platform_id, binary_name)
            source = platform_root / source_name
            source.write_text(f"{platform_id} {binary_name}", encoding="utf-8")
            if binary_name == "sky-cua-client":
                source.with_name(source.name + plugin_bundle.BUILD_STAMP_SUFFIX).write_text(
                    f'{{"source_fingerprint":"{platform_id}"}}\n',
                    encoding="utf-8",
                )

    plugin_bundle.merge_runtime_artifacts(bundle_root, artifacts_root)

    for platform_id in plugin_bundle.REQUIRED_RUNTIME_PLATFORMS:
        client_name = plugin_bundle.runtime_binary_source_name(platform_id, "sky-cua-client")
        client_path = bundle_root / plugin_bundle.runtime_binary_path(platform_id, "sky-cua-client")
        assert client_path.read_text(encoding="utf-8") == f"{platform_id} sky-cua-client"
        assert (
            client_path.with_name(client_name + plugin_bundle.BUILD_STAMP_SUFFIX).read_text(
                encoding="utf-8"
            )
            == f'{{"source_fingerprint":"{platform_id}"}}\n'
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

    canonical_client = tmp_path / "canonical-browser-client.mjs"
    canonical_client.write_text(
        "export async function setupBrowserRuntime() {}\n", encoding="utf-8"
    )

    monkeypatch.setattr(build_plugin, "bundled_resource_root", lambda: source_root)
    monkeypatch.setattr(build_plugin, "build_canonical_browser_client", lambda: canonical_client)
    monkeypatch.setattr(
        build_plugin, "project_canonical_browser_client", project_browser_client_fixture
    )
    monkeypatch.setattr(build_plugin, "install_bundled_chrome_host", lambda _root: None)
    temp_root = tmp_path / "bundle"

    build_plugin.stage_openai_bundled_plugins(temp_root)

    staged = temp_root / "resources" / "node_repl"
    assert staged.exists()
    assert staged.stat().st_mode & 0o111
    assert staged.read_bytes() == Path(true_binary).read_bytes()
    for plugin_name in ("browser-use", "chrome"):
        staged_client = (
            temp_root
            / "resources"
            / "plugins"
            / "openai-bundled"
            / "plugins"
            / plugin_name
            / "scripts"
            / "browser-client.mjs"
        )
        assert staged_client.read_bytes() == canonical_client.read_bytes()


def test_staging_drops_upstream_chrome_native_dependencies(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source_root = tmp_path / "upstream" / "resources" / "plugins" / "openai-bundled"
    marketplace = source_root / ".agents" / "plugins" / "marketplace.json"
    marketplace.parent.mkdir(parents=True)
    marketplace.write_text(json.dumps({"plugins": []}), encoding="utf-8")
    for plugin_name in ("browser-use", "chrome"):
        plugin = source_root / "plugins" / plugin_name
        (plugin / "scripts" / "node_modules" / "classic-level").mkdir(parents=True)
        (plugin / "scripts" / "browser-client.mjs").write_text(
            'import "classic-level";\n', encoding="utf-8"
        )
        (plugin / "scripts" / "node_modules" / "classic-level" / "package.json").write_text(
            json.dumps({"name": "classic-level", "version": "3.0.0"}), encoding="utf-8"
        )

    canonical_client = tmp_path / "canonical-browser-client.mjs"
    canonical_client.write_text(
        "export async function setupBrowserRuntime() {}\n", encoding="utf-8"
    )

    monkeypatch.setattr(build_plugin, "bundled_resource_root", lambda: source_root)
    monkeypatch.setattr(build_plugin, "build_canonical_browser_client", lambda: canonical_client)
    monkeypatch.setattr(
        build_plugin, "project_canonical_browser_client", project_browser_client_fixture
    )
    monkeypatch.setattr(build_plugin, "install_bundled_chrome_host", lambda _root: None)

    temp_root = tmp_path / "bundle"
    build_plugin.stage_openai_bundled_plugins(temp_root)

    for plugin_name in ("browser-use", "chrome"):
        scripts = (
            temp_root
            / "resources"
            / "plugins"
            / "openai-bundled"
            / "plugins"
            / plugin_name
            / "scripts"
        )
        assert (scripts / "browser-client.mjs").read_bytes() == canonical_client.read_bytes()
        assert not (scripts / "node_modules").exists()


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


def test_browser_preflight_update_config_enables_bundled_plugins(tmp_path: Path) -> None:
    chrome_preflight = load_chrome_preflight()
    codex_home = tmp_path / "codex-home"

    # Without a materialized compat root the compat id must stay disabled, or
    # the host would carry an enabled plugin with no working server.
    chrome_preflight.update_codex_config(codex_home)

    parsed = tomllib.loads((codex_home / "config.toml").read_text(encoding="utf-8"))
    assert parsed["features"]["plugins"] is True
    assert parsed["plugins"]["chrome@openai-bundled"]["enabled"] is True
    assert parsed["plugins"]["browser-use@openai-bundled"]["enabled"] is True
    assert parsed["plugins"]["computer-use@openai-bundled"]["enabled"] is False

    # Compat-first: Codex Desktop detects Computer Use plugins by the
    # built-in plugin name, so once the compat root is materialized the
    # compat id is the enabled one.
    compat_latest = codex_home / "plugins" / "cache" / "openai-bundled" / "computer-use" / "latest"
    compat_latest.mkdir(parents=True)
    (compat_latest / ".mcp.json").write_text("{}", encoding="utf-8")
    chrome_preflight.update_codex_config(codex_home)

    parsed = tomllib.loads((codex_home / "config.toml").read_text(encoding="utf-8"))
    assert parsed["plugins"]["computer-use@openai-bundled"]["enabled"] is True


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


def test_browser_preflight_selects_latest_keyed_chatgpt_fallback(tmp_path: Path) -> None:
    chrome_preflight = load_chrome_preflight()
    source_root = tmp_path / "resources" / "plugins" / "openai-bundled"
    source_root.mkdir(parents=True)
    fallback_root = tmp_path / "resources" / "chrome-extension" / "codex"

    manifests = {
        "1.1.5_0": {"name": "Codex", "version": "1.1.5", "key": "old-key"},
        "1.2.0_0": {"name": "ChatGPT", "version": "1.2.0", "key": "new-key"},
        "9.0.0_0": {"name": "Unrelated", "version": "9.0.0", "key": "foreign-key"},
        "9.1.0_0": {"name": "ChatGPT", "version": "9.1.1", "key": "mismatched"},
    }
    for directory, manifest in manifests.items():
        candidate = fallback_root / directory
        candidate.mkdir(parents=True)
        (candidate / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

    assert chrome_preflight.fallback_extension_path(source_root) == fallback_root / "1.2.0_0"


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
    skills_dir = sky_root / "skills" / "computer-use"
    skills_dir.mkdir(parents=True)
    (skills_dir / "SKILL.md").write_text("computer use skill", encoding="utf-8")
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
    # The compat root is the enabled plugin id, so it must carry the payload's
    # skills per docs/runtime/compat-plugin-contract.md.
    compat_skill = compat_mcp_path.parent / "skills" / "computer-use" / "SKILL.md"
    assert compat_skill.read_text(encoding="utf-8") == "computer use skill"

    # A no-op re-sync must not rewrite the materialized root: remove/recreate
    # churn under a running Codex host can reset per-plugin state such as
    # Computer Use app approvals.
    first_sync_stat = compat_mcp_path.stat()
    chrome_preflight.sync_computer_use_compat_plugin(source_root, codex_home)
    second_sync_stat = compat_mcp_path.stat()
    assert second_sync_stat.st_mtime_ns == first_sync_stat.st_mtime_ns
    assert second_sync_stat.st_ino == first_sync_stat.st_ino


def test_compat_plugin_targets_payload_rejects_stale_root(tmp_path: Path) -> None:
    codex_home = tmp_path / "codex-home"
    latest = codex_home / "plugins" / "cache" / "openai-bundled" / "computer-use" / "latest"
    latest.mkdir(parents=True)
    (latest / ".mcp.json").write_text(
        json.dumps(
            {
                "mcpServers": {
                    "computer-use": {
                        "command": str(tmp_path / "old" / "bin" / "sky-cua-client"),
                        "args": ["mcp"],
                    }
                }
            }
        ),
        encoding="utf-8",
    )
    payload = tmp_path / "payload"
    (payload / "bin").mkdir(parents=True)

    assert not compat_plugin_targets_payload(codex_home, payload)

    (latest / ".mcp.json").write_text(
        json.dumps(
            {
                "mcpServers": {
                    "computer-use": {
                        "command": str((payload / "bin" / "sky-cua-client").resolve()),
                        "args": ["mcp"],
                    }
                }
            }
        ),
        encoding="utf-8",
    )

    assert compat_plugin_targets_payload(codex_home, payload)


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


def test_build_release_binaries_retries_windows_sccache_shim_failure(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[dict[str, str] | None] = []
    stamps: list[Path] = []

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
    monkeypatch.setattr(build_plugin, "write_build_stamp", lambda path: stamps.append(path))

    build_plugin.build_release_binaries()

    assert len(calls) == 2
    assert calls[0] is None
    assert calls[1] is not None
    assert calls[1]["RUSTC_WRAPPER"] == ""
    assert calls[1]["RUSTC_WORKSPACE_WRAPPER"] == ""
    assert stamps == [build_plugin.REPO_ROOT / "target" / "release" / "sky-cua-client.exe"]


def test_portable_elf_rejects_gfni(tmp_path: Path) -> None:
    path = tmp_path / "runtime"
    header = bytearray(64)
    header[:4] = b"\x7fELF"
    header[4:6] = b"\x02\x01"
    header[18:20] = (62).to_bytes(2, "little")
    path.write_bytes(header)

    def fake_objdump(*_args: object, **_kwargs: object) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(
            [], 0, stdout="  vgf2p8affineqb %ymm7,%ymm2,%ymm2\n", stderr=""
        )

    with pytest.raises(ValueError, match="vgf2p8affineqb"):
        portable_elf.validate_x86_64_v3_elf(path, runner=fake_objdump)


def test_portable_elf_accepts_x86_64_v3_instructions(tmp_path: Path) -> None:
    path = tmp_path / "runtime"
    header = bytearray(64)
    header[:4] = b"\x7fELF"
    header[4:6] = b"\x02\x01"
    header[18:20] = (62).to_bytes(2, "little")
    path.write_bytes(header)

    def fake_objdump(*_args: object, **_kwargs: object) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess([], 0, stdout="  vpshufb %ymm5,%ymm6,%ymm5\n", stderr="")

    portable_elf.validate_x86_64_v3_elf(path, runner=fake_objdump)


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


def test_codex_config_upsert_updates_crlf_sections_without_duplicate_tables(tmp_path: Path) -> None:
    config_path = tmp_path / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text(
        '[features]\r\nplugins = false\r\n\r\n[plugins."sky-cua@local"]\r\nenabled = false\r\n'
    )

    update_codex_config(config_path)
    config = config_path.read_text()
    parsed = tomllib.loads(config)

    assert config.count("[features]") == 1
    assert config.count('[plugins."sky-cua@local"]') == 1
    assert parsed["features"]["plugins"] is True
    assert parsed["plugins"]["sky-cua@local"]["enabled"] is True


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
    assert interface["category"] == "Coding"
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
    assert "SKY_CUA_AGENT_CURSOR" in env_vars
    assert "SKY_CUA_BROWSER" in env_vars
    assert "SKY_CUA_OVERLAY_BACKEND" in env_vars
    assert "SKY_CUA_INPUT_BACKEND" in env_vars
    assert "SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE" in env_vars
    assert "SKY_CUA_OVERLAY_HOST_PATH" in env_vars
    assert "SKY_CUA_OVERLAY_HOST_TCP_ADDR" in env_vars
    assert "SKY_CUA_PRESENCE_ENABLED" in env_vars
    assert "SKY_CUA_PRESENCE_IDLE_RELEASE_SECS" in env_vars
    assert "SKY_CUA_PRESENCE_INHIBIT_LOCK" in env_vars
    assert "SKY_CUA_PRESENCE_INHIBIT_SUSPEND" in env_vars
    assert "SKY_CUA_PRESENCE_RELOCK" in env_vars
    assert "SKY_CUA_PRESENCE_UNLOCK" in env_vars
    assert "SKY_CUA_SCREENSHOT_CURSOR" in env_vars
    assert "SKY_CUA_REPO_ROOT" in env_vars
    assert "SKY_CUA_SERVICE_PATH" in env_vars
    assert "YDOTOOL_SOCKET" in env_vars


def test_chrome_preflight_default_env_allowlist_matches_primary_mcp_config() -> None:
    chrome_preflight = load_chrome_preflight()
    mcp_config = json.loads((plugin_bundle.REPO_ROOT / ".mcp.json").read_text(encoding="utf-8"))
    env_vars = mcp_config["mcpServers"]["computer-use"]["env_vars"]

    assert env_vars == chrome_preflight.DEFAULT_COMPUTER_USE_ENV_VARS


def test_bundled_chrome_extension_cursor_overlay_contract() -> None:
    extension_dir = _chrome_bridge.FALLBACK_EXTENSION_DIR
    manifest = json.loads((extension_dir / "manifest.json").read_text(encoding="utf-8"))
    content_script = (extension_dir / "content-scripts" / "codex.js").read_text(encoding="utf-8")
    background = (extension_dir / "background.js").read_text(encoding="utf-8")

    assert extension_dir.name == f"{manifest['version']}_0"
    assert (extension_dir / "images" / "cursor-chat.png").exists()
    native_cursor_asset = (
        plugin_bundle.REPO_ROOT / "crates" / "sky-cua-overlay-host" / "assets" / "cursor-chat.png"
    )
    assert (
        native_cursor_asset.read_bytes()
        == (extension_dir / "images" / "cursor-chat.png").read_bytes()
    )
    from PIL import Image

    with Image.open(native_cursor_asset) as cursor_image:
        assert cursor_image.size == (
            live_agent_cursor_kde_smoke.CURSOR_ASSET_SOURCE_WIDTH,
            live_agent_cursor_kde_smoke.CURSOR_ASSET_SOURCE_HEIGHT,
        )
    assert live_agent_cursor_kde_smoke.CURSOR_ASSET_WIDTH == 46
    assert live_agent_cursor_kde_smoke.CURSOR_ASSET_HEIGHT == 48
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
