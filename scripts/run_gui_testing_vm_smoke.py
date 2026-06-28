#!/usr/bin/env python3
"""Build, sync, and run GUI desktop smoke profiles on the Arch testing VM."""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import time
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Protocol

from deploy_freshness import write_build_stamp
from live_agent_cursor_kde_smoke import MarkerProbe, probe_marker  # type: ignore[import-not-found]

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REMOTE_ROOT = Path("/workspace")
DEFAULT_CODEX_HOME = Path.home() / ".codex"
KWIN_EFFECT_SYSTEM_POINT = (420.0, 260.0)
COSMIC_HOST_AGENT_POINT = (360.0, 260.0)
COSMIC_HOST_OBSERVED_AGENT_MARKER_POINT = (360.0, 295.0)
COSMIC_HOST_RESTORED_CURSOR_POINT = (160.0, 171.0)
RUNTIME_PACKAGES = (
    "sky-cua-chrome-host",
    "sky-cua-client",
    "sky-cua-service",
    "sky-cua-cosmic-helper",
    "sky-cua-input-helper",
    "sky-cua-overlay-host",
)
RSYNC_EXCLUDES = (
    ".git/",
    ".venv/",
    "artifacts/",
    "dist/",
    "target/debug/",
    "target/doc/",
    "target/tmp/",
)
CODEX_SETTING_PATHS = (
    "config.json",
    "config.toml",
    "keybindings.json",
    "browser/config.toml",
)
CODEX_SETTING_DIRS = ("plugins", "skills")


@dataclass(frozen=True)
class VmProfileDescriptor:
    name: str
    dispatch: str = "remote"
    remote_profile: str | None = None
    # Membership in the trimmed pre-merge curated set executed by
    # `--profile curated`. The curated set is session-agnostic: every member
    # must be able to pass against whichever real desktop session the VM is
    # currently booted into. Session-specific host proofs (KWin system
    # install, COSMIC cursor proofs, scaled output) stay outside the curated
    # set and remain per-session feature/release gates.
    curated: bool = False
    preauthorize_gnome_remote_desktop: bool = False
    preauthorize_kde_remote_desktop: bool = False
    preauthorize_screenshot_portal: bool = False

    def runner_profile(self) -> str:
        return self.remote_profile or self.name

    @property
    def host_framebuffer_proof(self) -> bool:
        # Membership comes from the runner registry keys, so a host-proof
        # profile cannot exist without a registered host-side runner.
        return self.dispatch in HOST_FRAMEBUFFER_PROOF_DISPATCHES


VM_PROFILE_DESCRIPTORS: dict[str, VmProfileDescriptor] = {
    profile.name: profile
    for profile in (
        VmProfileDescriptor("kde-kwin-effect"),
        VmProfileDescriptor(
            "kde-kwin-effect-system-install",
            dispatch="kwin-system-install",
            remote_profile="agent-cursor-kde",
        ),
        VmProfileDescriptor("kde-plasma"),
        VmProfileDescriptor("i3"),
        VmProfileDescriptor(
            "computer-use",
            preauthorize_gnome_remote_desktop=True,
            preauthorize_kde_remote_desktop=True,
        ),
        VmProfileDescriptor("codex-desktop", curated=True),
        VmProfileDescriptor("cosmic-helper"),
        VmProfileDescriptor(
            "cosmic-patched-cursor-host-proof",
            dispatch="cosmic-patched-host-proof",
            remote_profile="agent-cursor-cosmic-patched-host-proof",
        ),
        VmProfileDescriptor(
            "cosmic-transparent-xcursor-host-proof",
            dispatch="cosmic-transparent-xcursor-host-proof",
            remote_profile="agent-cursor-cosmic-transparent-xcursor-host-proof",
        ),
        # This lane is a service-backed overlay/screenshot proof that still needs
        # a real Wayland session socket, so keep it outside the headless curated set.
        VmProfileDescriptor(
            "wayland-layer-shell-overlay",
            preauthorize_screenshot_portal=True,
        ),
        VmProfileDescriptor(
            "wayland-pointer",
            curated=True,
            preauthorize_gnome_remote_desktop=True,
            preauthorize_kde_remote_desktop=True,
        ),
        VmProfileDescriptor(
            "targeted-screenshot",
            preauthorize_gnome_remote_desktop=True,
            preauthorize_kde_remote_desktop=True,
            preauthorize_screenshot_portal=True,
        ),
        VmProfileDescriptor(
            "display-screenshot",
            preauthorize_gnome_remote_desktop=True,
            preauthorize_kde_remote_desktop=True,
            preauthorize_screenshot_portal=True,
        ),
        VmProfileDescriptor("wayland-pointer-scaled"),
        VmProfileDescriptor(
            "session-env",
            curated=True,
            preauthorize_gnome_remote_desktop=True,
            preauthorize_kde_remote_desktop=True,
        ),
        VmProfileDescriptor(
            "text-readback",
            curated=True,
            preauthorize_gnome_remote_desktop=True,
            preauthorize_kde_remote_desktop=True,
        ),
        # The full strict-capture direct smoke requires live PipeWire frames,
        # which COSMIC does not deliver headless; it stays a per-session lane
        # outside the curated set.
        VmProfileDescriptor(
            "desktop-smoke",
            preauthorize_gnome_remote_desktop=True,
            preauthorize_kde_remote_desktop=True,
        ),
        VmProfileDescriptor("gnome"),
        VmProfileDescriptor("cosmic"),
        VmProfileDescriptor("hyprland"),
        VmProfileDescriptor("opencode-mcp", preauthorize_screenshot_portal=True),
        VmProfileDescriptor("pi-mcp", preauthorize_screenshot_portal=True),
        # Heavy single-run codex tool-use profile. The deterministic coverage gate
        # runs in the VM; the host-side judge runs after the remote run (host gpt-5.5
        # auth is not available in the VM), hence the dedicated dispatch.
        VmProfileDescriptor(
            "codex-cua",
            dispatch="codex-cua-judge",
            preauthorize_screenshot_portal=True,
            preauthorize_kde_remote_desktop=True,
            preauthorize_gnome_remote_desktop=True,
        ),
        VmProfileDescriptor("curated", dispatch="curated"),
        VmProfileDescriptor(
            "all",
            preauthorize_gnome_remote_desktop=True,
            preauthorize_kde_remote_desktop=True,
            preauthorize_screenshot_portal=True,
        ),
    )
}

CURATED_PROFILE_NAMES = tuple(
    descriptor.name for descriptor in VM_PROFILE_DESCRIPTORS.values() if descriptor.curated
)

PROFILES = tuple(VM_PROFILE_DESCRIPTORS)
AGENT_AUTH_PROFILES = {"all", "opencode-mcp", "pi-mcp"}
AGENT_AUTH_ENV_KEYS = (
    "FIREWORKS_API_KEY",
    "OPENAI_API_KEY",
    "OPENCODE_API_KEY",
    "MOONSHOT_API_KEY",
    "CONTEXT7_API_KEY",
    "SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG",
    "SKY_CUA_SMOKE_OPENCODE_MODEL",
    "SKY_CUA_SMOKE_PI_MODEL",
)
MCP_LAUNCH_POLICY_ENV_KEYS = (
    "SKY_CUA_BROWSER_EVAL",
    "SKY_CUA_MODEL_SUPPORTS_IMAGES",
)
# Host dir whose `plugins/openai-bundled` holds the OpenAI bundled marketplace
# (the production compat surface). Matches build_plugin.py's default parent. When
# present, codex-cua runs against `computer-use@openai-bundled` instead of the
# `sky-cua@local` dev fallback; the tool surface is identical (same sky-cua-client
# server + skills), so only the plugin identity and resolution path change.
DEFAULT_OPENAI_BUNDLED_RESOURCE_ROOT = (
    REPO_ROOT.parent / "codex-desktop-linux" / "codex-app" / "resources"
)
# Fixed VM-relative location the compat resources are staged to; codex-cua.sh
# points SKY_CUA_OPENAI_BUNDLED_RESOURCE_ROOT here.
OPENAI_BUNDLED_REMOTE_REL = ".cache/sky-cua/openai-bundled/plugins/openai-bundled"
OPENAI_BUNDLED_PROFILES = {"codex-cua", "all"}
# OpenCode stores its zen API key here, keyed by provider. The pi smoke config
# authenticates its `opencode` provider via `$OPENCODE_API_KEY`, so we source the
# key from OpenCode's own store when the caller did not export it, instead of
# requiring a manual `OPENCODE_API_KEY=...` on every pi/all run.
OPENCODE_AUTH_JSON = Path.home() / ".local" / "share" / "opencode" / "auth.json"


