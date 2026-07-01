"""Tests for the GUI testing VM runner, provisioner, and profiles."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

import run_gui_testing_vm_smoke


def load_screenshot_preauth_module() -> Any:
    script_path = (
        Path(__file__).resolve().parents[1]
        / "scripts"
        / "testing-vm"
        / "preauthorize_screenshot_portal.py"
    )
    spec = importlib.util.spec_from_file_location("preauthorize_screenshot_portal", script_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
        "commandlinetools-linux-15641748_latest.zip",
        "sdkmanager",
        "platform-tools",
        "ANDROID_PLATFORM:-android-36",
        "platforms;",
        "build-tools;",
        "36.0.0",
        "rust",
        "rsync",
        "openssh",
        "greetd",
        "kdialog",
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
        "jdk21-openjdk",
        "python-dbus",
        "python-gobject",
        "python-pytest",
        "uv",
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
    assert "auth.json" not in content
    assert "config.toml" in content
    assert "config.json" in content
    assert "installation_id" not in content
    assert "internal_storage.json" not in content
    assert "cap_sid" not in content
    assert "state_5.sqlite" not in content
    assert "browser/config.toml" in content
    assert "for relative_dir in plugins skills" in content
    assert "/mnt/host-codex" in content


def test_testing_vm_opencode_sync_has_user_writable_latest_fallback() -> None:
    script_root = Path(__file__).resolve().parents[1] / "scripts" / "testing-vm"
    sync_script = (script_root / "sync-opencode-to-vm.sh").read_text(encoding="utf-8")
    opencode_profile = (script_root / "profiles" / "opencode-mcp.sh").read_text(encoding="utf-8")
    pi_profile = (script_root / "profiles" / "pi-mcp.sh").read_text(encoding="utf-8")

    assert 'export PATH="$HOME/.local/bin:$PATH"' in sync_script
    assert "sudo -n npm install -g opencode-ai@latest" in sync_script
    assert "npm install -g --prefix ~/.local opencode-ai@latest" in sync_script
    assert "warning: opencode update failed; continuing with existing install" in sync_script
    assert 'export PATH="${HOME}/.local/bin:${PATH}"' in opencode_profile
    for profile in (opencode_profile, pi_profile):
        assert "install_policy_args+=(--browser-eval" in profile
        assert "install_policy_args+=(--model-supports-images" in profile
        # Consolidated to a single wiring check: no dialog fixtures, no fixture loop.
        assert "--mode wiring" in profile
        assert "for fixture in zenity kdialog; do" not in profile
        assert "missing_fixtures=()" not in profile
    assert "--agent opencode --mode wiring" in opencode_profile
    assert "--agent pi --mode wiring" in pi_profile


def test_testing_vm_pi_sync_has_user_writable_latest_fallback() -> None:
    sync_script = (
        Path(__file__).resolve().parents[1] / "scripts" / "testing-vm" / "sync-pi-to-vm.sh"
    ).read_text(encoding="utf-8")

    assert 'export PATH="${HOME}/.local/bin:${PATH}"' in sync_script
    assert "sudo -n npm install -g @earendil-works/pi-coding-agent@latest" in sync_script
    assert "npm install -g --prefix ~/.local @earendil-works/pi-coding-agent@latest" in sync_script
    assert "sudo -n npm install -g pi-mcp-adapter@latest" in sync_script
    assert "npm install -g --prefix ~/.local pi-mcp-adapter@latest" in sync_script


def test_gui_test_profiles_use_host_built_rust_artifacts() -> None:
    profile_root = Path(__file__).resolve().parents[1] / "scripts" / "testing-vm" / "profiles"
    wayland_pointer = (profile_root / "wayland-pointer.sh").read_text(encoding="utf-8")
    kde = (profile_root / "kde-kwin-effect.sh").read_text(encoding="utf-8")
    cosmic_helper = (profile_root / "cosmic-helper.sh").read_text(encoding="utf-8")
    display_screenshot = (profile_root / "display-screenshot.sh").read_text(encoding="utf-8")
    run_profile = (profile_root / "run-profile.sh").read_text(encoding="utf-8")

    assert "cargo build" not in wayland_pointer
    assert "sky-cua-chrome-host" in run_gui_testing_vm_smoke.RUNTIME_PACKAGES
    assert "live_wayland_pointer_smoke.py" in wayland_pointer
    assert "cargo build" not in cosmic_helper
    assert "/workspace/target/release/sky-cua-cosmic-helper" in cosmic_helper
    assert "weston-flower" in cosmic_helper
    assert "cargo build" not in kde
    assert "SKY_CUA_OVERLAY_HOST_PATH" in kde
    assert "/workspace/target/release/sky-cua-overlay-host" in kde
    assert "SKY_CUA_DISPLAY_SCREENSHOT_REQUIRE_SECONDARY" in display_screenshot
    assert "${SKY_CUA_COPY_CODEX_SETTINGS:-0}" in run_profile


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
    monkeypatch.setenv("OPENAI_API_KEY", "openai-secret")

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
    assert "OPENAI_API_KEY" not in command_text
    assert "kde-plasma" in run_gui_testing_vm_smoke.PROFILES
    assert "mcp-x11" not in run_gui_testing_vm_smoke.PROFILES
    assert "computer-use" in run_gui_testing_vm_smoke.PROFILES


def test_testing_vm_runner_forwards_auth_only_to_agent_profiles(
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
    monkeypatch.setenv("OPENCODE_API_KEY", "opencode-secret")
    monkeypatch.setenv("CONTEXT7_API_KEY", "context-secret")
    monkeypatch.setenv("SKY_CUA_SMOKE_OPENCODE_MODEL", "opencode/nemotron-3-ultra-free")
    monkeypatch.setenv("SKY_CUA_MCP_TOOL_PROFILE", "compact")
    monkeypatch.setenv("SKY_CUA_BROWSER_EVAL", "on")
    monkeypatch.setenv("SKY_CUA_MODEL_SUPPORTS_IMAGES", "false")

    assert (
        run_gui_testing_vm_smoke.run_remote_profile(
            "skycua@testing-vm",
            2222,
            [],
            Path("/workspace"),
            "opencode-mcp",
        )
        == 0
    )

    command_text = " ".join(commands[0])
    assert "OPENCODE_API_KEY=opencode-secret" in command_text
    assert "CONTEXT7_API_KEY=context-secret" in command_text
    assert "SKY_CUA_SMOKE_OPENCODE_MODEL=opencode/nemotron-3-ultra-free" in command_text
    assert "SKY_CUA_MCP_TOOL_PROFILE" not in command_text
    assert "SKY_CUA_BROWSER_EVAL=on" in command_text
    assert "SKY_CUA_MODEL_SUPPORTS_IMAGES=false" in command_text


def test_testing_vm_runner_forwards_auth_to_all_profile(
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
    monkeypatch.setenv("OPENCODE_API_KEY", "opencode-secret")
    monkeypatch.setenv("CONTEXT7_API_KEY", "context-secret")
    monkeypatch.setenv("SKY_CUA_SMOKE_OPENCODE_MODEL", "opencode/nemotron-3-ultra-free")

    assert (
        run_gui_testing_vm_smoke.run_remote_profile(
            "skycua@testing-vm",
            2222,
            [],
            Path("/workspace"),
            "all",
        )
        == 0
    )

    command_text = " ".join(commands[0])
    assert "OPENCODE_API_KEY=opencode-secret" in command_text
    assert "CONTEXT7_API_KEY=context-secret" in command_text
    assert "SKY_CUA_SMOKE_OPENCODE_MODEL=opencode/nemotron-3-ultra-free" in command_text


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
    assert descriptors["targeted-screenshot"].preauthorize_screenshot_portal
    assert descriptors["display-screenshot"].preauthorize_screenshot_portal
    assert descriptors["all"].preauthorize_screenshot_portal
    curated = {name for name, descriptor in descriptors.items() if descriptor.curated}
    assert curated == {
        "codex-desktop",
        "wayland-pointer",
        "session-env",
        "text-readback",
    }
    assert curated == set(run_gui_testing_vm_smoke.CURATED_PROFILE_NAMES)
    assert descriptors["curated"].dispatch == "curated"
    assert not descriptors["curated"].curated
    assert not descriptors["all"].curated
    assert not descriptors["computer-use"].curated
    assert not descriptors["kde-kwin-effect-system-install"].curated
    assert not descriptors["wayland-pointer-scaled"].curated
    # Compositor-specific capture lanes stay outside the session-agnostic set.
    assert not descriptors["wayland-layer-shell-overlay"].curated
    assert not descriptors["desktop-smoke"].curated


def test_testing_vm_isolated_xpra_profile_is_registered_as_a_remote_profile() -> None:
    descriptors = run_gui_testing_vm_smoke.VM_PROFILE_DESCRIPTORS

    assert "isolated-xpra" in descriptors
    assert "isolated-xpra" in run_gui_testing_vm_smoke.PROFILES
    isolated = descriptors["isolated-xpra"]
    # Mirrors the i3 lane: a plain remote profile, not curated, not a
    # host-framebuffer proof.
    assert isolated.dispatch == "remote"
    assert isolated.runner_profile() == "isolated-xpra"
    assert not isolated.curated
    assert not isolated.host_framebuffer_proof


def test_screenshot_portal_preauth_seeds_all_generic_app_ids() -> None:
    module = load_screenshot_preauth_module()

    assert module.screenshot_permissions() == {"": ["yes"], "desktop": ["yes"]}
    assert module.missing_screenshot_permissions({"": ["yes"], "desktop": ["yes"]}) == []
    assert module.missing_screenshot_permissions({"": ["yes"]}) == ["desktop"]


def test_testing_vm_host_framebuffer_proof_membership_comes_from_runner_registry() -> None:
    descriptors = run_gui_testing_vm_smoke.VM_PROFILE_DESCRIPTORS

    assert set(run_gui_testing_vm_smoke.HOST_FRAMEBUFFER_PROOF_RUNNERS) == {
        "kwin-system-install",
        "cosmic-patched-host-proof",
        "cosmic-transparent-xcursor-host-proof",
    }
    host_proof = {
        name for name, descriptor in descriptors.items() if descriptor.host_framebuffer_proof
    }
    assert host_proof == {
        "kde-kwin-effect-system-install",
        "cosmic-patched-cursor-host-proof",
        "cosmic-transparent-xcursor-host-proof",
    }


def test_testing_vm_profile_listing_reports_metadata() -> None:
    listing = run_gui_testing_vm_smoke.format_profile_listing()

    lines = listing.splitlines()
    assert lines[0] == "profile  dispatch  curated  host-framebuffer-proof"
    assert len(lines) == len(run_gui_testing_vm_smoke.VM_PROFILE_DESCRIPTORS) + 1
    assert "kde-kwin-effect-system-install  kwin-system-install  -  host-proof" in lines
    assert "wayland-pointer-scaled  remote  -  -" in lines
    assert "session-env  remote  curated  -" in lines
    assert "text-readback  remote  curated  -" in lines
    assert "desktop-smoke  remote  -  -" in lines
    assert "curated  curated  -  -" in lines
    assert "gnome  remote  -  -" in lines


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


def _patch_curated_main_dependencies(
    monkeypatch: pytest.MonkeyPatch,
    *,
    reset_calls: list[None],
    kde_preauth_calls: list[None],
    gnome_preauth_calls: list[None],
) -> None:
    monkeypatch.setattr(run_gui_testing_vm_smoke, "build_host_runtime_artifacts", lambda: None)
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "sync_checkout",
        lambda ssh_target, port, ssh_options, remote_root: None,
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "reset_guest_sky_cua_processes",
        lambda ssh_target, port, ssh_options: reset_calls.append(None),
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
        "preauthorize_kde_remote_desktop",
        lambda ssh_target, port, ssh_options, remote_root, *, wayland_display, desktop_env: (
            kde_preauth_calls.append(None)
        ),
    )
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "preauthorize_gnome_remote_desktop",
        lambda ssh_target, port, ssh_options, remote_root, *, wayland_display, desktop_env: (
            gnome_preauth_calls.append(None)
        ),
    )


def test_testing_vm_runner_main_dispatches_curated_profile_set(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    reset_calls: list[None] = []
    kde_preauth_calls: list[None] = []
    gnome_preauth_calls: list[None] = []
    executed_profiles: list[str] = []

    _patch_curated_main_dependencies(
        monkeypatch,
        reset_calls=reset_calls,
        kde_preauth_calls=kde_preauth_calls,
        gnome_preauth_calls=gnome_preauth_calls,
    )

    def fake_remote_profile(
        ssh_target: str,
        port: int,
        ssh_options: list[str],
        remote_root: Path,
        profile: str,
        *,
        headed: bool = False,
        wayland_display: str = "wayland-0",
        desktop_env: str = "",
        sync_codex_settings: bool = False,
    ) -> int:
        executed_profiles.append(profile)
        return 0

    monkeypatch.setattr(run_gui_testing_vm_smoke, "run_remote_profile", fake_remote_profile)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "run_gui_testing_vm_smoke.py",
            "--host",
            "127.0.0.1",
            "--profile",
            "curated",
            "--desktop-env",
            "KDE",
        ],
    )

    assert run_gui_testing_vm_smoke.main() == 0
    assert tuple(executed_profiles) == run_gui_testing_vm_smoke.CURATED_PROFILE_NAMES
    # KDE RemoteDesktop preauthorization runs once for the whole curated set,
    # not once per member profile.
    assert len(kde_preauth_calls) == 1
    assert len(gnome_preauth_calls) == 0
    # One reset before the first profile plus one between each pair.
    assert len(reset_calls) == len(run_gui_testing_vm_smoke.CURATED_PROFILE_NAMES)


def test_testing_vm_runner_curated_profile_set_aggregates_failures(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    reset_calls: list[None] = []
    kde_preauth_calls: list[None] = []
    gnome_preauth_calls: list[None] = []
    executed_profiles: list[str] = []

    _patch_curated_main_dependencies(
        monkeypatch,
        reset_calls=reset_calls,
        kde_preauth_calls=kde_preauth_calls,
        gnome_preauth_calls=gnome_preauth_calls,
    )

    def fake_remote_profile(
        ssh_target: str,
        port: int,
        ssh_options: list[str],
        remote_root: Path,
        profile: str,
        *,
        headed: bool = False,
        wayland_display: str = "wayland-0",
        desktop_env: str = "",
        sync_codex_settings: bool = False,
    ) -> int:
        executed_profiles.append(profile)
        return 3 if profile == "session-env" else 0

    monkeypatch.setattr(run_gui_testing_vm_smoke, "run_remote_profile", fake_remote_profile)
    monkeypatch.setattr(
        sys,
        "argv",
        ["run_gui_testing_vm_smoke.py", "--host", "127.0.0.1", "--profile", "curated"],
    )

    assert run_gui_testing_vm_smoke.main() == 1
    # A failing member does not stop the remaining curated lanes.
    assert tuple(executed_profiles) == run_gui_testing_vm_smoke.CURATED_PROFILE_NAMES


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


def test_testing_vm_kwin_host_framebuffer_prefers_native_requested_point() -> None:
    point = run_gui_testing_vm_smoke.requested_point_from_summary(
        {
            "requested_point": {"x": 720.0, "y": 450.0},
            "requested_native_point": {"x": 640.0, "y": 400.0},
        }
    )

    assert point == (640.0, 400.0)


def test_testing_vm_kwin_host_framebuffer_requested_point_fallbacks() -> None:
    assert run_gui_testing_vm_smoke.requested_point_from_summary(
        {"requested_point": {"x": 720.0, "y": 450.0}}
    ) == (720.0, 450.0)
    assert (
        run_gui_testing_vm_smoke.requested_point_from_summary({})
        == run_gui_testing_vm_smoke.KWIN_EFFECT_SYSTEM_POINT
    )


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
    stamps: list[Path] = []

    def fake_run(command: list[str], *, cwd: Path, check: bool) -> subprocess.CompletedProcess[str]:
        assert cwd == run_gui_testing_vm_smoke.REPO_ROOT
        assert check is True
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(run_gui_testing_vm_smoke.subprocess, "run", fake_run)
    monkeypatch.setattr(
        run_gui_testing_vm_smoke,
        "write_build_stamp",
        lambda path: stamps.append(path) or path.with_name(path.name + ".buildstamp.json"),
    )

    run_gui_testing_vm_smoke.build_host_runtime_artifacts()

    assert stamps == [run_gui_testing_vm_smoke.REPO_ROOT / "target" / "release" / "sky-cua-client"]
    assert commands == [
        [
            "cargo",
            "build",
            "--release",
            "-p",
            "sky-cua-chrome-host",
            "-p",
            "sky-cua-client",
            "-p",
            "sky-cua-service",
            "-p",
            "sky-cua-cosmic-helper",
            "-p",
            "sky-cua-input-helper",
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
    assert "auth.json skycua@testing-vm:.codex/auth.json" not in command_text
    assert "config.toml skycua@testing-vm:.codex/config.toml" in command_text
    assert "browser/config.toml" in command_text
    assert "--delete" in command_text
    assert "plugins/" in command_text


def test_codex_cua_profile_and_openai_bundled_compat_wiring() -> None:
    assert "codex-cua" in run_gui_testing_vm_smoke.PROFILES
    descriptor = run_gui_testing_vm_smoke.VM_PROFILE_DESCRIPTORS["codex-cua"]
    assert descriptor.dispatch == run_gui_testing_vm_smoke.CODEX_CUA_JUDGE_DISPATCH
    # The compat marketplace is staged for codex-cua and the `all` sequence.
    assert {"codex-cua", "all"} == run_gui_testing_vm_smoke.OPENAI_BUNDLED_PROFILES
    # The runner stage target and the profile-script env must agree on the path.
    assert ".cache/sky-cua/openai-bundled" in run_gui_testing_vm_smoke.OPENAI_BUNDLED_REMOTE_REL
    profile = (
        Path(__file__).resolve().parents[1] / "scripts" / "testing-vm" / "profiles" / "codex-cua.sh"
    ).read_text(encoding="utf-8")
    assert "SKY_CUA_OPENAI_BUNDLED_RESOURCE_ROOT" in profile
    assert "${HOME}/.cache/sky-cua/openai-bundled" in profile
