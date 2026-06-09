from __future__ import annotations

import importlib.util
import json
import shutil
import socket
import stat
import subprocess
import sys
import threading
import time
import tomllib
from pathlib import Path
from types import ModuleType
from typing import cast

import pytest

import _mcp_stdio
import _plugin_bundle as plugin_bundle
import build_plugin
import build_runtime_packages
import deploy_release_plugin as release_deploy
import install_mcp_server
import install_plugin
import live_agent_cursor_kde_smoke
import live_agent_cursor_x11_overlay_smoke
import live_chrome_host_client_smoke
import live_desktop_smoke
import live_portal_downgrade_smoke
import live_wayland_pointer_smoke
import package_runtime_artifact
import publish_marketplace_release
import run_gui_testing_vm_smoke
import setup_heliasar_marketplace
from _app_server_harness import build_schema_accept_value, response_contains_computer_use_server
from _codex_exec import DEFAULT_MODEL, DEFAULT_REASONING_EFFORT
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
from _pointer_geometry import adjusted_origin_for_visible_monitor
from _smoke_config import LIVE_SMOKE_MODEL, LIVE_SMOKE_REASONING_EFFORT


def load_chrome_preflight() -> ModuleType:
    module_path = Path(__file__).resolve().parents[1] / "resources" / "chrome_preflight.py"
    spec = importlib.util.spec_from_file_location("chrome_preflight", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_live_smoke_model_config_is_centralized() -> None:
    assert LIVE_SMOKE_MODEL == "gpt-5.5"
    assert LIVE_SMOKE_REASONING_EFFORT == "low"
    assert DEFAULT_MODEL == LIVE_SMOKE_MODEL
    assert DEFAULT_REASONING_EFFORT == LIVE_SMOKE_REASONING_EFFORT


def test_pointer_fixture_adjusts_origin_when_fullscreen_allocation_is_clipped() -> None:
    assert adjusted_origin_for_visible_monitor(
        origin_x=0,
        origin_y=0,
        allocation_width=1280,
        allocation_height=955,
        monitor_width=1280,
        monitor_height=800,
    ) == (0, -78)


def test_pointer_fixture_keeps_origin_when_allocation_fits_monitor() -> None:
    assert adjusted_origin_for_visible_monitor(
        origin_x=12,
        origin_y=34,
        allocation_width=1280,
        allocation_height=800,
        monitor_width=1280,
        monitor_height=800,
    ) == (12, 34)


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
    expected = [
        f"sky-cua-client{suffix}",
        f"sky-cua-service{suffix}",
        f"sky-cua-overlay-host{suffix}",
    ]
    if suffix == "":
        expected.append("sky-cua-cosmic-helper")
        expected.append("sky-cua-chrome-host")

    assert runtime_binary_names() == expected


def test_all_runtime_binary_names_include_linux_and_windows_binaries() -> None:
    assert all_runtime_binary_names() == [
        "runtimes/linux-x64/sky-cua-client",
        "runtimes/linux-x64/sky-cua-service",
        "runtimes/linux-x64/sky-cua-overlay-host",
        "runtimes/linux-x64/sky-cua-cosmic-helper",
        "runtimes/linux-x64/sky-cua-chrome-host",
        "runtimes/linux-arm64/sky-cua-client",
        "runtimes/linux-arm64/sky-cua-service",
        "runtimes/linux-arm64/sky-cua-overlay-host",
        "runtimes/linux-arm64/sky-cua-cosmic-helper",
        "runtimes/linux-arm64/sky-cua-chrome-host",
        "sky-cua-client.exe",
        "sky-cua-service.exe",
        "sky-cua-overlay-host.exe",
    ]


def test_runtime_binary_paths_map_platform_variants() -> None:
    assert runtime_binary_path("linux-x64", "sky-cua-client") == Path(
        "bin/runtimes/linux-x64/sky-cua-client"
    )
    assert runtime_binary_path("linux-arm64", "sky-cua-service") == Path(
        "bin/runtimes/linux-arm64/sky-cua-service"
    )
    assert runtime_binary_path("linux-x64", "sky-cua-overlay-host") == Path(
        "bin/runtimes/linux-x64/sky-cua-overlay-host"
    )
    assert runtime_binary_path("linux-x64", "sky-cua-cosmic-helper") == Path(
        "bin/runtimes/linux-x64/sky-cua-cosmic-helper"
    )
    assert runtime_binary_path("linux-arm64", "sky-cua-chrome-host") == Path(
        "bin/runtimes/linux-arm64/sky-cua-chrome-host"
    )
    assert runtime_binary_path("windows-x64", "sky-cua-client") == Path("bin/sky-cua-client.exe")
    assert runtime_binary_path("windows-x64", "sky-cua-overlay-host") == Path(
        "bin/sky-cua-overlay-host.exe"
    )


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
    assert Path("bin/sky-cua-overlay-host") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-client.exe") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-service.exe") in bundle_entrypoint_paths()
    assert Path("bin/sky-cua-overlay-host.exe") in bundle_entrypoint_paths()


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


def test_kwin_effect_static_mode_requires_explicit_install_flag(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "live_agent_cursor_kde_smoke.py",
            "--mode",
            "kwin-effect-static",
        ],
    )

    with pytest.raises(SystemExit) as exc_info:
        live_agent_cursor_kde_smoke.main()

    message = str(exc_info.value)
    assert "kwin-effect-static installs and loads a user-level KWin C++ effect" in message
    assert "--allow-kwin-effect-install" in message