def resolve_opencode_api_key() -> str | None:
    """Return OpenCode's stored `opencode` (zen) API key, or None if absent."""
    try:
        data = json.loads(OPENCODE_AUTH_JSON.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    entry = data.get("opencode") if isinstance(data, dict) else None
    if isinstance(entry, dict):
        key = entry.get("key")
        if isinstance(key, str) and key:
            return key
    return None


@dataclass(frozen=True)
class RemoteRunner:
    ssh_target: str
    port: int
    ssh_options: list[str]
    remote_root: Path
    wayland_display: str = "wayland-0"
    desktop_env: str = ""

    def run(self, script: str, *, check: bool = False) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            [*ssh_base_command(self.port, self.ssh_options), self.ssh_target, script],
            cwd=REPO_ROOT,
            check=check,
        )

    def runtime_script(self, command: list[str]) -> str:
        desktop_exports = desktop_environment_exports(self.desktop_env)
        return (
            f"cd {shlex.quote(str(self.remote_root))} && "
            'runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" && '
            'if [ ! -d "$runtime_dir" ]; then runtime_dir=/tmp/sky-cua-runtime; mkdir -p "$runtime_dir"; chmod 700 "$runtime_dir"; fi && '
            'export XDG_RUNTIME_DIR="$runtime_dir" && '
            f'export WAYLAND_DISPLAY="${{WAYLAND_DISPLAY:-{shlex.quote(self.wayland_display)}}}" && '
            "export XDG_SESSION_TYPE=wayland && "
            f"{desktop_exports}"
            f"{self.input_helper_start_script()}"
            f"{shlex.join(command)}"
        )

    def input_helper_start_script(self) -> str:
        helper = shlex.quote(str(self.remote_root / "target" / "release" / "sky-cua-input-helper"))
        return (
            f"if [ -x {helper} ]; then "
            "sudo systemctl stop sky-cua-input-helper.service >/dev/null 2>&1 || true; "
            "sudo systemctl reset-failed sky-cua-input-helper.service >/dev/null 2>&1 || true; "
            "sudo systemd-run --unit=sky-cua-input-helper --collect "
            "--setenv=SKY_CUA_INPUT_HELPER_SOCKET=/run/sky-cua/input-helper.sock "
            "--setenv=SKY_CUA_INPUT_HELPER_SOCKET_MODE=0660 "
            "--setenv=SKY_CUA_INPUT_HELPER_SOCKET_GROUP=input "
            f"{helper} serve >/dev/null; "
            "helper_ready=0; "
            "for _ in $(seq 1 40); do "
            "[ -S /run/sky-cua/input-helper.sock ] && helper_ready=1 && break; "
            "sleep 0.05; "
            "done; "
            'if [ "$helper_ready" -ne 1 ]; then '
            "printf 'sky-cua input helper socket did not appear\\n' >&2; exit 1; "
            "fi; "
            "fi && "
        )


@dataclass(frozen=True)
class CosmicHostFramebufferProofProfile:
    remote_profile_dir: str
    local_artifact_dir_name: str
    mode: str
    hide_reason: str


@dataclass(frozen=True)
class CosmicHostFramebufferProofPaths:
    remote_artifact_dir: Path
    local_artifact_dir: Path
    before_path: Path
    visible_path: Path
    hidden_path: Path

    @property
    def stdout_path(self) -> Path:
        return self.local_artifact_dir / "remote.stdout.log"

    @property
    def stderr_path(self) -> Path:
        return self.local_artifact_dir / "remote.stderr.log"

    @property
    def host_summary_path(self) -> Path:
        return self.local_artifact_dir / "host-summary.json"


@dataclass(frozen=True)
class KwinHostFramebufferProofProfile:
    remote_profile_dir: str
    local_artifact_dir_parts: tuple[str, ...]
    mode: str


@dataclass(frozen=True)
class KwinHostFramebufferProofPaths:
    remote_artifact_dir: Path
    local_artifact_dir: Path
    before_path: Path
    after_path: Path

    @property
    def stdout_path(self) -> Path:
        return self.local_artifact_dir / "remote.stdout.log"

    @property
    def stderr_path(self) -> Path:
        return self.local_artifact_dir / "remote.stderr.log"

    @property
    def host_summary_path(self) -> Path:
        return self.local_artifact_dir / "host-summary.json"


COSMIC_PATCHED_CURSOR_HOST_PROOF = CosmicHostFramebufferProofProfile(
    remote_profile_dir="agent-cursor-cosmic-patched-host-proof",
    local_artifact_dir_name="cosmic-framebuffer-cursor-proof",
    mode="cosmic-patched-cursor-host-framebuffer",
    hide_reason="cosmic-host-framebuffer-proof",
)
COSMIC_TRANSPARENT_XCURSOR_HOST_PROOF = CosmicHostFramebufferProofProfile(
    remote_profile_dir="agent-cursor-cosmic-transparent-xcursor-host-proof",
    local_artifact_dir_name="cosmic-transparent-xcursor-cursor-proof",
    mode="cosmic-transparent-xcursor-host-framebuffer",
    hide_reason="cosmic-transparent-xcursor-host-proof",
)
KWIN_EFFECT_SYSTEM_INSTALL_HOST_PROOF = KwinHostFramebufferProofProfile(
    remote_profile_dir="agent-cursor-kde",
    local_artifact_dir_parts=("kde-framebuffer-cursor-proof", "kwin-system-install"),
    mode="kde-kwin-effect-system-install-host-framebuffer",
)


def _sync_host_settings(
    ssh_target: str,
    port: int,
    script_name: str,
) -> None:
    """Run a testing-vm sync script with the standard environment."""
    script_path = REPO_ROOT / "scripts" / "testing-vm" / script_name
    env = os.environ.copy()
    env["SKY_CUA_TESTING_VM_HOST"] = ssh_target.split("@")[1]
    env["SKY_CUA_TESTING_VM_PORT"] = str(port)
    env["SKY_CUA_TESTING_VM_USER"] = ssh_target.split("@")[0]
    subprocess.run(
        [str(script_path)],
        cwd=REPO_ROOT,
        check=True,
        env=env,
    )


def sync_opencode_settings(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
) -> None:
    _sync_host_settings(ssh_target, port, "sync-opencode-to-vm.sh")


def sync_pi_settings(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
) -> None:
    _sync_host_settings(ssh_target, port, "sync-pi-to-vm.sh")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run sky-cua GUI desktop smoke profiles on the Arch testing VM."
    )
    parser.add_argument("--host", help="SSH host name or address for the VM.")
    parser.add_argument(
        "--list-profiles",
        action="store_true",
        help="Print the profile registry with dispatch, curated-set membership, and host-framebuffer-proof metadata, then exit.",
    )
    parser.add_argument("--user", default="skycua", help="SSH user for the VM.")
    parser.add_argument("--port", type=int, default=22, help="SSH port for the VM.")
    parser.add_argument(
        "--profile",
        choices=PROFILES,
        default="computer-use",
        help=(
            "Desktop profile to execute inside the VM. `curated` runs the "
            "trimmed pre-merge profile set in sequence against the current "
            "guest session."
        ),
    )
    parser.add_argument(
        "--remote-root",
        type=Path,
        default=DEFAULT_REMOTE_ROOT,
        help="Guest checkout path. Defaults to /workspace.",
    )
    parser.add_argument(
        "--skip-host-build",
        action="store_true",
        help="Do not build host Rust runtime artifacts before syncing.",
    )
    parser.add_argument(
        "--skip-sync",
        action="store_true",
        help="Run the remote profile without syncing the checkout first.",
    )
    parser.add_argument(
        "--openai-bundled-resource-root",
        type=Path,
        default=DEFAULT_OPENAI_BUNDLED_RESOURCE_ROOT,
        help=(
            "Host dir whose plugins/openai-bundled holds the compat marketplace; "
            "synced to the VM so codex-cua tests the computer-use@openai-bundled surface "
            f"(default: {DEFAULT_OPENAI_BUNDLED_RESOURCE_ROOT})."
        ),
    )
    parser.add_argument(
        "--skip-openai-bundled-sync",
        action="store_true",
        help="Do not stage the openai-bundled compat resources for codex-cua/all.",
    )
    parser.add_argument(
        "--skip-codex-settings",
        action="store_true",
        help="Deprecated compatibility flag; Codex settings are copied only when --sync-codex-settings is set and this flag is absent.",
    )
    parser.add_argument(
        "--sync-codex-settings",
        action="store_true",
        help="Copy selected non-auth host ~/.codex settings into the VM; use VM-local device login for auth.",
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=DEFAULT_CODEX_HOME,
        help="Host Codex settings directory to sync into the VM.",
    )
    parser.add_argument(
        "--sync-opencode-settings",
        action="store_true",
        help="Copy host ~/.config/opencode settings into the VM for OpenCode smokes.",
    )
    parser.add_argument(
        "--sync-pi-settings",
        action="store_true",
        help="Copy host ~/.pi settings into the VM for Pi smokes.",
    )
    parser.add_argument(
        "--headed",
        action="store_true",
        help="Pass --headed to legacy nested visual-debug profiles.",
    )
    parser.add_argument(
        "--wayland-display",
        default="wayland-0",
        help="Wayland socket to use for real VM desktop-session smokes.",
    )
    parser.add_argument(
        "--desktop-env",
        default="",
        help=(
            "Optional XDG_CURRENT_DESKTOP value for SSH-launched real-session smokes, "
            "for example COSMIC, KDE, GNOME, or Hyprland."
        ),
    )
    parser.add_argument(
        "--ssh-option",
        action="append",
        default=[],
        help="Additional -o option for ssh/rsync, for example StrictHostKeyChecking=no.",
    )
    parser.add_argument(
        "--skip-gnome-remote-desktop-preauth",
        action="store_true",
        help="Do not preseed GNOME RemoteDesktop portal restore tokens before real-session smokes.",
    )
    parser.add_argument(
        "--skip-kde-remote-desktop-preauth",
        action="store_true",
        help="Do not preseed KDE RemoteDesktop portal authorization before real-session smokes.",
    )
    parser.add_argument(
        "--vm-name",
        default="testing-vm",
        help="libvirt VM domain name for host-framebuffer proof profiles.",
    )
    parser.add_argument(
        "--libvirt-uri",
        default="qemu:///session",
        help="libvirt URI for host-framebuffer proof profiles.",
    )
    args = parser.parse_args()

    if args.list_profiles:
        print(format_profile_listing())
        return 0
    if not args.host:
        parser.error("--host is required unless --list-profiles is set")

    profile = VM_PROFILE_DESCRIPTORS[args.profile]
    ssh_target = f"{args.user}@{args.host}"
    remote_root = args.remote_root
    if not args.skip_host_build:
        build_host_runtime_artifacts()
    if not args.skip_sync:
        sync_checkout(ssh_target, args.port, args.ssh_option, remote_root)
    if args.sync_codex_settings and not args.skip_codex_settings:
        sync_codex_settings(ssh_target, args.port, args.ssh_option, args.codex_home)
    if args.sync_opencode_settings:
        sync_opencode_settings(ssh_target, args.port, args.ssh_option)
    if args.sync_pi_settings:
        sync_pi_settings(ssh_target, args.port, args.ssh_option)
    if args.profile in OPENAI_BUNDLED_PROFILES and not args.skip_openai_bundled_sync:
        sync_openai_bundled_resources(
            ssh_target, args.port, args.ssh_option, args.openai_bundled_resource_root
        )
    reset_guest_sky_cua_processes(ssh_target, args.port, args.ssh_option)
    wake_guest_display(ssh_target, args.port, args.ssh_option)
    if args.desktop_env:
        refresh_guest_portal_stack(
            ssh_target,
            args.port,
            args.ssh_option,
            wayland_display=args.wayland_display,
            desktop_env=args.desktop_env,
        )
    curated_mode = profile.dispatch == "curated"
    profiles_to_run = curated_profiles() if curated_mode else (profile,)
    preauthorize_for_profiles(
        profiles_to_run,
        ssh_target,
        args.port,
        args.ssh_option,
        remote_root,
        wayland_display=args.wayland_display,
        desktop_env=args.desktop_env,
        skip_gnome_remote_desktop_preauth=args.skip_gnome_remote_desktop_preauth,
        skip_kde_remote_desktop_preauth=args.skip_kde_remote_desktop_preauth,
    )
    sync_codex = args.sync_codex_settings and not args.skip_codex_settings
    if curated_mode:
        return run_curated_profiles(
            profiles_to_run,
            ssh_target,
            args.port,
            args.ssh_option,
            remote_root,
            headed=args.headed,
            wayland_display=args.wayland_display,
            desktop_env=args.desktop_env,
            sync_codex_settings=sync_codex,
            vm_name=args.vm_name,
            libvirt_uri=args.libvirt_uri,
        )
    return execute_profile(
        profile,
        ssh_target,
        args.port,
        args.ssh_option,
        remote_root,
        headed=args.headed,
        wayland_display=args.wayland_display,
        desktop_env=args.desktop_env,
        sync_codex_settings=sync_codex,
        vm_name=args.vm_name,
        libvirt_uri=args.libvirt_uri,
    )


def curated_profiles() -> tuple[VmProfileDescriptor, ...]:
    return tuple(VM_PROFILE_DESCRIPTORS[name] for name in CURATED_PROFILE_NAMES)


def preauthorize_for_profiles(
    profiles: tuple[VmProfileDescriptor, ...],
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_root: Path,
    *,
    wayland_display: str,
    desktop_env: str,
    skip_gnome_remote_desktop_preauth: bool,
    skip_kde_remote_desktop_preauth: bool,
) -> None:
    """Run each required preauthorization once for the set of profiles."""
    if not skip_gnome_remote_desktop_preauth and any(
        should_preauthorize_gnome_remote_desktop(profile, desktop_env) for profile in profiles
    ):
        preauthorize_gnome_remote_desktop(
            ssh_target,
            port,
            ssh_options,
            remote_root,
            wayland_display=wayland_display,
            desktop_env=desktop_env,
        )
    if not skip_kde_remote_desktop_preauth and any(
        should_preauthorize_kde_remote_desktop(profile, desktop_env) for profile in profiles
    ):
        preauthorize_kde_remote_desktop(
            ssh_target,
            port,
            ssh_options,
            remote_root,
            wayland_display=wayland_display,
            desktop_env=desktop_env,
        )
    if any(profile.preauthorize_screenshot_portal for profile in profiles):
        preauthorize_screenshot_portal(
            ssh_target,
            port,
            ssh_options,
            remote_root,
            wayland_display=wayland_display,
            desktop_env=desktop_env,
        )


def execute_profile(
    profile: VmProfileDescriptor,
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_root: Path,
    *,
    headed: bool,
    wayland_display: str,
    desktop_env: str,
    sync_codex_settings: bool,
    vm_name: str,
    libvirt_uri: str,
) -> int:
    if profile.dispatch == CODEX_CUA_JUDGE_DISPATCH:
        return run_codex_cua_judge_profile(
            ssh_target,
            port,
            ssh_options,
            remote_root,
            headed=headed,
            wayland_display=wayland_display,
            desktop_env=desktop_env,
            sync_codex_settings=sync_codex_settings,
        )
    if profile.host_framebuffer_proof:
        return run_host_framebuffer_proof_profile(
            profile,
            ssh_target,
            port,
            ssh_options,
            remote_root,
            wayland_display=wayland_display,
            desktop_env=desktop_env,
            sync_codex_settings=sync_codex_settings,
            vm_name=vm_name,
            libvirt_uri=libvirt_uri,
        )
    return run_remote_profile(
        ssh_target,
        port,
        ssh_options,
        remote_root,
        profile.runner_profile(),
        headed=headed,
        wayland_display=wayland_display,
        desktop_env=desktop_env,
        sync_codex_settings=sync_codex_settings,
    )


def run_curated_profiles(
    profiles: tuple[VmProfileDescriptor, ...],
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_root: Path,
    *,
    headed: bool,
    wayland_display: str,
    desktop_env: str,
    sync_codex_settings: bool,
    vm_name: str,
    libvirt_uri: str,
) -> int:
    """Run the trimmed pre-merge curated profile set in registry order.

    Each member runs against the currently selected guest session after a
    guest process reset, so a failed or leaky profile cannot poison the next
    lane. The aggregate exits nonzero when any member fails; the per-profile
    summary keeps individual results honest.
    """
    results: list[tuple[str, int]] = []
    for index, profile in enumerate(profiles):
        if index > 0:
            reset_guest_sky_cua_processes(ssh_target, port, ssh_options)
        print(f"curated profile starting: {profile.name}", flush=True)
        returncode = execute_profile(
            profile,
            ssh_target,
            port,
            ssh_options,
            remote_root,
            headed=headed,
            wayland_display=wayland_display,
            desktop_env=desktop_env,
            sync_codex_settings=sync_codex_settings,
            vm_name=vm_name,
            libvirt_uri=libvirt_uri,
        )
        results.append((profile.name, returncode))
        print(f"curated profile finished: {profile.name} exit={returncode}", flush=True)
    print("curated summary:")
    for name, returncode in results:
        print(f"  {name}: {'ok' if returncode == 0 else f'exit {returncode}'}")
    return 0 if all(returncode == 0 for _, returncode in results) else 1


class HostFramebufferProofRunner(Protocol):
    def __call__(
        self,
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
    ) -> int: ...