def test_agent_cursor_smoke_x11_mode_forces_x11_backend(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    captured: dict[str, object] = {}

    class FakePopen:
        def __init__(self, args: list[str], **kwargs: object) -> None:
            captured["args"] = args
            captured.update(kwargs)

    monkeypatch.setattr(live_agent_cursor_kde_smoke.subprocess, "Popen", FakePopen)

    process = live_agent_cursor_kde_smoke.start_service(
        tmp_path / "svc.sock", tmp_path, mode="x11-debug-visible"
    )

    assert isinstance(process, FakePopen)
    env = cast(dict[str, str], captured["env"])
    assert env["SKY_CUA_OVERLAY_BACKEND"] == "x11"
    assert env["SKY_CUA_SCREENSHOT_CURSOR"] == "never"
    assert env["SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE"] == "never"


def test_agent_cursor_smoke_layer_shell_click_through_mode_forces_visible_overlay_env(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    captured: dict[str, object] = {}

    class FakePopen:
        def __init__(self, args: list[str], **kwargs: object) -> None:
            captured["args"] = args
            captured.update(kwargs)

    monkeypatch.setattr(live_agent_cursor_kde_smoke.subprocess, "Popen", FakePopen)

    process = live_agent_cursor_kde_smoke.start_service(
        tmp_path / "svc.sock", tmp_path, mode="layer-shell-click-through"
    )

    assert isinstance(process, FakePopen)
    env = cast(dict[str, str], captured["env"])
    assert env["SKY_CUA_OVERLAY_BACKEND"] == "wayland-layer-shell"
    assert env["SKY_CUA_SCREENSHOT_CURSOR"] == "never"
    assert env["SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE"] == "never"


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


def test_chrome_host_smoke_finds_service_process_by_socket_env(tmp_path: Path) -> None:
    proc_root = tmp_path / "proc"
    matching_proc = proc_root / "123"
    matching_proc.mkdir(parents=True)
    (matching_proc / "environ").write_bytes(
        b"PATH=/usr/bin\0SKY_CUA_SERVICE_SOCKET_PATH=/tmp/sky-cua-smoke.sock\0"
    )

    ignored_proc = proc_root / "456"
    ignored_proc.mkdir()
    (ignored_proc / "environ").write_bytes(b"SKY_CUA_SERVICE_SOCKET_PATH=/tmp/other.sock\0")

    assert _mcp_stdio.process_ids_with_env_var(
        "SKY_CUA_SERVICE_SOCKET_PATH",
        "/tmp/sky-cua-smoke.sock",
        proc_root=proc_root,
    ) == [123]


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
    assert (
        Path("docs/operations/testing-vm-desktop-smokes.md") in build_plugin.WORKTREE_BUNDLE_FILES
    )
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
    (effect_dir / "metadata.json").write_text(
        json.dumps(
            {
                "KPackageStructure": "KWin/Effect",
                "KPlugin": {"Id": "sky-cua-agent-cursor"},
            }
        ),
        encoding="utf-8",
    )
    bundle_root = tmp_path / "bundle"

    monkeypatch.setattr(build_plugin, "REPO_ROOT", repo_root)

    build_plugin.copy_worktree_bundle_dirs(bundle_root)

    copied = (
        bundle_root / "resources" / "kwin" / "effects" / "sky-cua-agent-cursor" / "metadata.json"
    )
    assert json.loads(copied.read_text(encoding="utf-8"))["KPlugin"]["Id"] == (
        "sky-cua-agent-cursor"
    )


def test_testing_vm_provisioner_installs_arch_desktop_packages() -> None:
    provisioner = (
        Path(__file__).resolve().parents[1]
        / "scripts"
        / "testing-vm"
        / "provision-arch-testing-vm.sh"
    )
    content = provisioner.read_text(encoding="utf-8")

    required_tokens = {
        "pacman-key --populate archlinux",
        "google-chrome-stable_current_amd64.deb",
        "google-chrome-stable",
        "CODEX_DESKTOP_PACKAGE",
        "pacman -U --noconfirm",
        "rust",
        "rsync",
        "openssh",
        "greetd",
        "seatd",
        "ydotool",
        "xorg-xrandr",
        "systemctl --global enable ydotool.service",
        "80-sky-cua-uinput.rules",
        "org.gnome.desktop.session idle-delay 0",
        "org.gnome.desktop.screensaver lock-enabled false",
        "org.gnome.settings-daemon.plugins.power sleep-inactive-ac-type nothing",
        "kwin",
        "plasma-desktop",
        "gnome-shell",
        "cosmic-session",
        "hyprland",
        "i3-wm",
        "gst-plugins-good",
        "libxss",
        "nss",
        "python-dbus",
        "python-gobject",
        "qt6-tools",
        "imagemagick",
        "jq",
        "libinput",
        "slurp",
        "socat",
        "strace",
        "tk",
        "wev",
        "weston",
        "wl-clipboard",
        "xorg-server",
        "xorg-xev",
        "xorg-xdpyinfo",
        "xorg-xwininfo",
        "libxtst",
        "xdg-utils",
        "xdg-desktop-portal-cosmic",
        "xdg-desktop-portal-gnome",
        "xdg-desktop-portal-hyprland",
        "xdg-desktop-portal-kde",
        "xdg-desktop-portal-wlr",
        "XDG_CURRENT_DESKTOP=COSMIC",
        "XDG_CURRENT_DESKTOP=KDE",
        "XDG_CURRENT_DESKTOP=GNOME",
        "XDG_CURRENT_DESKTOP=Hyprland",
        "XDG_CURRENT_DESKTOP=i3",
        "sky-cua-testing-vm-session",
    }
    missing = {token for token in required_tokens if token not in content}
    assert not missing, f"Provisioner is missing expected tokens: {missing}"

    assert "xorg-server-xvfb" not in content


def test_gui_test_profile_copies_essential_codex_settings() -> None:
    run_profile = (
        Path(__file__).resolve().parents[1]
        / "scripts"
        / "testing-vm"
        / "profiles"
        / "run-profile.sh"
    )
    content = run_profile.read_text(encoding="utf-8")

    assert "SKY_CUA_COPY_CODEX_SETTINGS" in content
    assert "auth.json" in content
    assert "config.toml" in content
    assert "config.json" in content
    assert "installation_id" in content
    assert "internal_storage.json" in content
    assert "cap_sid" in content
    assert "browser/config.toml" in content
    assert "for relative_dir in plugins skills" in content
    assert "/mnt/host-codex" in content


def test_gui_test_profiles_use_host_built_rust_artifacts() -> None:
    profile_root = Path(__file__).resolve().parents[1] / "scripts" / "testing-vm" / "profiles"
    wayland_pointer = (profile_root / "wayland-pointer.sh").read_text(encoding="utf-8")
    kde = (profile_root / "kde-kwin-effect.sh").read_text(encoding="utf-8")
    cosmic_helper = (profile_root / "cosmic-helper.sh").read_text(encoding="utf-8")
    run_profile = (profile_root / "run-profile.sh").read_text(encoding="utf-8")

    assert "cargo build" not in wayland_pointer
    assert "live_wayland_pointer_smoke.py" in wayland_pointer
    assert "cargo build" not in cosmic_helper
    assert "/workspace/target/release/sky-cua-cosmic-helper" in cosmic_helper
    assert "weston-flower" in cosmic_helper
    assert "cargo build" not in kde
    assert "SKY_CUA_OVERLAY_HOST_PATH" in kde
    assert "/workspace/target/release/sky-cua-overlay-host" in kde
    assert "${SKY_CUA_COPY_CODEX_SETTINGS:-0}" in run_profile


def test_wayland_pointer_smoke_requires_gnome_eis_diagnostics() -> None:
    success = {
        "structuredContent": {
            "diagnostics": [{"code": "PortalEisInputUsed"}],
        },
    }
    live_wayland_pointer_smoke.require_gnome_eis_input_used(success, "click", is_gnome=True)
    live_wayland_pointer_smoke.require_gnome_eis_input_used(
        {"structuredContent": {"diagnostics": []}}, "click", is_gnome=False
    )

    fallback = {
        "structuredContent": {
            "diagnostics": [{"code": "PortalEisInputFallback"}],
        },
    }
    import os

    # Without SKY_CUA_REQUIRE_EIS, fallback is a warning, not a hard failure
    live_wayland_pointer_smoke.require_gnome_eis_input_used(fallback, "click", is_gnome=True)

    # With SKY_CUA_REQUIRE_EIS=1, fallback is a hard failure
    os.environ["SKY_CUA_REQUIRE_EIS"] = "1"
    with pytest.raises(RuntimeError, match="PortalEisInputFallback"):
        live_wayland_pointer_smoke.require_gnome_eis_input_used(fallback, "click", is_gnome=True)
    del os.environ["SKY_CUA_REQUIRE_EIS"]

    # Without SKY_CUA_REQUIRE_EIS, missing EIS is also a warning
    live_wayland_pointer_smoke.require_gnome_eis_input_used(
        {"structuredContent": {"diagnostics": []}}, "click", is_gnome=True
    )

    # With SKY_CUA_REQUIRE_EIS=1, missing EIS is a hard failure
    os.environ["SKY_CUA_REQUIRE_EIS"] = "1"
    with pytest.raises(RuntimeError, match="did not use GNOME RemoteDesktop EIS input"):
        live_wayland_pointer_smoke.require_gnome_eis_input_used(
            {"structuredContent": {"diagnostics": []}}, "click", is_gnome=True
        )
    del os.environ["SKY_CUA_REQUIRE_EIS"]


def test_testing_vm_runner_runs_remote_arch_profile(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(
        command: list[str],
        *,
        cwd: Path,
        check: bool,
    ) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is False
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    assert (
        run_gui_testing_vm_smoke.run_remote_profile(
            "skycua@testing-vm",
            2222,
            ["StrictHostKeyChecking=no"],
            Path("/workspace"),
            "kde-kwin-effect",
            desktop_env="KDE",
        )
        == 0
    )

    command_text = " ".join(commands[0])
    assert "ssh -p 2222" in command_text
    assert "-o StrictHostKeyChecking=no" in command_text
    assert "SKY_CUA_USE_PREBUILT_RUNTIMES=1" in command_text
    assert "SKY_CUA_COPY_CODEX_SETTINGS=0" in command_text
    assert (
        "SKY_CUA_OVERLAY_HOST_PATH=/workspace/target/release/sky-cua-overlay-host" in command_text
    )
    assert (
        "SKY_CUA_DEBUG_OVERLAY_HOST_PATH=/workspace/target/debug/sky-cua-overlay-host"
        in command_text
    )
    assert "SKY_CUA_COSMIC_HELPER=/workspace/target/release/sky-cua-cosmic-helper" in command_text
    assert "PATH=/workspace/bin:" in command_text
    assert "XDG_CURRENT_DESKTOP=KDE" in command_text
    assert "systemctl --user import-environment" in command_text
    assert "scripts/testing-vm/profiles/run-profile.sh" in command_text
    assert "kde-kwin-effect" in command_text
    assert "kde-plasma" in run_gui_testing_vm_smoke.PROFILES
    assert "mcp-x11" not in run_gui_testing_vm_smoke.PROFILES
    assert "computer-use" in run_gui_testing_vm_smoke.PROFILES


def test_testing_vm_runner_remote_profile_opts_into_codex_settings(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(
        command: list[str],
        *,
        cwd: Path,
        check: bool,
    ) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is False
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    assert (
        run_gui_testing_vm_smoke.run_remote_profile(
            "skycua@testing-vm",
            2222,
            [],
            Path("/workspace"),
            "kde-kwin-effect",
            sync_codex_settings=True,
        )
        == 0
    )

    assert "SKY_CUA_COPY_CODEX_SETTINGS=1" in " ".join(commands[0])
    assert "codex-desktop" in run_gui_testing_vm_smoke.PROFILES
    assert "cosmic-helper" in run_gui_testing_vm_smoke.PROFILES
    assert "wayland-pointer" in run_gui_testing_vm_smoke.PROFILES


def test_testing_vm_profile_descriptors_carry_dispatch_and_curated_metadata() -> None:
    descriptors = run_gui_testing_vm_smoke.VM_PROFILE_DESCRIPTORS

    assert tuple(descriptors) == run_gui_testing_vm_smoke.PROFILES
    assert descriptors["kde-kwin-effect-system-install"].dispatch == "kwin-system-install"
    assert descriptors["kde-kwin-effect-system-install"].host_framebuffer_proof
    assert descriptors["kde-kwin-effect-system-install"].runner_profile() == "agent-cursor-kde"
    assert descriptors["cosmic-patched-cursor-host-proof"].dispatch == "cosmic-patched-host-proof"
    assert descriptors["cosmic-patched-cursor-host-proof"].host_framebuffer_proof
    assert descriptors["opencode-mcp"].preauthorize_screenshot_portal
    curated = {name for name, descriptor in descriptors.items() if descriptor.curated}
    assert {"computer-use", "codex-desktop", "wayland-pointer", "all"} <= curated


def test_testing_vm_remote_runner_builds_shared_runtime_script() -> None:
    runner = run_gui_testing_vm_smoke.RemoteRunner(
        ssh_target="skycua@testing-vm",
        port=2222,
        ssh_options=["StrictHostKeyChecking=no"],
        remote_root=Path("/workspace"),
        wayland_display="wayland-1",
        desktop_env="KDE",
    )

    script = runner.runtime_script(
        ["bash", "/workspace/scripts/testing-vm/profiles/run-profile.sh", "computer-use"]
    )

    assert "cd /workspace" in script
    assert 'runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"' in script
    assert 'WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-1}"' in script
    assert "XDG_CURRENT_DESKTOP=KDE" in script
    assert "run-profile.sh computer-use" in script


def test_testing_vm_runner_main_dispatches_kwin_system_install_profile(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[dict[str, object]] = []

    monkeypatch.setattr(
        sys,
        "argv",
        [
            "run_gui_testing_vm_smoke.py",
            "--host",
            "127.0.0.1",
            "--port",
            "22222",
            "--user",
            "skycua",
            "--profile",
            "kde-kwin-effect-system-install",
            "--desktop-env",
            "KDE",
            "--wayland-display",
            "wayland-0",
            "--vm-name",
            "testing-vm",
            "--libvirt-uri",
            "qemu:///session",
            "--sync-codex-settings",
        ],
    )
    monkeypatch.setattr(run_gui_testing_vm_smoke, "build_host_runtime_artifacts", lambda: None)
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "sync_checkout",
        lambda ssh_target, port, ssh_options, remote_root: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "sync_codex_settings",
        lambda ssh_target, port, ssh_options, codex_home: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "reset_guest_sky_cua_processes",
        lambda ssh_target, port, ssh_options: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "wake_guest_display",
        lambda ssh_target, port, ssh_options: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "refresh_guest_portal_stack",
        lambda ssh_target, port, ssh_options, *, wayland_display, desktop_env: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "refresh_guest_portal_stack",
        lambda ssh_target, port, ssh_options, *, wayland_display, desktop_env: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "preauthorize_kde_remote_desktop",
        lambda ssh_target, port, ssh_options, remote_root, *, wayland_display, desktop_env: None,
    )

    def fake_kwin_profile(
        ssh_target: str,
        port: int,
        ssh_options: list[str],
        remote_root: Path,
        *,
        wayland_display: str,
        desktop_env: str,
        sync_codex_settings: bool,
        vm_name: str,
        libvirt_uri: str,
    ) -> int:
        calls.append(
            {
                "ssh_target": ssh_target,
                "port": port,
                "ssh_options": ssh_options,
                "remote_root": remote_root,
                "wayland_display": wayland_display,
                "desktop_env": desktop_env,
                "sync_codex_settings": sync_codex_settings,
                "vm_name": vm_name,
                "libvirt_uri": libvirt_uri,
            }
        )
        return 17

    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "run_remote_kwin_effect_system_install_profile",
        fake_kwin_profile,
    )

    assert run_gui_testing_vm_smoke.main() == 17
    assert calls == [
        {
            "ssh_target": "skycua@127.0.0.1",
            "port": 22222,
            "ssh_options": [],
            "remote_root": Path("/workspace"),
            "wayland_display": "wayland-0",
            "desktop_env": "KDE",
            "sync_codex_settings": True,
            "vm_name": "testing-vm",
            "libvirt_uri": "qemu:///session",
        }
    ]


def test_testing_vm_runner_main_dispatches_cosmic_host_proof_profiles(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[str, str, str]] = []

    monkeypatch.setattr(run_gui_testing_vm_smoke, "build_host_runtime_artifacts", lambda: None)
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "sync_checkout",
        lambda ssh_target, port, ssh_options, remote_root: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "reset_guest_sky_cua_processes",
        lambda ssh_target, port, ssh_options: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "wake_guest_display",
        lambda ssh_target, port, ssh_options: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "refresh_guest_portal_stack",
        lambda ssh_target, port, ssh_options, *, wayland_display, desktop_env: None,
    )

    def fake_patched_profile(
        ssh_target: str,
        port: int,
        ssh_options: list[str],
        remote_root: Path,
        *,
        wayland_display: str,
        desktop_env: str,
        vm_name: str,
        libvirt_uri: str,
    ) -> int:
        calls.append(("patched", wayland_display, desktop_env))
        return 0

    def fake_transparent_profile(
        ssh_target: str,
        port: int,
        ssh_options: list[str],
        remote_root: Path,
        *,
        wayland_display: str,
        desktop_env: str,
        vm_name: str,
        libvirt_uri: str,
    ) -> int:
        calls.append(("transparent", wayland_display, desktop_env))
        return 0

    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "run_remote_cosmic_patched_cursor_host_proof_profile",
        fake_patched_profile,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "run_remote_cosmic_transparent_xcursor_host_proof_profile",
        fake_transparent_profile,
    )

    monkeypatch.setattr(
        sys,
        "argv",
        [
            "run_gui_testing_vm_smoke.py",
            "--host",
            "127.0.0.1",
            "--profile",
            "cosmic-patched-cursor-host-proof",
            "--wayland-display",
            "wayland-1",
        ],
    )
    assert run_gui_testing_vm_smoke.main() == 0

    monkeypatch.setattr(
        sys,
        "argv",
        [
            "run_gui_testing_vm_smoke.py",
            "--host",
            "127.0.0.1",
            "--profile",
            "cosmic-transparent-xcursor-host-proof",
            "--desktop-env",
            "cosmic",
            "--wayland-display",
            "wayland-2",
        ],
    )
    assert run_gui_testing_vm_smoke.main() == 0

    assert calls == [
        ("patched", "wayland-1", "COSMIC"),
        ("transparent", "wayland-2", "cosmic"),
    ]


def test_testing_vm_cosmic_host_framebuffer_profile_paths_are_stable() -> None:
    patched_paths = run_gui_testing_vm_smoke.cosmic_host_framebuffer_proof_paths(
        run_gui_testing_vm_smoke.COSMIC_PATCHED_CURSOR_HOST_PROOF,
        Path("/workspace"),
        "20260521T123456Z",
    )
    transparent_paths = run_gui_testing_vm_smoke.cosmic_host_framebuffer_proof_paths(
        run_gui_testing_vm_smoke.COSMIC_TRANSPARENT_XCURSOR_HOST_PROOF,
        Path("/workspace"),
        "20260521T123456Z",
    )

    assert patched_paths.remote_artifact_dir == Path(
        "/workspace/artifacts/codex-e2e/agent-cursor-cosmic-patched-host-proof/20260521T123456Z"
    )
    assert patched_paths.local_artifact_dir == (
        run_gui_testing_vm_smoke.REPO_ROOT
        / "artifacts"
        / "cosmic-framebuffer-cursor-proof"
        / "20260521T123456Z"
    )
    assert patched_paths.before_path == patched_paths.local_artifact_dir / "before.png"
    assert patched_paths.visible_path == patched_paths.local_artifact_dir / "visible.png"
    assert patched_paths.hidden_path == patched_paths.local_artifact_dir / "hidden.png"
    assert patched_paths.stdout_path == patched_paths.local_artifact_dir / "remote.stdout.log"
    assert patched_paths.stderr_path == patched_paths.local_artifact_dir / "remote.stderr.log"
    assert patched_paths.host_summary_path == patched_paths.local_artifact_dir / "host-summary.json"
    assert transparent_paths.remote_artifact_dir == Path(
        "/workspace/artifacts/codex-e2e/agent-cursor-cosmic-transparent-xcursor-host-proof/20260521T123456Z"
    )
    assert transparent_paths.local_artifact_dir == (
        run_gui_testing_vm_smoke.REPO_ROOT
        / "artifacts"
        / "cosmic-transparent-xcursor-cursor-proof"
        / "20260521T123456Z"
    )


def test_testing_vm_kwin_host_framebuffer_profile_paths_are_stable() -> None:
    paths = run_gui_testing_vm_smoke.kwin_host_framebuffer_proof_paths(
        run_gui_testing_vm_smoke.KWIN_EFFECT_SYSTEM_INSTALL_HOST_PROOF,
        Path("/workspace"),
        "20260521T123456Z",
    )

    assert paths.remote_artifact_dir == Path(
        "/workspace/artifacts/codex-e2e/agent-cursor-kde/20260521T123456Z-kwin-system-runner"
    )
    assert paths.local_artifact_dir == (
        run_gui_testing_vm_smoke.REPO_ROOT
        / "artifacts"
        / "kde-framebuffer-cursor-proof"
        / "kwin-system-install"
        / "20260521T123456Z"
    )
    assert paths.before_path == paths.local_artifact_dir / "before.png"
    assert paths.after_path == paths.local_artifact_dir / "after.png"
    assert paths.stdout_path == paths.local_artifact_dir / "remote.stdout.log"
    assert paths.stderr_path == paths.local_artifact_dir / "remote.stderr.log"
    assert paths.host_summary_path == paths.local_artifact_dir / "host-summary.json"


def test_testing_vm_cosmic_host_framebuffer_summary_writer_preserves_json_shape(
    tmp_path: Path,
) -> None:
    summary_path = tmp_path / "host-summary.json"

    run_gui_testing_vm_smoke.write_host_summary(
        summary_path,
        {"mode": "cosmic-patched-cursor-host-framebuffer", "ok": True},
    )

    assert json.loads(summary_path.read_text(encoding="utf-8")) == {
        "mode": "cosmic-patched-cursor-host-framebuffer",
        "ok": True,
    }
    assert summary_path.read_text(encoding="utf-8").endswith("\n")


def test_testing_vm_remote_process_wait_returns_success() -> None:
    process = subprocess.Popen(
        [sys.executable, "-c", "raise SystemExit(7)"],
        text=True,
    )

    returncode, timeout_error = run_gui_testing_vm_smoke.wait_for_remote_process(process, timeout=5)

    assert returncode == 7
    assert timeout_error is None


def test_testing_vm_remote_process_wait_kills_timed_out_process() -> None:
    process = subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(30)"],
        text=True,
    )

    returncode, timeout_error = run_gui_testing_vm_smoke.wait_for_remote_process(
        process, timeout=0.01
    )

    assert returncode != 0
    assert timeout_error == "remote process timed out after 0.01s"
    assert process.poll() is not None