def _kwin_system_install_host_proof(
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
    return run_remote_kwin_effect_system_install_profile(
        ssh_target,
        port,
        ssh_options,
        remote_root,
        wayland_display=wayland_display,
        desktop_env=desktop_env,
        sync_codex_settings=sync_codex_settings,
        vm_name=vm_name,
        libvirt_uri=libvirt_uri,
    )


def _cosmic_patched_host_proof(
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
    # The COSMIC host-proof runners default the desktop env and do not sync
    # Codex settings; they run an unauthenticated cursor proof.
    del sync_codex_settings
    return run_remote_cosmic_patched_cursor_host_proof_profile(
        ssh_target,
        port,
        ssh_options,
        remote_root,
        wayland_display=wayland_display,
        desktop_env=desktop_env or "COSMIC",
        vm_name=vm_name,
        libvirt_uri=libvirt_uri,
    )


def _cosmic_transparent_xcursor_host_proof(
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
    del sync_codex_settings
    return run_remote_cosmic_transparent_xcursor_host_proof_profile(
        ssh_target,
        port,
        ssh_options,
        remote_root,
        wayland_display=wayland_display,
        desktop_env=desktop_env or "COSMIC",
        vm_name=vm_name,
        libvirt_uri=libvirt_uri,
    )


# Host-side runners that capture the VM framebuffer through libvirt instead of
# the generic remote profile path. The registry keys are the single source of
# host-framebuffer-proof dispatch membership.
HOST_FRAMEBUFFER_PROOF_RUNNERS: dict[str, HostFramebufferProofRunner] = {
    "kwin-system-install": _kwin_system_install_host_proof,
    "cosmic-patched-host-proof": _cosmic_patched_host_proof,
    "cosmic-transparent-xcursor-host-proof": _cosmic_transparent_xcursor_host_proof,
}
HOST_FRAMEBUFFER_PROOF_DISPATCHES = frozenset(HOST_FRAMEBUFFER_PROOF_RUNNERS)


def run_host_framebuffer_proof_profile(
    profile: VmProfileDescriptor,
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
    runner = HOST_FRAMEBUFFER_PROOF_RUNNERS[profile.dispatch]
    return runner(
        ssh_target,
        port,
        ssh_options,
        remote_root,
        wayland_display=wayland_display,
        desktop_env=desktop_env,
        sync_codex_settings=sync_codex_settings,
        vm_name=vm_name,
        libvirt_uri=libvirt_uri,
    )


CODEX_CUA_JUDGE_DISPATCH = "codex-cua-judge"


def pull_remote_file(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_path: Path,
    local_path: Path,
) -> bool:
    """Copy a single remote file to a local path via ssh cat. Returns success."""
    completed = subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, "cat", str(remote_path)],
        cwd=REPO_ROOT,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        return False
    local_path.parent.mkdir(parents=True, exist_ok=True)
    local_path.write_bytes(completed.stdout)
    return True


def run_codex_cua_judge_profile(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_root: Path,
    *,
    headed: bool,
    wayland_display: str,
    desktop_env: str,
    sync_codex_settings: bool,
) -> int:
    """Run the codex CUA smoke in the VM, then judge its tool use on the host.

    The VM produces the transcript + deterministic coverage summary; the host
    pulls them back and runs the gpt-5.5 judge (host-only auth). Overall success
    requires both the deterministic VM gate and the judge to pass, but the judge
    runs even when the VM gate failed so a triage list is always produced.
    """
    remote_exit = run_remote_profile(
        ssh_target,
        port,
        ssh_options,
        remote_root,
        "codex-cua",
        headed=headed,
        wayland_display=wayland_display,
        desktop_env=desktop_env,
        sync_codex_settings=sync_codex_settings,
    )

    local_dir = REPO_ROOT / "artifacts" / "codex-cua-judge" / timestamp_utc()
    local_dir.mkdir(parents=True, exist_ok=True)
    summary_path = local_dir / "host-summary.json"

    latest_marker = remote_root / "artifacts" / "codex-e2e" / "codex-cua" / "latest.json"
    latest = read_remote_json(ssh_target, port, ssh_options, latest_marker)
    remote_artifact = latest.get("artifact_dir") if latest else None
    if not isinstance(remote_artifact, str):
        write_host_summary(
            summary_path,
            {
                "profile": "codex-cua",
                "remote_exit": remote_exit,
                "judge": "skipped: no remote artifacts found",
                "ok": False,
            },
        )
        print("codex-cua: no remote artifacts to judge; inspect the VM run", flush=True)
        return remote_exit or 1

    remote_artifact_dir = Path(remote_artifact)
    # latest.json carries the codex *exec* exit (0 when codex itself ran). The
    # authoritative deterministic-gate result is the smoke's overall exit
    # (remote_exit), which is nonzero when required tools/operations are missing or
    # a tool errored. Gate on remote_exit; pass the codex exit to the judge as context.
    codex_exit = latest.get("exit_code") if latest else None
    codex_exit = codex_exit if isinstance(codex_exit, int) else remote_exit

    pulled = {
        name: pull_remote_file(
            ssh_target, port, ssh_options, remote_artifact_dir / name, local_dir / name
        )
        for name in ("codex-output.jsonl", "coverage-summary.json", "last-message.json")
    }
    if not all(pulled.values()):
        write_host_summary(
            summary_path,
            {
                "profile": "codex-cua",
                "remote_exit": remote_exit,
                "judge": f"skipped: could not pull artifacts {pulled}",
                "ok": False,
            },
        )
        print(f"codex-cua: could not pull artifacts for the judge: {pulled}", flush=True)
        return 1

    # Best-effort: pull Chrome's verbose log and the captured stderr (which carries
    # the sky-cua-chrome-host relay trace) if the run produced them (absent when the
    # browser never launched). Never gates the judge.
    for chrome_log_name in ("chrome-debug.log", "chrome-stderr.log"):
        if pull_remote_file(
            ssh_target,
            port,
            ssh_options,
            remote_artifact_dir / chrome_log_name,
            local_dir / chrome_log_name,
        ):
            print(
                f"codex-cua: pulled {chrome_log_name} -> {local_dir / chrome_log_name}", flush=True
            )

    judge = subprocess.run(
        [
            "python3",
            str(REPO_ROOT / "scripts" / "live_agent_perf_judge.py"),
            "--transcript",
            str(local_dir / "codex-output.jsonl"),
            "--last-message",
            str(local_dir / "last-message.json"),
            "--coverage-summary",
            str(local_dir / "coverage-summary.json"),
            "--artifact-dir",
            str(local_dir / "judge"),
            "--exit-code",
            str(codex_exit),
        ],
        cwd=REPO_ROOT,
        check=False,
    )
    # Overall success requires BOTH the deterministic VM gate (remote_exit) and the
    # judge. The judge ran regardless of the gate so triage is always produced.
    host_ok = remote_exit == 0 and judge.returncode == 0
    write_host_summary(
        summary_path,
        {
            "profile": "codex-cua",
            "gate_exit": remote_exit,
            "codex_exit": codex_exit,
            "judge_exit": judge.returncode,
            "ok": host_ok,
            "artifacts": str(local_dir),
        },
    )
    print(
        f"codex-cua judge summary: gate_exit={remote_exit} codex_exit={codex_exit} "
        f"judge_exit={judge.returncode} ok={host_ok}; artifacts: {local_dir}",
        flush=True,
    )
    return 0 if host_ok else 1


def format_profile_listing() -> str:
    lines = ["profile  dispatch  curated  host-framebuffer-proof"]
    for descriptor in VM_PROFILE_DESCRIPTORS.values():
        lines.append(
            f"{descriptor.name}  {descriptor.dispatch}  "
            f"{'curated' if descriptor.curated else '-'}  "
            f"{'host-proof' if descriptor.host_framebuffer_proof else '-'}"
        )
    return "\n".join(lines)


def build_host_runtime_artifacts() -> None:
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            *(item for package in RUNTIME_PACKAGES for item in ("-p", package)),
        ],
        cwd=REPO_ROOT,
        check=True,
    )
    write_build_stamp(REPO_ROOT / "target" / "release" / "sky-cua-client")
    subprocess.run(
        ["cargo", "build", "-p", "sky-cua-overlay-host"],
        cwd=REPO_ROOT,
        check=True,
    )


def ssh_base_command(port: int, ssh_options: list[str]) -> list[str]:
    control_path = f"/tmp/.ssh-sky-cua-{os.getpid()}-{port}"
    command = [
        "ssh",
        "-p",
        str(port),
        "-o",
        "ControlMaster=auto",
        "-o",
        f"ControlPath={control_path}",
        "-o",
        "ControlPersist=60",
    ]
    for option in ssh_options:
        command.extend(["-o", option])
    return command


def rsync_ssh_command(port: int, ssh_options: list[str]) -> str:
    control_path = f"/tmp/.ssh-sky-cua-{os.getpid()}-{port}"
    parts = [
        "ssh",
        "-p",
        str(port),
        "-o",
        "ControlMaster=auto",
        "-o",
        f"ControlPath={control_path}",
        "-o",
        "ControlPersist=60",
    ]
    for option in ssh_options:
        parts.extend(["-o", option])
    return " ".join(parts)


def sync_checkout(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_root: Path,
) -> None:
    subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, "mkdir", "-p", str(remote_root)],
        cwd=REPO_ROOT,
        check=True,
    )
    command = [
        "rsync",
        "-a",
        "--delete",
        "--human-readable",
        "-e",
        rsync_ssh_command(port, ssh_options),
    ]
    for exclude in RSYNC_EXCLUDES:
        command.extend(["--exclude", exclude])
    command.extend([f"{REPO_ROOT}/", f"{ssh_target}:{remote_root}/"])
    subprocess.run(command, cwd=REPO_ROOT, check=True)


def sync_openai_bundled_resources(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    resource_root: Path,
) -> bool:
    """Stage the OpenAI bundled compat marketplace into the VM for codex-cua.

    Returns True when the resources were synced. When the host source is absent,
    codex-cua falls back to the sky-cua@local dev surface, so this is a best-effort
    step rather than a hard failure.
    """
    source = resource_root.expanduser() / "plugins" / "openai-bundled"
    marketplace = source / ".agents" / "plugins" / "marketplace.json"
    if not marketplace.exists():
        print(
            f"openai-bundled compat source not found at {marketplace}; "
            "codex-cua will fall back to the sky-cua@local surface",
            flush=True,
        )
        return False
    subprocess.run(
        [
            *ssh_base_command(port, ssh_options),
            ssh_target,
            "mkdir",
            "-p",
            OPENAI_BUNDLED_REMOTE_REL,
        ],
        cwd=REPO_ROOT,
        check=True,
    )
    command = [
        "rsync",
        "-a",
        "--delete",
        "--human-readable",
        "-e",
        rsync_ssh_command(port, ssh_options),
        f"{source}/",
        f"{ssh_target}:{OPENAI_BUNDLED_REMOTE_REL}/",
    ]
    subprocess.run(command, cwd=REPO_ROOT, check=True)
    print(
        f"synced openai-bundled compat resources -> {ssh_target}:{OPENAI_BUNDLED_REMOTE_REL}",
        flush=True,
    )
    return True


def sync_codex_settings(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    codex_home: Path,
) -> None:
    codex_home = codex_home.expanduser().resolve()
    if not codex_home.exists():
        return
    # Batch all required remote directories into a single mkdir -p call.
    remote_parents: set[str] = {".codex"}
    for relative_path in CODEX_SETTING_PATHS:
        source_path = codex_home / relative_path
        if source_path.exists():
            remote_parent = str(Path(".codex") / relative_path).rsplit("/", maxsplit=1)[0]
            remote_parents.add(remote_parent)
    for relative_dir in CODEX_SETTING_DIRS:
        source_dir = codex_home / relative_dir
        if source_dir.is_dir():
            remote_parents.add(f".codex/{relative_dir}")
    subprocess.run(
        [
            *ssh_base_command(port, ssh_options),
            ssh_target,
            "mkdir",
            "-p",
            *sorted(remote_parents),
        ],
        cwd=REPO_ROOT,
        check=True,
    )
    for relative_path in CODEX_SETTING_PATHS:
        source_path = codex_home / relative_path
        if source_path.exists():
            remote_path = f"{ssh_target}:.codex/{relative_path}"
            subprocess.run(
                [
                    "rsync",
                    "-aL",
                    "-e",
                    rsync_ssh_command(port, ssh_options),
                    str(source_path),
                    remote_path,
                ],
                cwd=REPO_ROOT,
                check=True,
            )
    for relative_dir in CODEX_SETTING_DIRS:
        source_dir = codex_home / relative_dir
        if source_dir.is_dir():
            subprocess.run(
                [
                    "rsync",
                    "-aL",
                    "--delete",
                    "-e",
                    rsync_ssh_command(port, ssh_options),
                    f"{source_dir}/",
                    f"{ssh_target}:.codex/{relative_dir}/",
                ],
                cwd=REPO_ROOT,
                check=True,
            )


def reset_guest_sky_cua_processes(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
) -> None:
    remote_script = (
        'runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"; '
        "sudo systemctl stop sky-cua-input-helper.service 2>/dev/null || true; "
        "pkill -x sky-cua-service 2>/dev/null || true; "
        "pkill -x sky-cua-input-helper 2>/dev/null || true; "
        "pkill -f '(^|/)sky-cua-overlay-host( |$)' 2>/dev/null || true; "
        "pkill -x sky-cua-overlay 2>/dev/null || true; "
        "pkill -f '(^|/)gtk_pointer_smoke_fixture.py( |$)' 2>/dev/null || true; "
        "pkill -x cosmic-randr 2>/dev/null || true; "
        'rm -f "$runtime_dir/sky-cua/service.sock" "$runtime_dir/sky-cua/agent-cursor.sock" '
        "/tmp/sky-cua-runtime/sky-cua/service.sock "
        "/tmp/sky-cua-runtime/sky-cua/agent-cursor.sock; "
        "rm -rf /tmp/sky-cua-wayland-pointer-*"
    )
    subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
        cwd=REPO_ROOT,
        check=True,
    )


def wake_guest_display(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
) -> None:
    remote_script = (
        "if command -v ydotool >/dev/null 2>&1; then "
        "ydotool mousemove --absolute 20 20 >/dev/null 2>&1 || true; "
        "ydotool key 57:1 57:0 >/dev/null 2>&1 || true; "
        "fi"
    )
    subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
        cwd=REPO_ROOT,
        check=True,
    )


def refresh_guest_portal_stack(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    *,
    wayland_display: str,
    desktop_env: str,
) -> None:
    desktop_exports = desktop_environment_exports(desktop_env)
    remote_script = (
        'runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"; '
        'if [ ! -d "$runtime_dir" ]; then runtime_dir=/tmp/sky-cua-runtime; mkdir -p "$runtime_dir"; chmod 700 "$runtime_dir"; fi; '
        'export XDG_RUNTIME_DIR="$runtime_dir"; '
        f'export WAYLAND_DISPLAY="${{WAYLAND_DISPLAY:-{shlex.quote(wayland_display)}}}"; '
        "export XDG_SESSION_TYPE=wayland; "
        f"{desktop_exports}"
        'mkdir -p "$HOME/.config/systemd/user"; '
        "printf '%s\\n' "
        "'[Unit]' "
        "'Description=Portal service (sky-cua testing VM session override)' "
        "'After=dbus.socket basic.target' "
        "'' "
        "'[Service]' "
        "'Type=dbus' "
        "'BusName=org.freedesktop.portal.Desktop' "
        "'ExecStart=/usr/lib/xdg-desktop-portal' "
        "'Slice=session.slice' "
        '> "$HOME/.config/systemd/user/xdg-desktop-portal.service"; '
        "systemctl --user daemon-reload >/dev/null 2>&1 || true; "
        "systemctl --user stop "
        "xdg-desktop-portal.service "
        "xdg-desktop-portal-gtk.service "
        "xdg-desktop-portal-gnome.service "
        "xdg-desktop-portal-cosmic.service "
        "xdg-desktop-portal-hyprland.service "
        "xdg-desktop-portal-wlr.service "
        "plasma-xdg-desktop-portal-kde.service "
        "xdg-permission-store.service "
        "xdg-document-portal.service >/dev/null 2>&1 || true; "
        "pkill -x xdg-desktop-por 2>/dev/null || true; "
        "pkill -f '(^|/)xdg-desktop-portal( |$)' 2>/dev/null || true; "
        "pkill -f '(^|/)xdg-desktop-portal-[^/ ]+( |$)' 2>/dev/null || true; "
        "pkill -x xdg-document-po 2>/dev/null || true; "
        "pkill -f '(^|/)xdg-document-portal( |$)' 2>/dev/null || true; "
        "pkill -x xdg-permission- 2>/dev/null || true; "
        "pkill -f '(^|/)xdg-permission-store( |$)' 2>/dev/null || true"
    )
    subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
        cwd=REPO_ROOT,
        check=True,
    )


def profile_descriptor(name: str) -> VmProfileDescriptor:
    return VM_PROFILE_DESCRIPTORS[name]


def should_preauthorize_gnome_remote_desktop(
    profile: VmProfileDescriptor, desktop_env: str
) -> bool:
    return profile.preauthorize_gnome_remote_desktop and "gnome" in desktop_env.lower()


def should_preauthorize_kde_remote_desktop(profile: VmProfileDescriptor, desktop_env: str) -> bool:
    normalized_desktop = desktop_env.lower()
    return profile.preauthorize_kde_remote_desktop and (
        "kde" in normalized_desktop or "plasma" in normalized_desktop
    )


def preauthorize_gnome_remote_desktop(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_root: Path,
    *,
    wayland_display: str,
    desktop_env: str,
) -> None:
    desktop_exports = desktop_environment_exports(desktop_env)
    remote_script = (
        f"cd {shlex.quote(str(remote_root))} && "
        'runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" && '
        'if [ ! -d "$runtime_dir" ]; then runtime_dir=/tmp/sky-cua-runtime; mkdir -p "$runtime_dir"; chmod 700 "$runtime_dir"; fi && '
        'export XDG_RUNTIME_DIR="$runtime_dir" && '
        f'export WAYLAND_DISPLAY="${{WAYLAND_DISPLAY:-{shlex.quote(wayland_display)}}}" && '
        "export XDG_SESSION_TYPE=wayland && "
        f"{desktop_exports}"
        "python3 scripts/testing-vm/preauthorize_gnome_remote_desktop.py --print-json"
    )
    subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
        cwd=REPO_ROOT,
        check=True,
    )


def preauthorize_kde_remote_desktop(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_root: Path,
    *,
    wayland_display: str,
    desktop_env: str,
) -> None:
    desktop_exports = desktop_environment_exports(desktop_env)
    remote_script = (
        f"cd {shlex.quote(str(remote_root))} && "
        'runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" && '
        'if [ ! -d "$runtime_dir" ]; then runtime_dir=/tmp/sky-cua-runtime; mkdir -p "$runtime_dir"; chmod 700 "$runtime_dir"; fi && '
        'export XDG_RUNTIME_DIR="$runtime_dir" && '
        f'export WAYLAND_DISPLAY="${{WAYLAND_DISPLAY:-{shlex.quote(wayland_display)}}}" && '
        "export XDG_SESSION_TYPE=wayland && "
        f"{desktop_exports}"
        "python3 scripts/testing-vm/preauthorize_kde_remote_desktop.py "
        "--app-id '' --app-id desktop --print-json"
    )
    subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
        cwd=REPO_ROOT,
        check=True,
    )


def preauthorize_screenshot_portal(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_root: Path,
    *,
    wayland_display: str,
    desktop_env: str,
) -> None:
    desktop_exports = desktop_environment_exports(desktop_env)
    remote_script = (
        f"cd {shlex.quote(str(remote_root))} && "
        'runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" && '
        'if [ ! -d "$runtime_dir" ]; then runtime_dir=/tmp/sky-cua-runtime; mkdir -p "$runtime_dir"; chmod 700 "$runtime_dir"; fi && '
        'export XDG_RUNTIME_DIR="$runtime_dir" && '
        f'export WAYLAND_DISPLAY="${{WAYLAND_DISPLAY:-{shlex.quote(wayland_display)}}}" && '
        "export XDG_SESSION_TYPE=wayland && "
        f"{desktop_exports}"
        "python3 scripts/testing-vm/preauthorize_screenshot_portal.py"
    )
    subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
        cwd=REPO_ROOT,
        check=True,
    )


def desktop_environment_exports(desktop_env: str) -> str:
    if not desktop_env:
        return ""
    quoted_desktop = shlex.quote(desktop_env)
    return (
        f"export XDG_CURRENT_DESKTOP={quoted_desktop} && "
        f"export XDG_SESSION_DESKTOP={quoted_desktop} && "
        f"export DESKTOP_SESSION={quoted_desktop} && "
        'if [ "$XDG_CURRENT_DESKTOP" = "Hyprland" ] && [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] '
        '&& [ -d "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/hypr" ]; then '
        'export HYPRLAND_INSTANCE_SIGNATURE="$(ls "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/hypr" 2>/dev/null | head -n1)"; '
        "fi && "
        "systemctl --user import-environment "
        "XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP DESKTOP_SESSION XDG_SESSION_TYPE "
        "XDG_RUNTIME_DIR WAYLAND_DISPLAY DISPLAY DBUS_SESSION_BUS_ADDRESS "
        "HYPRLAND_INSTANCE_SIGNATURE >/dev/null 2>&1 || true && "
        "dbus-update-activation-environment --systemd "
        "XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP DESKTOP_SESSION XDG_SESSION_TYPE "
        "XDG_RUNTIME_DIR WAYLAND_DISPLAY DISPLAY DBUS_SESSION_BUS_ADDRESS "
        "HYPRLAND_INSTANCE_SIGNATURE >/dev/null 2>&1 || true && "
    )