def test_testing_vm_runner_reads_remote_jsons_with_nul_separators(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(
        command: list[str],
        *,
        cwd: Path,
        text: bool,
        capture_output: bool,
        check: bool,
    ) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert text is True
        assert capture_output is True
        assert check is False
        commands.append(command)
        return subprocess.CompletedProcess(
            command,
            0,
            stdout='{"first": 1}\x00---FILE_NOT_FOUND---\x00{"third": 3}\x00',
            stderr="",
        )

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    assert run_gui_testing_vm_smoke.read_remote_jsons(
        "skycua@testing-vm",
        2222,
        ["StrictHostKeyChecking=no"],
        [Path("/tmp/one.json"), Path("/tmp/missing.json"), Path("/tmp/three.json")],
    ) == [{"first": 1}, None, {"third": 3}]

    command_text = " ".join(commands[0])
    assert "printf '\\0'" in command_text
    assert "---FILE_NOT_FOUND---" in command_text


def test_testing_vm_runner_preauthorizes_kde_remote_desktop(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(command: list[str], *, cwd: Path, check: bool) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    assert run_gui_testing_vm_smoke.should_preauthorize_kde_remote_desktop(
        run_gui_testing_vm_smoke.profile_descriptor("computer-use"), "KDE"
    )
    assert run_gui_testing_vm_smoke.should_preauthorize_kde_remote_desktop(
        run_gui_testing_vm_smoke.profile_descriptor("wayland-pointer"), "plasma"
    )
    assert not run_gui_testing_vm_smoke.should_preauthorize_kde_remote_desktop(
        run_gui_testing_vm_smoke.profile_descriptor("codex-desktop"), "KDE"
    )
    assert not run_gui_testing_vm_smoke.should_preauthorize_kde_remote_desktop(
        run_gui_testing_vm_smoke.profile_descriptor("computer-use"), "GNOME"
    )

    run_gui_testing_vm_smoke.preauthorize_kde_remote_desktop(
        "skycua@testing-vm",
        2222,
        ["StrictHostKeyChecking=no"],
        Path("/workspace"),
        wayland_display="wayland-1",
        desktop_env="KDE",
    )

    command_text = " ".join(commands[0])
    assert "ssh -p 2222" in command_text
    assert "-o StrictHostKeyChecking=no" in command_text
    assert "XDG_CURRENT_DESKTOP=KDE" in command_text
    assert 'WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-1}"' in command_text
    assert "scripts/testing-vm/preauthorize_kde_remote_desktop.py" in command_text
    assert "--app-id '' --app-id desktop --print-json" in command_text


def test_testing_vm_runner_preauthorizes_gnome_remote_desktop(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(command: list[str], *, cwd: Path, check: bool) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    assert run_gui_testing_vm_smoke.should_preauthorize_gnome_remote_desktop(
        run_gui_testing_vm_smoke.profile_descriptor("computer-use"), "GNOME"
    )
    assert run_gui_testing_vm_smoke.should_preauthorize_gnome_remote_desktop(
        run_gui_testing_vm_smoke.profile_descriptor("wayland-pointer"), "gnome"
    )
    assert not run_gui_testing_vm_smoke.should_preauthorize_gnome_remote_desktop(
        run_gui_testing_vm_smoke.profile_descriptor("codex-desktop"), "GNOME"
    )
    assert not run_gui_testing_vm_smoke.should_preauthorize_gnome_remote_desktop(
        run_gui_testing_vm_smoke.profile_descriptor("computer-use"), "KDE"
    )

    run_gui_testing_vm_smoke.preauthorize_gnome_remote_desktop(
        "skycua@testing-vm",
        2222,
        ["StrictHostKeyChecking=no"],
        Path("/workspace"),
        wayland_display="wayland-0",
        desktop_env="GNOME",
    )

    command_text = " ".join(commands[0])
    assert "ssh -p 2222" in command_text
    assert "-o StrictHostKeyChecking=no" in command_text
    assert "XDG_CURRENT_DESKTOP=GNOME" in command_text
    assert 'WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"' in command_text
    assert "scripts/testing-vm/preauthorize_gnome_remote_desktop.py --print-json" in command_text


def test_kde_remote_desktop_preauthorizer_grants_empty_and_desktop_app_ids() -> None:
    helper = (
        Path(__file__).resolve().parents[1]
        / "scripts"
        / "testing-vm"
        / "preauthorize_kde_remote_desktop.py"
    )
    content = helper.read_text(encoding="utf-8")

    assert 'DEFAULT_APP_IDS = ("", "desktop")' in content
    assert "for app_id in app_ids" in content
    assert "missing_app_ids" in content


def test_testing_vm_runner_resets_overlay_host_processes(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(command: list[str], *, cwd: Path, check: bool) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    run_gui_testing_vm_smoke.reset_guest_sky_cua_processes(
        "skycua@testing-vm",
        2222,
        ["StrictHostKeyChecking=no"],
    )

    command_text = " ".join(commands[0])
    assert "pkill -x sky-cua-service" in command_text
    assert "pkill -f '(^|/)sky-cua-overlay-host( |$)'" in command_text
    assert "pkill -x sky-cua-overlay" in command_text
    assert "pkill -f '(^|/)gtk_pointer_smoke_fixture.py( |$)'" in command_text
    assert "pkill -x cosmic-randr" in command_text
    assert "agent-cursor.sock" in command_text


def test_testing_vm_runner_wakes_guest_display(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(command: list[str], *, cwd: Path, check: bool) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    run_gui_testing_vm_smoke.wake_guest_display(
        "skycua@testing-vm", 2222, ["StrictHostKeyChecking=no"]
    )

    command_text = " ".join(commands[0])
    assert "ssh -p 2222" in command_text
    assert "ydotool mousemove --absolute 20 20" in command_text
    assert "ydotool key 57:1 57:0" in command_text


def test_testing_vm_runner_refreshes_portal_stack_after_session_switch(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(command: list[str], *, cwd: Path, check: bool) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    run_gui_testing_vm_smoke.refresh_guest_portal_stack(
        "skycua@testing-vm",
        2222,
        ["StrictHostKeyChecking=no"],
        wayland_display="wayland-0",
        desktop_env="KDE",
    )

    command_text = " ".join(commands[0])
    assert "XDG_CURRENT_DESKTOP=KDE" in command_text
    assert 'WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"' in command_text
    assert "systemctl --user stop xdg-desktop-portal.service" in command_text
    assert "plasma-xdg-desktop-portal-kde.service" in command_text
    assert "pkill -f '(^|/)xdg-desktop-portal-[^/ ]+( |$)'" in command_text


def test_testing_vm_syncs_checkout_with_excludes(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(command: list[str], *, cwd: Path, check: bool) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    run_gui_testing_vm_smoke.sync_checkout("skycua@testing-vm", 22, [], Path("/workspace"))

    assert commands[0][0] == "ssh"
    assert commands[0][1] == "-p"
    assert commands[0][2] == "22"
    assert "ControlMaster=auto" in commands[0]
    assert "skycua@testing-vm" in commands[0]
    assert "mkdir" in commands[0]
    command_text = " ".join(commands[1])
    assert commands[1][0] == "rsync"
    assert "--delete" in commands[1]
    assert "--exclude target/debug/" in command_text
    assert "--exclude artifacts/" in command_text
    assert f"{run_gui_testing_vm_smoke.REPO_ROOT}/" in command_text
    assert "skycua@testing-vm:/workspace/" in command_text


def test_testing_vm_runner_builds_runtimes_on_host(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(command: list[str], *, cwd: Path, check: bool) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    run_gui_testing_vm_smoke.build_host_runtime_artifacts()

    assert commands == [
        [
            "cargo",
            "build",
            "--release",
            "-p",
            "sky-cua-client",
            "-p",
            "sky-cua-service",
            "-p",
            "sky-cua-cosmic-helper",
            "-p",
            "sky-cua-overlay-host",
        ],
        ["cargo", "build", "-p", "sky-cua-overlay-host"],
    ]


def test_testing_vm_runner_syncs_essential_codex_settings(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    commands: list[list[str]] = []
    codex_home = tmp_path / ".codex"
    (codex_home / "browser").mkdir(parents=True)
    (codex_home / "auth.json").write_text("{}", encoding="utf-8")
    (codex_home / "config.toml").write_text("model = 'gpt-5'\n", encoding="utf-8")
    (codex_home / "browser" / "config.toml").write_text("", encoding="utf-8")
    (codex_home / "plugins").mkdir()

    def fake_run(
        command: list[str],
        *,
        cwd: Path,
        check: bool,
    ) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)

    run_gui_testing_vm_smoke.sync_codex_settings(
        "skycua@testing-vm", 2222, ["StrictHostKeyChecking=no"], codex_home
    )

    command_text = "\n".join(" ".join(command) for command in commands)
    assert "ssh -p 2222" in command_text
    assert "ControlMaster=auto" in command_text
    assert "StrictHostKeyChecking=no" in command_text
    assert "skycua@testing-vm mkdir -p .codex" in command_text
    assert "auth.json skycua@testing-vm:.codex/auth.json" in command_text
    assert "config.toml skycua@testing-vm:.codex/config.toml" in command_text
    assert "browser/config.toml" in command_text
    assert "--delete" in command_text
    assert "plugins/" in command_text


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
    (root / "bin" / "sky-cua-overlay-host").write_text("#!/bin/sh\n", encoding="utf-8")
    (root / "bin" / "sky-cua-browser-preflight").write_text("#!/bin/sh\n", encoding="utf-8")
    (root / "skills" / "computer-use").mkdir(parents=True)
    (root / "skills" / "computer-use" / "SKILL.md").write_text(
        "skill",
        encoding="utf-8",
    )
    (root / "skills" / "browser-use").mkdir(parents=True)
    (root / "skills" / "browser-use" / "SKILL.md").write_text(
        "skill",
        encoding="utf-8",
    )
    (root / "docs" / "operations").mkdir(parents=True)
    (root / "docs" / "operations" / "testing-vm-desktop-smokes.md").write_text(
        "testing vm desktop smoke notes\n",
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
        Path("bin/sky-cua-overlay-host"),
        Path("bin/sky-cua-browser-preflight"),
        Path("skills/computer-use/SKILL.md"),
        Path("skills/browser-use/SKILL.md"),
        Path("docs/operations/testing-vm-desktop-smokes.md"),
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
        "sky-cua-overlay-host",
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
        "sky-cua-overlay-host.exe",
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


def test_build_runtime_packages_uses_packaging_contract(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[list[str]] = []

    def fake_run(command: list[str], *, check: bool) -> None:
        calls.append(command)
        assert check is True

    monkeypatch.setattr(build_runtime_packages.subprocess, "run", fake_run)

    build_runtime_packages.build_runtime_packages("linux-x64")
    build_runtime_packages.build_runtime_packages("windows-x64")

    assert calls == [
        [
            "cargo",
            "build",
            "--release",
            "--package",
            "sky-cua-client",
            "--package",
            "sky-cua-service",
            "--package",
            "sky-cua-overlay-host",
            "--package",
            "sky-cua-cosmic-helper",
            "--package",
            "sky-cua-chrome-host",
        ],
        [
            "cargo",
            "build",
            "--release",
            "--package",
            "sky-cua-client",
            "--package",
            "sky-cua-service",
            "--package",
            "sky-cua-overlay-host",
        ],
    ]


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
    assert (bin_dir / "sky-cua-overlay-host.exe").readlink() == (
        target_dir / "bin" / "sky-cua-overlay-host.exe"
    )
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
    overlay = target_dir / install_mcp_server.entrypoint_path("windows-x64", "sky-cua-overlay-host")
    overlay.write_text("overlay", encoding="utf-8")

    def fake_symlink_to(self: Path, target: Path) -> None:
        _ = self, target
        raise OSError("symlinks unavailable")

    monkeypatch.setattr(install_mcp_server, "current_platform", lambda: "windows-x64")
    monkeypatch.setattr(Path, "symlink_to", fake_symlink_to)

    install_mcp_server.link_current_platform_binaries(target_dir, bin_dir)

    assert (bin_dir / "sky-cua-client.exe").read_text(encoding="utf-8") == "client"
    assert (bin_dir / "sky-cua-service.exe").read_text(encoding="utf-8") == "service"
    assert (bin_dir / "sky-cua-overlay-host.exe").read_text(encoding="utf-8") == "overlay"


def test_generic_mcp_restart_runtime_stops_installed_processes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    calls: list[list[Path]] = []

    def fake_stop_unix_runtime_processes(search_roots: list[Path]) -> None:
        calls.append(search_roots)

    monkeypatch.setattr(
        install_mcp_server,
        "stop_unix_runtime_processes",
        fake_stop_unix_runtime_processes,
    )
    monkeypatch.setattr(install_mcp_server, "stop_windows_cache_processes", lambda _root: None)

    install_mcp_server.restart_runtime_processes(target_dir)

    assert calls == [[target_dir]]


def test_generic_mcp_main_can_restart_runtime_after_install(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    client_path = target_dir / "bin" / "sky-cua-client"
    restarted: list[Path] = []

    monkeypatch.setattr(
        sys, "argv", ["install_mcp_server.py", "--target-dir", str(target_dir), "--restart-runtime"]
    )
    monkeypatch.setattr(install_mcp_server, "install_binaries", lambda _target_dir: client_path)
    monkeypatch.setattr(
        install_mcp_server,
        "write_mcp_json",
        lambda target, _config: target / ".mcp.json",
    )
    monkeypatch.setattr(
        install_mcp_server,
        "restart_runtime_processes",
        lambda target: restarted.append(target),
    )
    monkeypatch.setattr(install_mcp_server, "print_next_steps", lambda *_args: None)

    assert install_mcp_server.main() == 0
    assert restarted == [target_dir.resolve()]


def test_generic_mcp_main_stops_windows_runtime_before_binary_copy(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    client_path = target_dir / "bin" / "sky-cua-client.exe"
    events: list[str] = []

    monkeypatch.setattr(
        sys,
        "argv",
        ["install_mcp_server.py", "--target-dir", str(target_dir), "--restart-runtime"],
    )
    monkeypatch.setattr(install_mcp_server.sys, "platform", "win32")
    monkeypatch.setattr(
        install_mcp_server,
        "restart_runtime_processes",
        lambda _target: events.append("restart"),
    )

    def fake_install_binaries(_target_dir: Path) -> Path:
        events.append("install")
        return client_path

    monkeypatch.setattr(install_mcp_server, "install_binaries", fake_install_binaries)
    monkeypatch.setattr(
        install_mcp_server,
        "write_mcp_json",
        lambda target, _config: target / ".mcp.json",
    )
    monkeypatch.setattr(install_mcp_server, "print_next_steps", lambda *_args: None)

    assert install_mcp_server.main() == 0
    assert events == ["restart", "install", "restart"]


def test_generic_mcp_next_steps_document_restart_runtime(
    capsys: pytest.CaptureFixture[str],
) -> None:
    target_dir = Path("/tmp/sky-cua-install")
    client_path = target_dir / "bin" / "sky-cua-client"
    config_path = target_dir / "opencode.json"

    install_mcp_server.print_next_steps("opencode", target_dir, client_path, config_path)
    install_mcp_server.print_next_steps("pi", target_dir, client_path, target_dir / "pi_mcp.json")
    install_mcp_server.print_next_steps(
        "openclaw", target_dir, client_path, target_dir / "openclaw_mcp.json"
    )

    output = capsys.readouterr().out
    assert "--restart-runtime" in output
    assert "Restart or reload the OpenCode session" in output
    assert "Restart Pi or run /reload" in output
    assert "configured OpenClaw workspace" in output
    assert "~/.openclaw/workspace/skills" not in output


def test_opencode_install_configures_browser_tools_without_enable_flag(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(install_mcp_server.BROWSER_SELECTION_ENV, raising=False)
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"

    config_path = install_mcp_server.install_opencode(target_dir, client_path)

    config = json.loads(config_path.read_text(encoding="utf-8"))
    env = config["mcp"]["sky_cua"]["environment"]
    assert env["SKY_CUA_REPO_ROOT"] == str(install_mcp_server.REPO_ROOT)
    assert install_mcp_server.BROWSER_SELECTION_ENV not in env


def test_opencode_install_preserves_browser_selection_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(install_mcp_server.BROWSER_SELECTION_ENV, "brave")
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"

    config_path = install_mcp_server.install_opencode(target_dir, client_path)

    config = json.loads(config_path.read_text(encoding="utf-8"))
    env = config["mcp"]["sky_cua"]["environment"]
    assert env[install_mcp_server.BROWSER_SELECTION_ENV] == "brave"


def test_claude_desktop_install_preserves_browser_selection_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(install_mcp_server.BROWSER_SELECTION_ENV, "brave")
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"

    config_path = install_mcp_server.install_claude_desktop(target_dir, client_path)

    config = json.loads(config_path.read_text(encoding="utf-8"))
    env = config["mcpServers"]["computer-use"]["env"]
    assert env["SKY_CUA_REPO_ROOT"] == str(install_mcp_server.REPO_ROOT)
    assert env[install_mcp_server.BROWSER_SELECTION_ENV] == "brave"


def test_pi_install_merges_mcp_config_and_copies_sky_cua_skills(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(install_mcp_server.BROWSER_SELECTION_ENV, "brave")
    repo_root = tmp_path / "repo"
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    agent_dir = tmp_path / "pi-agent"
    agent_dir.mkdir()
    (agent_dir / "mcp.json").write_text(
        json.dumps({"mcpServers": {"context7": {"command": "context7"}}}),
        encoding="utf-8",
    )
    stale_skill = agent_dir / "skills" / install_mcp_server.SKY_CUA_SKILLS[0]
    stale_skill.mkdir(parents=True)
    (stale_skill / "obsolete.md").write_text("old", encoding="utf-8")
    unrelated_skill = agent_dir / "skills" / "other-skill"
    unrelated_skill.mkdir(parents=True)
    (unrelated_skill / "SKILL.md").write_text("# other\n", encoding="utf-8")

    snippet_path = install_mcp_server.install_pi(target_dir, client_path, agent_dir)

    wrapper = target_dir / "pi_mcp_wrapper.sh"
    wrapper_text = wrapper.read_text(encoding="utf-8")
    assert f"export {install_mcp_server.BROWSER_SELECTION_ENV}=brave" in wrapper_text
    assert snippet_path == target_dir / "pi_mcp.json"
    snippet = json.loads(snippet_path.read_text(encoding="utf-8"))
    assert snippet["mcpServers"]["sky_cua"]["command"] == str(wrapper)

    merged = json.loads((agent_dir / "mcp.json").read_text(encoding="utf-8"))
    assert merged["mcpServers"]["context7"] == {"command": "context7"}
    assert merged["mcpServers"]["sky_cua"]["command"] == str(wrapper)
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        assert (agent_dir / "skills" / skill_name / "SKILL.md").read_text(
            encoding="utf-8"
        ) == f"# {skill_name}\n"
    assert not (stale_skill / "obsolete.md").exists()
    assert (unrelated_skill / "SKILL.md").read_text(encoding="utf-8") == "# other\n"


def test_pi_install_preserves_symlinked_mcp_config(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    agent_dir = tmp_path / "pi-agent"
    agent_dir.mkdir()
    real_config = tmp_path / "real-mcp.json"
    real_config.write_text(
        json.dumps({"mcpServers": {"context7": {"command": "context7"}}}) + "\n",
        encoding="utf-8",
    )
    config_link = agent_dir / "mcp.json"
    try:
        config_link.symlink_to(real_config)
    except OSError as error:
        pytest.skip(f"symlink creation is unavailable: {error}")

    install_mcp_server.install_pi(target_dir, client_path, agent_dir)

    assert config_link.is_symlink()
    merged = json.loads(real_config.read_text(encoding="utf-8"))
    assert merged["mcpServers"]["context7"] == {"command": "context7"}
    assert merged["mcpServers"]["sky_cua"]["command"] == str(target_dir / "pi_mcp_wrapper.sh")


def test_openclaw_install_sets_mcp_config_and_copies_sky_cua_skills(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(install_mcp_server.BROWSER_SELECTION_ENV, "brave")
    repo_root = tmp_path / "repo"
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)
    calls: list[dict[str, object]] = []

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
    ) -> subprocess.CompletedProcess[str]:
        calls.append({"command": command, "check": check, "env": env, "timeout": timeout})
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    openclaw_dir = tmp_path / "openclaw"
    (openclaw_dir / "workspace" / "skills" / install_mcp_server.SKY_CUA_SKILLS[0]).mkdir(
        parents=True
    )
    (
        openclaw_dir / "workspace" / "skills" / install_mcp_server.SKY_CUA_SKILLS[0] / "obsolete.md"
    ).write_text("old", encoding="utf-8")

    config_path = install_mcp_server.install_openclaw(
        target_dir,
        client_path,
        openclaw_dir=openclaw_dir,
        openclaw_bin="openclaw",
    )

    assert config_path == target_dir / "openclaw_mcp.json"
    snippet = json.loads(config_path.read_text(encoding="utf-8"))
    server = snippet["mcp"]["servers"]["sky_cua"]
    assert server["command"] == str(client_path)
    assert server["args"] == ["mcp"]
    assert server["cwd"] == str(target_dir)
    assert server["env"]["SKY_CUA_REPO_ROOT"] == str(repo_root)
    assert server["env"][install_mcp_server.BROWSER_SELECTION_ENV] == "brave"
    assert server["codex"]["defaultToolsApprovalMode"] == "approve"

    assert len(calls) == 1
    command = cast(list[str], calls[0]["command"])
    assert command[:4] == ["openclaw", "mcp", "set", "sky_cua"]
    assert json.loads(command[4]) == server
    assert calls[0]["check"] is True
    assert calls[0]["timeout"] == install_mcp_server.OPENCLAW_MCP_SET_TIMEOUT_SECONDS
    env = cast(dict[str, str], calls[0]["env"])
    assert env["OPENCLAW_STATE_DIR"] == str(openclaw_dir)
    assert env["OPENCLAW_CONFIG_PATH"] == str(openclaw_dir / "openclaw.json")

    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        assert (openclaw_dir / "workspace" / "skills" / skill_name / "SKILL.md").read_text(
            encoding="utf-8"
        ) == f"# {skill_name}\n"
    assert not (
        openclaw_dir / "workspace" / "skills" / install_mcp_server.SKY_CUA_SKILLS[0] / "obsolete.md"
    ).exists()


def test_openclaw_install_reports_registration_timeout(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
    ) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(command, timeout)

    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    with pytest.raises(TimeoutError, match="timed out registering sky-cua with OpenClaw"):
        install_mcp_server.install_openclaw(
            tmp_path / "installed",
            tmp_path / "installed" / "bin" / "sky-cua-client",
            openclaw_dir=tmp_path / "openclaw",
            openclaw_bin="openclaw",
        )


def test_generic_mcp_main_can_install_openclaw_host(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    client_path = target_dir / "bin" / "sky-cua-client"
    installed: list[tuple[Path, Path, Path]] = []

    monkeypatch.setattr(
        sys,
        "argv",
        [
            "install_mcp_server.py",
            "--target-dir",
            str(target_dir),
            "--host",
            "openclaw",
            "--openclaw-dir",
            str(openclaw_dir),
        ],
    )
    monkeypatch.setattr(install_mcp_server, "install_binaries", lambda _target_dir: client_path)
    monkeypatch.setattr(
        install_mcp_server,
        "install_openclaw",
        lambda target, client, openclaw_dir, openclaw_bin="openclaw": (
            installed.append((target, client, openclaw_dir)) or target / "openclaw_mcp.json"
        ),
    )
    monkeypatch.setattr(install_mcp_server, "print_next_steps", lambda *_args: None)

    assert install_mcp_server.main() == 0
    assert installed == [(target_dir.resolve(), client_path, openclaw_dir.resolve())]


def test_pi_mcp_config_merge_keeps_existing_file_when_replace_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config_path = tmp_path / "mcp.json"
    original = json.dumps({"mcpServers": {"context7": {"command": "context7"}}}) + "\n"
    config_path.write_text(original, encoding="utf-8")

    def fail_replace(_source: Path, _destination: Path) -> None:
        raise OSError("replace failed")

    monkeypatch.setattr(install_mcp_server.os, "replace", fail_replace)

    with pytest.raises(OSError, match="replace failed"):
        install_mcp_server.merge_pi_mcp_config(
            config_path,
            {"mcpServers": {"sky_cua": {"command": "/tmp/sky-cua-client"}}},
        )

    assert config_path.read_text(encoding="utf-8") == original
    assert not list(tmp_path.glob(".mcp.json.tmp-*"))


def test_pi_mcp_config_merge_preserves_existing_file_permissions(tmp_path: Path) -> None:
    config_path = tmp_path / "mcp.json"
    config_path.write_text(
        json.dumps({"mcpServers": {"context7": {"command": "context7"}}}) + "\n",
        encoding="utf-8",
    )
    config_path.chmod(0o600)

    install_mcp_server.merge_pi_mcp_config(
        config_path,
        {"mcpServers": {"sky_cua": {"command": "/tmp/sky-cua-client"}}},
    )

    assert stat.S_IMODE(config_path.stat().st_mode) == 0o600


def test_pi_skill_install_keeps_existing_skill_when_copy_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    skill_name = "computer-use"
    monkeypatch.setattr(install_mcp_server, "SKY_CUA_SKILLS", (skill_name,))
    repo_root = tmp_path / "repo"
    source = repo_root / "skills" / skill_name
    source.mkdir(parents=True)
    (source / "SKILL.md").write_text("# new\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    skills_dir = tmp_path / "skills"
    destination = skills_dir / skill_name
    destination.mkdir(parents=True)
    (destination / "SKILL.md").write_text("# old\n", encoding="utf-8")

    def fail_copytree(_source: Path, destination: Path) -> None:
        destination.mkdir(parents=True)
        (destination / "partial.md").write_text("partial\n", encoding="utf-8")
        raise OSError("copy failed")

    monkeypatch.setattr(install_mcp_server.shutil, "copytree", fail_copytree)

    with pytest.raises(OSError, match="copy failed"):
        install_mcp_server.install_pi_skills(skills_dir)

    assert (destination / "SKILL.md").read_text(encoding="utf-8") == "# old\n"
    assert not (destination / "partial.md").exists()
    assert not list(skills_dir.glob(f".{skill_name}.tmp-*"))


def test_replace_tree_atomically_restores_file_destination_when_replace_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source = tmp_path / "source"
    source.mkdir()
    (source / "SKILL.md").write_text("# new\n", encoding="utf-8")
    destination = tmp_path / "skill"
    destination.write_text("# old-file\n", encoding="utf-8")
    real_replace = install_mcp_server.os.replace

    def fail_new_tree_replace(source_path: Path, destination_path: Path) -> None:
        if (
            Path(source_path).name.startswith(".skill.tmp-")
            and Path(destination_path) == destination
        ):
            raise OSError("replace failed")
        real_replace(source_path, destination_path)

    monkeypatch.setattr(install_mcp_server.os, "replace", fail_new_tree_replace)

    with pytest.raises(OSError, match="replace failed"):
        install_mcp_server.replace_tree_atomically(source, destination)

    assert destination.read_text(encoding="utf-8") == "# old-file\n"
    assert not list(tmp_path.glob(".skill.tmp-*"))
    assert not list(tmp_path.glob(".skill.backup-*"))


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
    assert "SKY_CUA_AGENT_CURSOR" in env_vars
    assert "SKY_CUA_BROWSER" in env_vars
    assert "SKY_CUA_OVERLAY_BACKEND" in env_vars
    assert "SKY_CUA_INPUT_BACKEND" in env_vars
    assert "SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE" in env_vars
    assert "SKY_CUA_OVERLAY_HOST_PATH" in env_vars
    assert "SKY_CUA_OVERLAY_HOST_TCP_ADDR" in env_vars
    assert "SKY_CUA_SCREENSHOT_CURSOR" in env_vars
    assert "SKY_CUA_REPO_ROOT" in env_vars
    assert "SKY_CUA_SERVICE_PATH" in env_vars
    assert "YDOTOOL_SOCKET" in env_vars


def test_tab_list_proof_redacts_titles_and_urls() -> None:
    proof = live_chrome_host_client_smoke.redacted_tab_list_proof(
        {
            "id": "client-get-user-tabs-mcp-proof",
            "result": {
                "tabs": [
                    {
                        "id": 42,
                        "title": "Private tab title",
                        "url": "https://private.example.test/path",
                    }
                ]
            },
        },
        expected_tab_id=42,
    )

    assert proof == {
        "id": "client-get-user-tabs-mcp-proof",
        "has_result": True,
        "expected_tab_id": 42,
        "expected_tab_present": True,
        "tabs_count": 1,
    }
    assert "Private tab title" not in json.dumps(proof)
    assert "private.example.test" not in json.dumps(proof)


def test_expected_tab_present_accepts_mcp_tab_id_shape() -> None:
    tabs: list[object] = [
        {"tab_id": "7", "title": "Private tab title", "url": "https://private.example.test"}
    ]

    assert live_chrome_host_client_smoke.expected_tab_present(tabs, 7) is True
    assert live_chrome_host_client_smoke.expected_tab_present(tabs, 8) is False
    assert live_chrome_host_client_smoke.expected_tab_present(tabs, None) is None


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
    assert live_agent_cursor_kde_smoke.CURSOR_ASSET_WIDTH == 23
    assert live_agent_cursor_kde_smoke.CURSOR_ASSET_HEIGHT == 24
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


def test_x11_overlay_smoke_forces_true_x11_backend_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("WAYLAND_DISPLAY", "wayland-0")

    env = live_agent_cursor_x11_overlay_smoke.x11_overlay_env(":42", tmp_path)

    assert env["DISPLAY"] == ":42"
    assert env["XDG_SESSION_TYPE"] == "x11"
    assert env["XDG_RUNTIME_DIR"] == str(tmp_path)
    assert env["SKY_CUA_OVERLAY_BACKEND"] == "x11"
    assert "WAYLAND_DISPLAY" not in env


def test_x11_overlay_smoke_cursor_message_uses_stream_pixels() -> None:
    message = live_agent_cursor_x11_overlay_smoke.cursor_message((330.0, 240.0), sequence=7)

    assert message == {
        "version": 1,
        "kind": "set_cursor",
        "state": {
            "visible": True,
            "sequence": 7,
            "model_point": {
                "x": 330.0,
                "y": 240.0,
                "coordinate_space": "stream_pixels",
            },
            "source_action": "click",
            "updated_at_ms": 0,
        },
    }


def test_x11_overlay_smoke_show_message_reuses_state_and_forces_visible() -> None:
    hidden_reply = {
        "ok": True,
        "state": {
            "visible": False,
            "sequence": 7,
            "model_point": {
                "x": 330.0,
                "y": 240.0,
                "coordinate_space": "stream_pixels",
            },
            "source_action": "click",
            "updated_at_ms": 0,
        },
    }

    message = live_agent_cursor_x11_overlay_smoke.show_cursor_message(hidden_reply)

    assert message["version"] == 1
    assert message["kind"] == "show"
    assert message["state"]["visible"] is True
    assert message["state"]["sequence"] == 7
    assert hidden_reply["state"]["visible"] is False


def test_x11_overlay_smoke_show_message_requires_state() -> None:
    with pytest.raises(RuntimeError, match="did not include cursor state"):
        live_agent_cursor_x11_overlay_smoke.show_cursor_message({"ok": True})


def test_x11_overlay_smoke_rejects_non_x11_overlay_reply() -> None:
    with pytest.raises(RuntimeError, match="x11_shaped_window"):
        live_agent_cursor_x11_overlay_smoke.require_x11_overlay_reply(
            {
                "ok": True,
                "capabilities": {
                    "backend": "none",
                    "visible_overlay": False,
                    "click_through": False,
                    "system_cursor_hide_supported": False,
                    "system_cursor_hidden": False,
                },
            }
        )


def test_x11_overlay_smoke_visible_cursor_reply_requires_visible_state() -> None:
    with pytest.raises(RuntimeError, match="visible cursor"):
        live_agent_cursor_x11_overlay_smoke.require_visible_cursor_reply(
            {"ok": True, "state": {"visible": False}},
            context="show",
        )

    live_agent_cursor_x11_overlay_smoke.require_visible_cursor_reply(
        {"ok": True, "state": {"visible": True}},
        context="show",
    )


def test_x11_overlay_smoke_system_cursor_reply_tracks_hide_state() -> None:
    live_agent_cursor_x11_overlay_smoke.require_system_cursor_reply(
        {
            "ok": True,
            "capabilities": {
                "system_cursor_hide_supported": True,
                "system_cursor_hidden": True,
            },
        },
        hidden=True,
        context="set",
    )
    live_agent_cursor_x11_overlay_smoke.require_system_cursor_reply(
        {
            "ok": True,
            "capabilities": {
                "system_cursor_hide_supported": True,
                "system_cursor_hidden": False,
            },
        },
        hidden=False,
        context="hide",
    )
    assert (
        live_agent_cursor_x11_overlay_smoke.capability_bool(
            {
                "capabilities": {
                    "system_cursor_hidden": True,
                }
            },
            "system_cursor_hidden",
        )
        is True
    )


def test_x11_overlay_current_display_requires_real_x11_session(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("DISPLAY", ":0")
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    monkeypatch.setenv("WAYLAND_DISPLAY", "wayland-0")

    with pytest.raises(RuntimeError, match="real X11 session"):
        live_agent_cursor_x11_overlay_smoke.require_current_x11_display()


def test_x11_overlay_current_display_accepts_x11_without_wayland(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("DISPLAY", ":7")
    monkeypatch.setenv("XDG_SESSION_TYPE", "x11")
    monkeypatch.delenv("WAYLAND_DISPLAY", raising=False)

    assert live_agent_cursor_x11_overlay_smoke.require_current_x11_display() == ":7"


def test_kwin_effect_list_parser_ignores_blank_lines() -> None:
    assert live_agent_cursor_kde_smoke.parse_kwin_effect_list(
        "\nblur\n\nsky-cua-agent-cursor\n  showfps  \n"
    ) == ["blur", "sky-cua-agent-cursor", "showfps"]


def test_kde_smoke_names_expected_visible_overlay_backend_by_mode() -> None:
    assert (
        live_agent_cursor_kde_smoke.expected_overlay_backend("layer-shell-debug-visible")
        == "wayland_layer_shell"
    )
    assert (
        live_agent_cursor_kde_smoke.expected_overlay_backend("layer-shell-hide-for-capture")
        == "wayland_layer_shell"
    )
    assert (
        live_agent_cursor_kde_smoke.expected_overlay_backend("layer-shell-click-through")
        == "wayland_layer_shell"
    )
    assert (
        live_agent_cursor_kde_smoke.expected_overlay_backend("x11-debug-visible")
        == "x11_shaped_window"
    )
    assert live_agent_cursor_kde_smoke.expected_overlay_backend("synthetic") is None


def test_kde_smoke_rejects_visible_overlay_mode_without_expected_backend() -> None:
    with pytest.raises(RuntimeError, match="wayland_layer_shell"):
        live_agent_cursor_kde_smoke.require_cursor_backend_capabilities(
            {
                "capabilities": {
                    "backend": "screenshot_synthetic",
                    "visible_overlay": False,
                    "click_through": False,
                }
            },
            expected_backend="wayland_layer_shell",
        )


def test_kde_smoke_accepts_expected_visible_overlay_capabilities() -> None:
    live_agent_cursor_kde_smoke.require_cursor_backend_capabilities(
        {
            "capabilities": {
                "backend": "wayland_layer_shell",
                "visible_overlay": True,
                "click_through": True,
            }
        },
        expected_backend="wayland_layer_shell",
    )


def test_kde_smoke_native_point_for_portal_capture_is_output_local() -> None:
    point = live_agent_cursor_kde_smoke.native_point_from_capture(
        {
            "backend": "portal_pipe_wire",
            "pixel_size": {"width": 400, "height": 200},
            "logical_rect": {
                "x": 100,
                "y": 50,
                "width": 200,
                "height": 100,
                "space": "desktop_logical",
            },
            "mapping_id": "mapping",
        },
        (40.0, 50.0),
    )

    assert point == {
        "x": 20.0,
        "y": 25.0,
        "coordinate_space": "stream_logical",
        "mapping_id": "mapping",
    }


def test_kde_smoke_maps_logical_fixture_point_to_model_pixels() -> None:
    point = live_agent_cursor_kde_smoke.model_point_from_logical_capture(
        {
            "backend": "portal_pipe_wire",
            "pixel_size": {"width": 800, "height": 400},
            "logical_rect": {
                "x": 100,
                "y": 50,
                "width": 400,
                "height": 200,
                "space": "desktop_logical",
            },
        },
        {"x": 300.0, "y": 100.0},
    )

    assert point == (400.0, 100.0)


def test_kde_smoke_rejects_fixture_point_outside_capture() -> None:
    with pytest.raises(RuntimeError, match="outside capture pixel bounds"):
        live_agent_cursor_kde_smoke.model_point_from_logical_capture(
            {
                "pixel_size": {"width": 800, "height": 400},
                "logical_rect": {
                    "x": 100,
                    "y": 50,
                    "width": 400,
                    "height": 200,
                    "space": "desktop_logical",
                },
            },
            {"x": 900.0, "y": 100.0},
        )


def test_kde_smoke_execute_click_request_uses_snapshot_and_stream_pixels() -> None:
    assert live_agent_cursor_kde_smoke.execute_click_request("snap-1", (12.5, 99.0)) == {
        "type": "execute_action",
        "request": {
            "action": "click",
            "snapshot_id": "snap-1",
            "arguments": {"x": 12.5, "y": 99.0},
        },
    }


def test_kde_smoke_fixture_point_requires_named_point() -> None:
    with pytest.raises(RuntimeError, match="click_button"):
        live_agent_cursor_kde_smoke.fixture_point({"points": {}}, "click_button")


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


def test_chrome_mcp_client_times_out_when_process_sends_no_frame() -> None:
    client = live_chrome_host_client_smoke.McpClient(
        [sys.executable, "-c", "import time; time.sleep(60)"],
        extra_env={},
        read_timeout=0.05,
    )

    started_at = time.monotonic()
    with pytest.raises(RuntimeError, match="timed out while reading MCP headers"):
        client._read_message()

    assert time.monotonic() - started_at < 2
    assert client.proc.poll() is not None


def test_chrome_native_request_uses_aggregate_timeout_for_pings() -> None:
    client_sock, server_sock = socket.socketpair()
    stop = threading.Event()

    def serve_pings() -> None:
        try:
            live_chrome_host_client_smoke.read_native_frame(server_sock, timeout=1)
            while not stop.is_set():
                try:
                    live_chrome_host_client_smoke.write_native_frame(
                        server_sock,
                        {"jsonrpc": "2.0", "id": "ping", "method": "ping"},
                    )
                except OSError:
                    break
                time.sleep(0.01)
        finally:
            server_sock.close()

    thread = threading.Thread(target=serve_pings)
    thread.start()
    started_at = time.monotonic()
    try:
        with pytest.raises(TimeoutError, match=r"native request getInfo.*timed out"):
            live_chrome_host_client_smoke.native_request(
                client_sock,
                "getInfo",
                {},
                timeout=0.05,
                request_id="aggregate-timeout",
            )
        assert time.monotonic() - started_at < 2
    finally:
        stop.set()
        client_sock.close()
        thread.join(timeout=2)


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