def run_remote_profile(
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
    runner = RemoteRunner(
        ssh_target=ssh_target,
        port=port,
        ssh_options=ssh_options,
        remote_root=remote_root,
        wayland_display=wayland_display,
        desktop_env=desktop_env,
    )
    profile_command = [
        "env",
        "SKY_CUA_USE_PREBUILT_RUNTIMES=1",
        f"SKY_CUA_COPY_CODEX_SETTINGS={int(sync_codex_settings)}",
        "SKY_CUA_INPUT_HELPER_SOCKET=/run/sky-cua/input-helper.sock",
        f"SKY_CUA_OVERLAY_HOST_PATH={remote_root}/target/release/sky-cua-overlay-host",
        f"SKY_CUA_DEBUG_OVERLAY_HOST_PATH={remote_root}/target/debug/sky-cua-overlay-host",
        f"SKY_CUA_COSMIC_HELPER={remote_root}/target/release/sky-cua-cosmic-helper",
        f"PATH={remote_root}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    ]
    if profile in AGENT_AUTH_PROFILES:
        agent_env = dict(os.environ)
        # pi's `opencode` provider authenticates via $OPENCODE_API_KEY; source it
        # from OpenCode's own credential store so the pi/all smokes work without a
        # manual export.
        if not agent_env.get("OPENCODE_API_KEY"):
            opencode_key = resolve_opencode_api_key()
            if opencode_key:
                agent_env["OPENCODE_API_KEY"] = opencode_key
        for key in (*AGENT_AUTH_ENV_KEYS, *MCP_LAUNCH_POLICY_ENV_KEYS):
            value = agent_env.get(key)
            if value:
                profile_command.append(f"{key}={value}")
    profile_command.extend(
        [
            "bash",
            str(remote_root / "scripts" / "testing-vm" / "profiles" / "run-profile.sh"),
            profile,
        ]
    )
    if headed:
        profile_command.append("--headed")
    completed = runner.run(runner.runtime_script(profile_command), check=False)
    return completed.returncode


def timestamp_utc() -> str:
    return datetime.now(UTC).strftime("%Y%m%dT%H%M%S%fZ")


def cosmic_host_framebuffer_proof_paths(
    profile: CosmicHostFramebufferProofProfile,
    remote_root: Path,
    timestamp: str,
) -> CosmicHostFramebufferProofPaths:
    local_artifact_dir = REPO_ROOT / "artifacts" / profile.local_artifact_dir_name / timestamp
    return CosmicHostFramebufferProofPaths(
        remote_artifact_dir=remote_root
        / "artifacts"
        / "codex-e2e"
        / profile.remote_profile_dir
        / timestamp,
        local_artifact_dir=local_artifact_dir,
        before_path=local_artifact_dir / "before.png",
        visible_path=local_artifact_dir / "visible.png",
        hidden_path=local_artifact_dir / "hidden.png",
    )


def kwin_host_framebuffer_proof_paths(
    profile: KwinHostFramebufferProofProfile,
    remote_root: Path,
    timestamp: str,
) -> KwinHostFramebufferProofPaths:
    local_artifact_dir = REPO_ROOT / "artifacts"
    for part in profile.local_artifact_dir_parts:
        local_artifact_dir /= part
    local_artifact_dir /= timestamp
    return KwinHostFramebufferProofPaths(
        remote_artifact_dir=remote_root
        / "artifacts"
        / "codex-e2e"
        / profile.remote_profile_dir
        / f"{timestamp}-kwin-system-runner",
        local_artifact_dir=local_artifact_dir,
        before_path=local_artifact_dir / "before.png",
        after_path=local_artifact_dir / "after.png",
    )


def prepare_cosmic_host_framebuffer_proof(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    vm_name: str,
    libvirt_uri: str,
    paths: CosmicHostFramebufferProofPaths,
) -> None:
    paths.local_artifact_dir.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            *ssh_base_command(port, ssh_options),
            ssh_target,
            "ydotool mousemove --absolute -x 80 -y 80 >/dev/null 2>&1 || true",
        ],
        cwd=REPO_ROOT,
        check=False,
    )
    time.sleep(0.5)
    capture_vm_framebuffer(vm_name, libvirt_uri, paths.before_path)


def wait_for_cosmic_host_framebuffer_markers(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    vm_name: str,
    libvirt_uri: str,
    remote_process: subprocess.Popen[str],
    paths: CosmicHostFramebufferProofPaths,
) -> tuple[bool, bool]:
    visible_ready = wait_for_remote_path(
        ssh_target,
        port,
        ssh_options,
        paths.remote_artifact_dir / "visible-ready",
        remote_process,
        deadline=time.time() + 30,
    )
    if visible_ready:
        capture_vm_framebuffer(vm_name, libvirt_uri, paths.visible_path)
    hidden_ready = wait_for_remote_path(
        ssh_target,
        port,
        ssh_options,
        paths.remote_artifact_dir / "hidden-ready",
        remote_process,
        deadline=time.time() + 30,
    )
    if not hidden_ready:
        hidden_ready = remote_path_exists(
            ssh_target,
            port,
            ssh_options,
            paths.remote_artifact_dir / "hidden-ready",
        )
    if hidden_ready:
        capture_vm_framebuffer(vm_name, libvirt_uri, paths.hidden_path)
    return visible_ready, hidden_ready


def wait_for_remote_process(
    remote_process: subprocess.Popen[str], timeout: float
) -> tuple[int, str | None]:
    try:
        return remote_process.wait(timeout=timeout), None
    except subprocess.TimeoutExpired:
        remote_process.terminate()
        try:
            returncode = remote_process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            remote_process.kill()
            returncode = remote_process.wait(timeout=5)
        return returncode, f"remote process timed out after {timeout:g}s"


def write_host_summary(path: Path, summary: dict[str, object]) -> None:
    path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_remote_cosmic_patched_cursor_host_proof_profile(
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
    profile = COSMIC_PATCHED_CURSOR_HOST_PROOF
    paths = cosmic_host_framebuffer_proof_paths(profile, remote_root, timestamp_utc())
    prepare_cosmic_host_framebuffer_proof(
        ssh_target, port, ssh_options, vm_name, libvirt_uri, paths
    )

    desktop_exports = desktop_environment_exports(desktop_env)
    remote_script = f"""
set -euo pipefail
cd {shlex.quote(str(remote_root))}
mkdir -p {shlex.quote(str(paths.remote_artifact_dir))}
test -e /run/user/$(id -u)/sky-cua-cosmic-cursor-ready
export SKY_CUA_COSMIC_HOST_PROOF_ARTIFACT_DIR={shlex.quote(str(paths.remote_artifact_dir))}
export SKY_CUA_REMOTE_ROOT={shlex.quote(str(remote_root))}
export SKY_CUA_OVERLAY_BACKEND=layer-shell
export XDG_RUNTIME_DIR="${{XDG_RUNTIME_DIR:-/run/user/$(id -u)}}"
export DBUS_SESSION_BUS_ADDRESS="${{DBUS_SESSION_BUS_ADDRESS:-unix:path=${{XDG_RUNTIME_DIR}}/bus}}"
export WAYLAND_DISPLAY="${{WAYLAND_DISPLAY:-{shlex.quote(wayland_display)}}}"
export XDG_SESSION_TYPE=wayland
{desktop_exports}
python3 - <<'PY'
import json
import os
import subprocess
import time
from pathlib import Path

artifact = Path(os.environ["SKY_CUA_COSMIC_HOST_PROOF_ARTIFACT_DIR"])
remote_root = Path(os.environ["SKY_CUA_REMOTE_ROOT"])
proc = subprocess.Popen(
    [str(remote_root / "target/release/sky-cua-overlay-host"), "serve"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    env=os.environ.copy(),
    cwd=remote_root,
)
assert proc.stdin and proc.stdout
state = {{
    "version": 1,
    "kind": "set_cursor",
    "state": {{
        "visible": True,
        "sequence": 1,
        "native_point": {{"x": 360.0, "y": 260.0, "coordinate_space": "desktop_logical"}},
        "model_point": {{"x": 360.0, "y": 260.0, "coordinate_space": "stream_pixels"}},
        "source_action": "click",
        "updated_at_ms": int(time.time() * 1000),
    }},
}}
proc.stdin.write(json.dumps(state) + "\\n")
proc.stdin.flush()
set_reply = proc.stdout.readline().strip()
(artifact / "set-reply.json").write_text(set_reply + "\\n", encoding="utf-8")
(artifact / "visible-ready").write_text("ready\\n", encoding="utf-8")
time.sleep(8)
hide = {{"version": 1, "kind": "hide", "reason": {profile.hide_reason!r}}}
proc.stdin.write(json.dumps(hide) + "\\n")
proc.stdin.flush()
hide_reply = proc.stdout.readline().strip()
(artifact / "hide-reply.json").write_text(hide_reply + "\\n", encoding="utf-8")
time.sleep(0.5)
(artifact / "hidden-ready").write_text("ready\\n", encoding="utf-8")
proc.terminate()
try:
    proc.wait(timeout=2)
except subprocess.TimeoutExpired:
    proc.kill()
    proc.wait(timeout=2)
stderr = proc.stderr.read() if proc.stderr else ""
if stderr:
    (artifact / "overlay-stderr.log").write_text(stderr, encoding="utf-8")
PY
"""
    with (
        paths.stdout_path.open("w", encoding="utf-8") as stdout,
        paths.stderr_path.open("w", encoding="utf-8") as stderr,
    ):
        remote_process = subprocess.Popen(
            [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
            cwd=REPO_ROOT,
            stdout=stdout,
            stderr=stderr,
            text=True,
        )
        visible_ready, hidden_ready = wait_for_cosmic_host_framebuffer_markers(
            ssh_target, port, ssh_options, vm_name, libvirt_uri, remote_process, paths
        )
        returncode, remote_timeout_error = wait_for_remote_process(remote_process, timeout=30)

    set_reply, hide_reply = read_remote_jsons(
        ssh_target,
        port,
        ssh_options,
        [
            paths.remote_artifact_dir / "set-reply.json",
            paths.remote_artifact_dir / "hide-reply.json",
        ],
    )
    agent_probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    restored_cursor_probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    before_vs_hidden_probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    probe_error: str | None = None
    if visible_ready and hidden_ready:
        try:
            agent_probe = probe_marker(
                paths.hidden_path, paths.visible_path, COSMIC_HOST_OBSERVED_AGENT_MARKER_POINT
            )
            restored_cursor_probe = probe_marker(
                paths.hidden_path, paths.visible_path, COSMIC_HOST_RESTORED_CURSOR_POINT
            )
            before_vs_hidden_probe = probe_marker(
                paths.before_path, paths.hidden_path, COSMIC_HOST_RESTORED_CURSOR_POINT
            )
        except Exception as error:
            probe_error = f"{type(error).__name__}: {error}"
    else:
        probe_error = "remote proof did not reach visible-ready and hidden-ready markers"

    set_capabilities = json_object_field(set_reply, "capabilities")
    hide_capabilities = json_object_field(hide_reply, "capabilities")
    system_cursor_ok = (
        set_capabilities.get("system_cursor_backend") == "cosmic_comp_bridge"
        and set_capabilities.get("system_cursor_hide_supported") is True
        and set_capabilities.get("system_cursor_hidden") is True
        and hide_capabilities.get("system_cursor_hidden") is False
    )
    host_ok = (
        returncode == 0
        and remote_timeout_error is None
        and system_cursor_ok
        and agent_probe.found
        and restored_cursor_probe.found
        and not before_vs_hidden_probe.found
    )
    host_summary = {
        "ok": host_ok,
        "mode": profile.mode,
        "vm_name": vm_name,
        "libvirt_uri": libvirt_uri,
        "remote_returncode": returncode,
        "remote_artifact_dir": str(paths.remote_artifact_dir),
        "local_artifact_dir": str(paths.local_artifact_dir),
        "before_screenshot": str(paths.before_path),
        "visible_screenshot": str(paths.visible_path) if paths.visible_path.exists() else None,
        "hidden_screenshot": str(paths.hidden_path) if paths.hidden_path.exists() else None,
        "visible_ready": visible_ready,
        "hidden_ready": hidden_ready,
        "remote_timeout_error": remote_timeout_error,
        "requested_agent_point": {"x": COSMIC_HOST_AGENT_POINT[0], "y": COSMIC_HOST_AGENT_POINT[1]},
        "observed_agent_marker_point": {
            "x": COSMIC_HOST_OBSERVED_AGENT_MARKER_POINT[0],
            "y": COSMIC_HOST_OBSERVED_AGENT_MARKER_POINT[1],
        },
        "observed_restored_cursor_point": {
            "x": COSMIC_HOST_RESTORED_CURSOR_POINT[0],
            "y": COSMIC_HOST_RESTORED_CURSOR_POINT[1],
        },
        "agent_visible_vs_hidden_marker_probe": marker_probe_to_json(agent_probe),
        "real_cursor_restore_delta_probe": marker_probe_to_json(restored_cursor_probe),
        "before_vs_hidden_real_cursor_probe": marker_probe_to_json(before_vs_hidden_probe),
        "host_probe_error": probe_error,
        "system_cursor_ok": system_cursor_ok,
        "set_reply": set_reply,
        "hide_reply": hide_reply,
    }
    write_host_summary(paths.host_summary_path, host_summary)
    print(json.dumps(host_summary, indent=2, sort_keys=True))
    return 0 if host_ok else 1


def run_remote_cosmic_transparent_xcursor_host_proof_profile(
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
    profile = COSMIC_TRANSPARENT_XCURSOR_HOST_PROOF
    paths = cosmic_host_framebuffer_proof_paths(profile, remote_root, timestamp_utc())
    prepare_cosmic_host_framebuffer_proof(
        ssh_target, port, ssh_options, vm_name, libvirt_uri, paths
    )

    desktop_exports = desktop_environment_exports(desktop_env)
    remote_script = f"""
set -euo pipefail
cd {shlex.quote(str(remote_root))}
mkdir -p {shlex.quote(str(paths.remote_artifact_dir))}
! test -e /run/user/$(id -u)/sky-cua-cosmic-cursor-ready
pid="$(pgrep -n -x cosmic-comp)"
tr '\\0' '\\n' <"/proc/${{pid}}/environ" >{shlex.quote(str(paths.remote_artifact_dir))}/cosmic-comp-environ.txt
grep -qx 'XCURSOR_THEME=sky-cua-blank' {shlex.quote(str(paths.remote_artifact_dir))}/cosmic-comp-environ.txt
test -f "$HOME/.local/share/icons/sky-cua-blank/cursors/left_ptr"
export SKY_CUA_COSMIC_HOST_PROOF_ARTIFACT_DIR={shlex.quote(str(paths.remote_artifact_dir))}
export SKY_CUA_REMOTE_ROOT={shlex.quote(str(remote_root))}
export SKY_CUA_OVERLAY_BACKEND=layer-shell
export XDG_RUNTIME_DIR="${{XDG_RUNTIME_DIR:-/run/user/$(id -u)}}"
export DBUS_SESSION_BUS_ADDRESS="${{DBUS_SESSION_BUS_ADDRESS:-unix:path=${{XDG_RUNTIME_DIR}}/bus}}"
export WAYLAND_DISPLAY="${{WAYLAND_DISPLAY:-{shlex.quote(wayland_display)}}}"
export XDG_SESSION_TYPE=wayland
export XCURSOR_THEME=sky-cua-blank
export XCURSOR_SIZE=24
{desktop_exports}
python3 - <<'PY'
import json
import os
import subprocess
import time
from pathlib import Path

artifact = Path(os.environ["SKY_CUA_COSMIC_HOST_PROOF_ARTIFACT_DIR"])
remote_root = Path(os.environ["SKY_CUA_REMOTE_ROOT"])
proc = subprocess.Popen(
    [str(remote_root / "target/release/sky-cua-overlay-host"), "serve"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    env=os.environ.copy(),
    cwd=remote_root,
)
assert proc.stdin and proc.stdout
state = {{
    "version": 1,
    "kind": "set_cursor",
    "state": {{
        "visible": True,
        "sequence": 1,
        "native_point": {{"x": 360.0, "y": 260.0, "coordinate_space": "desktop_logical"}},
        "model_point": {{"x": 360.0, "y": 260.0, "coordinate_space": "stream_pixels"}},
        "source_action": "click",
        "updated_at_ms": int(time.time() * 1000),
    }},
}}
proc.stdin.write(json.dumps(state) + "\\n")
proc.stdin.flush()
set_reply = proc.stdout.readline().strip()
(artifact / "set-reply.json").write_text(set_reply + "\\n", encoding="utf-8")
(artifact / "visible-ready").write_text("ready\\n", encoding="utf-8")
time.sleep(8)
hide = {{"version": 1, "kind": "hide", "reason": {profile.hide_reason!r}}}
proc.stdin.write(json.dumps(hide) + "\\n")
proc.stdin.flush()
hide_reply = proc.stdout.readline().strip()
(artifact / "hide-reply.json").write_text(hide_reply + "\\n", encoding="utf-8")
time.sleep(0.5)
(artifact / "hidden-ready").write_text("ready\\n", encoding="utf-8")
proc.terminate()
try:
    proc.wait(timeout=2)
except subprocess.TimeoutExpired:
    proc.kill()
    proc.wait(timeout=2)
stderr = proc.stderr.read() if proc.stderr else ""
if stderr:
    (artifact / "overlay-stderr.log").write_text(stderr, encoding="utf-8")
PY
"""
    with (
        paths.stdout_path.open("w", encoding="utf-8") as stdout,
        paths.stderr_path.open("w", encoding="utf-8") as stderr,
    ):
        remote_process = subprocess.Popen(
            [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
            cwd=REPO_ROOT,
            stdout=stdout,
            stderr=stderr,
            text=True,
        )
        visible_ready, hidden_ready = wait_for_cosmic_host_framebuffer_markers(
            ssh_target, port, ssh_options, vm_name, libvirt_uri, remote_process, paths
        )
        returncode, remote_timeout_error = wait_for_remote_process(remote_process, timeout=30)

    set_reply, hide_reply = read_remote_jsons(
        ssh_target,
        port,
        ssh_options,
        [
            paths.remote_artifact_dir / "set-reply.json",
            paths.remote_artifact_dir / "hide-reply.json",
        ],
    )
    set_capabilities = json_object_field(set_reply, "capabilities")
    hide_capabilities = json_object_field(hide_reply, "capabilities")
    agent_probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    hidden_agent_probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    native_cursor_probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    probe_error: str | None = None
    if visible_ready and hidden_ready:
        try:
            agent_probe = probe_marker(
                paths.hidden_path, paths.visible_path, COSMIC_HOST_OBSERVED_AGENT_MARKER_POINT
            )
            hidden_agent_probe = probe_marker(
                paths.before_path, paths.hidden_path, COSMIC_HOST_OBSERVED_AGENT_MARKER_POINT
            )
            native_cursor_probe = probe_marker(
                paths.before_path, paths.hidden_path, COSMIC_HOST_RESTORED_CURSOR_POINT
            )
        except Exception as error:
            probe_error = f"{type(error).__name__}: {error}"
    else:
        probe_error = "remote proof did not reach visible-ready and hidden-ready markers"

    system_cursor_ok = (
        set_capabilities.get("system_cursor_backend") == "cosmic_transparent_xcursor"
        and set_capabilities.get("system_cursor_hide_supported") is True
        and set_capabilities.get("system_cursor_hidden") is True
        and hide_capabilities.get("system_cursor_hidden") is False
    )
    host_ok = (
        returncode == 0
        and remote_timeout_error is None
        and system_cursor_ok
        and agent_probe.found
        and not hidden_agent_probe.found
        and not native_cursor_probe.found
    )
    host_summary = {
        "ok": host_ok,
        "mode": profile.mode,
        "vm_name": vm_name,
        "libvirt_uri": libvirt_uri,
        "remote_returncode": returncode,
        "remote_artifact_dir": str(paths.remote_artifact_dir),
        "local_artifact_dir": str(paths.local_artifact_dir),
        "before_screenshot": str(paths.before_path),
        "visible_screenshot": str(paths.visible_path) if paths.visible_path.exists() else None,
        "hidden_screenshot": str(paths.hidden_path) if paths.hidden_path.exists() else None,
        "visible_ready": visible_ready,
        "hidden_ready": hidden_ready,
        "remote_timeout_error": remote_timeout_error,
        "requested_agent_point": {"x": COSMIC_HOST_AGENT_POINT[0], "y": COSMIC_HOST_AGENT_POINT[1]},
        "observed_agent_marker_point": {
            "x": COSMIC_HOST_OBSERVED_AGENT_MARKER_POINT[0],
            "y": COSMIC_HOST_OBSERVED_AGENT_MARKER_POINT[1],
        },
        "agent_visible_vs_hidden_marker_probe": marker_probe_to_json(agent_probe),
        "hidden_frame_agent_marker_probe": marker_probe_to_json(hidden_agent_probe),
        "native_cursor_probe": marker_probe_to_json(native_cursor_probe),
        "host_probe_error": probe_error,
        "system_cursor_ok": system_cursor_ok,
        "set_reply": set_reply,
        "hide_reply": hide_reply,
    }
    write_host_summary(paths.host_summary_path, host_summary)
    print(json.dumps(host_summary, indent=2, sort_keys=True))
    return 0 if host_ok else 1


def run_remote_kwin_effect_system_install_profile(
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
    profile = KWIN_EFFECT_SYSTEM_INSTALL_HOST_PROOF
    paths = kwin_host_framebuffer_proof_paths(profile, remote_root, timestamp_utc())
    paths.local_artifact_dir.mkdir(parents=True, exist_ok=True)
    capture_vm_framebuffer(vm_name, libvirt_uri, paths.before_path)

    profile_command = [
        "env",
        "SKY_CUA_USE_PREBUILT_RUNTIMES=1",
        f"SKY_CUA_COPY_CODEX_SETTINGS={int(sync_codex_settings)}",
        f"SKY_CUA_OVERLAY_HOST_PATH={remote_root}/target/release/sky-cua-overlay-host",
        f"SKY_CUA_DEBUG_OVERLAY_HOST_PATH={remote_root}/target/debug/sky-cua-overlay-host",
        f"SKY_CUA_KWIN_SYSTEM_INSTALL_ARTIFACT_DIR={paths.remote_artifact_dir}",
        "SKY_CUA_KWIN_SYSTEM_INSTALL_HOST_FRAMEBUFFER_PROOF=1",
        "SKY_CUA_KWIN_SYSTEM_INSTALL_HOLD_SECONDS=8",
        f"PATH={remote_root}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "bash",
        str(remote_root / "scripts" / "testing-vm" / "profiles" / "run-profile.sh"),
        "kde-kwin-effect-system-install",
    ]
    desktop_exports = desktop_environment_exports(desktop_env)
    remote_script = (
        f"cd {shlex.quote(str(remote_root))} && "
        'runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}" && '
        'if [ ! -d "$runtime_dir" ]; then runtime_dir=/tmp/sky-cua-runtime; mkdir -p "$runtime_dir"; chmod 700 "$runtime_dir"; fi && '
        'export XDG_RUNTIME_DIR="$runtime_dir" && '
        f'export WAYLAND_DISPLAY="${{WAYLAND_DISPLAY:-{shlex.quote(wayland_display)}}}" && '
        "export XDG_SESSION_TYPE=wayland && "
        f"{desktop_exports}"
        f"{shlex.join(profile_command)}"
    )
    with (
        paths.stdout_path.open("w", encoding="utf-8") as stdout,
        paths.stderr_path.open("w", encoding="utf-8") as stderr,
    ):
        remote_process = subprocess.Popen(
            [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
            cwd=REPO_ROOT,
            stdout=stdout,
            stderr=stderr,
            text=True,
        )
        ready = wait_for_remote_path(
            ssh_target,
            port,
            ssh_options,
            paths.remote_artifact_dir / "host-framebuffer-ready.json",
            remote_process,
            deadline=time.time() + 90,
        )
        if ready:
            capture_vm_framebuffer(vm_name, libvirt_uri, paths.after_path)
        returncode, remote_timeout_error = wait_for_remote_process(remote_process, timeout=180)

    remote_summary = read_remote_json(
        ssh_target,
        port,
        ssh_options,
        paths.remote_artifact_dir / "summary.json",
    )
    remote_summary_error: str | None = None
    if remote_summary is None:
        remote_summary_error = "remote smoke did not write summary.json"
        remote_summary = {}
    probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    probe_error: str | None = None
    host_probe_point = requested_point_from_summary(remote_summary)
    if ready:
        try:
            probe = probe_marker(paths.before_path, paths.after_path, host_probe_point)
        except Exception as error:
            probe_error = f"{type(error).__name__}: {error}"
    else:
        probe_error = "remote smoke did not reach host-framebuffer-ready.json before exiting"

    remote_ok = remote_summary.get("ok") is True
    host_ok = (
        ready and probe.found and returncode == 0 and remote_ok and remote_timeout_error is None
    )
    host_summary = {
        "ok": host_ok,
        "mode": profile.mode,
        "vm_name": vm_name,
        "libvirt_uri": libvirt_uri,
        "remote_returncode": returncode,
        "remote_artifact_dir": str(paths.remote_artifact_dir),
        "remote_summary_error": remote_summary_error,
        "remote_timeout_error": remote_timeout_error,
        "local_artifact_dir": str(paths.local_artifact_dir),
        "before_screenshot": str(paths.before_path),
        "after_screenshot": str(paths.after_path) if paths.after_path.exists() else None,
        "host_framebuffer_ready": ready,
        "requested_point": {"x": host_probe_point[0], "y": host_probe_point[1]},
        "host_marker_probe": marker_probe_to_json(probe),
        "host_probe_error": probe_error,
        "remote_summary": remote_summary,
    }
    write_host_summary(paths.host_summary_path, host_summary)
    print(json.dumps(host_summary, indent=2, sort_keys=True))
    return 0 if host_ok else 1


def capture_vm_framebuffer(vm_name: str, libvirt_uri: str, output_path: Path) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["virsh", "--connect", libvirt_uri, "screenshot", vm_name, str(output_path)],
        cwd=REPO_ROOT,
        check=True,
        timeout=15,
    )


def marker_probe_to_json(probe: MarkerProbe) -> dict[str, object]:
    return {
        "found": probe.found,
        "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
        "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
        "checked_box": list(probe.checked_box),
    }


def remote_path_exists(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_path: Path,
) -> bool:
    return (
        subprocess.run(
            [
                *ssh_base_command(port, ssh_options),
                ssh_target,
                f"test -f {shlex.quote(str(remote_path))}",
            ],
            cwd=REPO_ROOT,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        ).returncode
        == 0
    )


def wait_for_remote_path(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_path: Path,
    remote_process: subprocess.Popen[str],
    *,
    deadline: float,
) -> bool:
    test_command = f"test -f {shlex.quote(str(remote_path))}"
    while time.time() < deadline:
        if remote_process.poll() is not None:
            return False
        exists = subprocess.run(
            [*ssh_base_command(port, ssh_options), ssh_target, test_command],
            cwd=REPO_ROOT,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if exists.returncode == 0:
            return True
        time.sleep(0.5)
    return False


def read_remote_json(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_path: Path,
) -> dict[str, object] | None:
    completed = subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, "cat", str(remote_path)],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        return None
    # A truncated or half-flushed remote marker must not crash the dispatch: treat
    # unparseable or non-object payloads as "no usable marker" so callers fall into
    # their graceful no-artifacts branch and still write a host summary.
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    return payload


def read_remote_jsons(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    remote_paths: list[Path],
) -> list[dict[str, object] | None]:
    """Read multiple remote JSON files in a single SSH call."""
    if not remote_paths:
        return []
    # Build a shell command that cats each file and emits a NUL separator.
    cat_commands = "; ".join(
        "(cat {path} 2>/dev/null || printf %s {missing}); printf '\\0'".format(
            path=shlex.quote(str(p)),
            missing=shlex.quote("---FILE_NOT_FOUND---"),
        )
        for p in remote_paths
    )
    completed = subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, cat_commands],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        return [None] * len(remote_paths)
    raw_parts = completed.stdout.split("\x00")
    results: list[dict[str, object] | None] = []
    for raw in raw_parts[: len(remote_paths)]:
        stripped = raw.strip()
        if stripped == "---FILE_NOT_FOUND---":
            results.append(None)
            continue
        try:
            payload = json.loads(stripped)
        except json.JSONDecodeError:
            results.append(None)
            continue
        if not isinstance(payload, dict):
            raise RuntimeError(f"remote JSON was not an object: {payload!r}")
        results.append(payload)
    while len(results) < len(remote_paths):
        results.append(None)
    return results


def json_object_field(parent: dict[str, object] | None, key: str) -> dict[str, object]:
    if parent is None:
        return {}
    value = parent.get(key)
    if not isinstance(value, dict):
        return {}
    return value


def requested_point_from_summary(summary: dict[str, object]) -> tuple[float, float]:
    native_point = summary.get("requested_native_point")
    if isinstance(native_point, dict):
        x = native_point.get("x")
        y = native_point.get("y")
        if isinstance(x, int | float) and isinstance(y, int | float):
            return (float(x), float(y))
    point = summary.get("requested_point")
    if isinstance(point, dict):
        x = point.get("x")
        y = point.get("y")
        if isinstance(x, int | float) and isinstance(y, int | float):
            return (float(x), float(y))
    return KWIN_EFFECT_SYSTEM_POINT


if __name__ == "__main__":
    raise SystemExit(main())
