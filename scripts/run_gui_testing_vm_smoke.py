#!/usr/bin/env python3
"""Build, sync, and run GUI desktop smoke profiles on the Arch testing VM."""

from __future__ import annotations

import argparse
import json
import shlex
import subprocess
import time
from datetime import UTC, datetime
from pathlib import Path

from live_agent_cursor_kde_smoke import MarkerProbe, probe_marker  # type: ignore[import-not-found]

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REMOTE_ROOT = Path("/workspace")
DEFAULT_CODEX_HOME = Path.home() / ".codex"
KWIN_EFFECT_SYSTEM_POINT = (420.0, 260.0)
PROFILES = (
    "kde-kwin-effect",
    "kde-kwin-effect-system-install",
    "kde-plasma",
    "i3",
    "computer-use",
    "codex-desktop",
    "cosmic-helper",
    "wayland-layer-shell-overlay",
    "wayland-pointer",
    "wayland-pointer-scaled",
    "gnome",
    "cosmic",
    "hyprland",
    "all",
)
RUNTIME_PACKAGES = (
    "sky-cua-client",
    "sky-cua-service",
    "sky-cua-cosmic-helper",
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
    "auth.json",
    "cap_sid",
    "config.json",
    "config.toml",
    ".codex-global-state.json",
    "installation_id",
    "internal_storage.json",
    "models_cache.json",
    "state_5.sqlite",
    "state_5.sqlite-shm",
    "state_5.sqlite-wal",
    "version.json",
    "keybindings.json",
    "browser/config.toml",
)
CODEX_SETTING_DIRS = ("plugins", "skills")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run sky-cua GUI desktop smoke profiles on the Arch testing VM."
    )
    parser.add_argument("--host", required=True, help="SSH host name or address for the VM.")
    parser.add_argument("--user", default="skycua", help="SSH user for the VM.")
    parser.add_argument("--port", type=int, default=22, help="SSH port for the VM.")
    parser.add_argument(
        "--profile",
        choices=PROFILES,
        default="computer-use",
        help="Desktop profile to execute inside the VM.",
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
        "--skip-codex-settings",
        action="store_true",
        help="Deprecated compatibility flag; Codex settings are copied only when --sync-codex-settings is set and this flag is absent.",
    )
    parser.add_argument(
        "--sync-codex-settings",
        action="store_true",
        help="Copy selected host ~/.codex settings into the VM for authenticated Codex smokes.",
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=DEFAULT_CODEX_HOME,
        help="Host Codex settings directory to sync into the VM.",
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

    ssh_target = f"{args.user}@{args.host}"
    remote_root = args.remote_root
    if not args.skip_host_build:
        build_host_runtime_artifacts()
    if not args.skip_sync:
        sync_checkout(ssh_target, args.port, args.ssh_option, remote_root)
    if args.sync_codex_settings and not args.skip_codex_settings:
        sync_codex_settings(ssh_target, args.port, args.ssh_option, args.codex_home)
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
    preauthorized_gnome_remote_desktop = (
        not args.skip_gnome_remote_desktop_preauth
        and should_preauthorize_gnome_remote_desktop(args.profile, args.desktop_env)
    )
    preauthorized_kde_remote_desktop = (
        not args.skip_kde_remote_desktop_preauth
        and should_preauthorize_kde_remote_desktop(args.profile, args.desktop_env)
    )
    if preauthorized_gnome_remote_desktop:
        preauthorize_gnome_remote_desktop(
            ssh_target,
            args.port,
            args.ssh_option,
            remote_root,
            wayland_display=args.wayland_display,
            desktop_env=args.desktop_env,
        )
    if preauthorized_kde_remote_desktop:
        preauthorize_kde_remote_desktop(
            ssh_target,
            args.port,
            args.ssh_option,
            remote_root,
            wayland_display=args.wayland_display,
            desktop_env=args.desktop_env,
        )
    if args.profile == "kde-kwin-effect-system-install":
        return run_remote_kwin_effect_system_install_profile(
            ssh_target,
            args.port,
            args.ssh_option,
            remote_root,
            wayland_display=args.wayland_display,
            desktop_env=args.desktop_env,
            sync_codex_settings=args.sync_codex_settings and not args.skip_codex_settings,
            vm_name=args.vm_name,
            libvirt_uri=args.libvirt_uri,
        )
    return run_remote_profile(
        ssh_target,
        args.port,
        args.ssh_option,
        remote_root,
        args.profile,
        headed=args.headed,
        wayland_display=args.wayland_display,
        desktop_env=args.desktop_env,
        sync_codex_settings=args.sync_codex_settings and not args.skip_codex_settings,
    )


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
    subprocess.run(
        ["cargo", "build", "-p", "sky-cua-overlay-host"],
        cwd=REPO_ROOT,
        check=True,
    )


def ssh_base_command(port: int, ssh_options: list[str]) -> list[str]:
    command = ["ssh", "-p", str(port)]
    for option in ssh_options:
        command.extend(["-o", option])
    return command


def rsync_ssh_command(port: int, ssh_options: list[str]) -> str:
    parts = ["ssh", "-p", str(port)]
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


def sync_codex_settings(
    ssh_target: str,
    port: int,
    ssh_options: list[str],
    codex_home: Path,
) -> None:
    codex_home = codex_home.expanduser().resolve()
    if not codex_home.exists():
        return
    subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, "mkdir", "-p", ".codex"],
        cwd=REPO_ROOT,
        check=True,
    )
    for relative_path in CODEX_SETTING_PATHS:
        source_path = codex_home / relative_path
        if source_path.exists():
            remote_path = f"{ssh_target}:.codex/{relative_path}"
            remote_parent = str(Path(".codex") / relative_path).rsplit("/", maxsplit=1)[0]
            subprocess.run(
                [*ssh_base_command(port, ssh_options), ssh_target, "mkdir", "-p", remote_parent],
                cwd=REPO_ROOT,
                check=True,
            )
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
        "pkill -x sky-cua-service 2>/dev/null || true; "
        "pkill -f '(^|/)sky-cua-overlay-host( |$)' 2>/dev/null || true; "
        "pkill -x sky-cua-overlay 2>/dev/null || true; "
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
        "systemctl --user stop "
        "xdg-desktop-portal.service "
        "xdg-desktop-portal-gtk.service "
        "xdg-desktop-portal-gnome.service "
        "xdg-desktop-portal-cosmic.service "
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


def should_preauthorize_gnome_remote_desktop(profile: str, desktop_env: str) -> bool:
    return profile in {"all", "computer-use", "wayland-pointer"} and "gnome" in desktop_env.lower()


def should_preauthorize_kde_remote_desktop(profile: str, desktop_env: str) -> bool:
    normalized_desktop = desktop_env.lower()
    return profile in {"all", "computer-use", "wayland-pointer"} and (
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


def desktop_environment_exports(desktop_env: str) -> str:
    if not desktop_env:
        return ""
    quoted_desktop = shlex.quote(desktop_env)
    return (
        f"export XDG_CURRENT_DESKTOP={quoted_desktop} && "
        f"export XDG_SESSION_DESKTOP={quoted_desktop} && "
        f"export DESKTOP_SESSION={quoted_desktop} && "
        "systemctl --user import-environment "
        "XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP DESKTOP_SESSION XDG_SESSION_TYPE "
        "XDG_RUNTIME_DIR WAYLAND_DISPLAY DISPLAY DBUS_SESSION_BUS_ADDRESS >/dev/null 2>&1 || true && "
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
    profile_command = [
        "env",
        "SKY_CUA_USE_PREBUILT_RUNTIMES=1",
        f"SKY_CUA_COPY_CODEX_SETTINGS={int(sync_codex_settings)}",
        f"SKY_CUA_OVERLAY_HOST_PATH={remote_root}/target/release/sky-cua-overlay-host",
        f"SKY_CUA_DEBUG_OVERLAY_HOST_PATH={remote_root}/target/debug/sky-cua-overlay-host",
        f"SKY_CUA_COSMIC_HELPER={remote_root}/target/release/sky-cua-cosmic-helper",
        f"PATH={remote_root}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "bash",
        str(remote_root / "scripts" / "testing-vm" / "profiles" / "run-profile.sh"),
        profile,
    ]
    if headed:
        profile_command.append("--headed")
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
    completed = subprocess.run(
        [*ssh_base_command(port, ssh_options), ssh_target, remote_script],
        cwd=REPO_ROOT,
        check=False,
    )
    return completed.returncode


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
    timestamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%S%fZ")
    remote_artifact_dir = (
        remote_root
        / "artifacts"
        / "codex-e2e"
        / "agent-cursor-kde"
        / f"{timestamp}-kwin-system-runner"
    )
    local_artifact_dir = (
        REPO_ROOT / "artifacts" / "kde-framebuffer-cursor-proof" / "kwin-system-install" / timestamp
    )
    local_artifact_dir.mkdir(parents=True, exist_ok=True)
    before_path = local_artifact_dir / "before.png"
    after_path = local_artifact_dir / "after.png"
    capture_vm_framebuffer(vm_name, libvirt_uri, before_path)

    profile_command = [
        "env",
        "SKY_CUA_USE_PREBUILT_RUNTIMES=1",
        f"SKY_CUA_COPY_CODEX_SETTINGS={int(sync_codex_settings)}",
        f"SKY_CUA_OVERLAY_HOST_PATH={remote_root}/target/release/sky-cua-overlay-host",
        f"SKY_CUA_DEBUG_OVERLAY_HOST_PATH={remote_root}/target/debug/sky-cua-overlay-host",
        f"SKY_CUA_KWIN_SYSTEM_INSTALL_ARTIFACT_DIR={remote_artifact_dir}",
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
    stdout_path = local_artifact_dir / "remote.stdout.log"
    stderr_path = local_artifact_dir / "remote.stderr.log"
    with (
        stdout_path.open("w", encoding="utf-8") as stdout,
        stderr_path.open("w", encoding="utf-8") as stderr,
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
            remote_artifact_dir / "host-framebuffer-ready.json",
            remote_process,
            deadline=time.time() + 90,
        )
        if ready:
            capture_vm_framebuffer(vm_name, libvirt_uri, after_path)
        returncode = remote_process.wait(timeout=180)

    remote_summary = read_remote_json(
        ssh_target,
        port,
        ssh_options,
        remote_artifact_dir / "summary.json",
    )
    remote_summary_error: str | None = None
    if remote_summary is None:
        remote_summary_error = "remote smoke did not write summary.json"
        remote_summary = {}
    probe = MarkerProbe(False, 0, 0, (0, 0, 0, 0))
    probe_error: str | None = None
    if ready:
        try:
            point = requested_point_from_summary(remote_summary)
            probe = probe_marker(before_path, after_path, point)
        except Exception as error:
            probe_error = f"{type(error).__name__}: {error}"
    else:
        probe_error = "remote smoke did not reach host-framebuffer-ready.json before exiting"

    remote_ok = remote_summary.get("ok") is True
    host_ok = ready and probe.found and returncode == 0 and remote_ok
    host_summary = {
        "ok": host_ok,
        "mode": "kde-kwin-effect-system-install-host-framebuffer",
        "vm_name": vm_name,
        "libvirt_uri": libvirt_uri,
        "remote_returncode": returncode,
        "remote_artifact_dir": str(remote_artifact_dir),
        "remote_summary_error": remote_summary_error,
        "local_artifact_dir": str(local_artifact_dir),
        "before_screenshot": str(before_path),
        "after_screenshot": str(after_path) if after_path.exists() else None,
        "host_framebuffer_ready": ready,
        "requested_point": {"x": KWIN_EFFECT_SYSTEM_POINT[0], "y": KWIN_EFFECT_SYSTEM_POINT[1]},
        "host_marker_probe": {
            "found": probe.found,
            "changed_pixels_near_hotspot": probe.changed_pixels_near_hotspot,
            "max_channel_delta_near_hotspot": probe.max_channel_delta_near_hotspot,
            "checked_box": list(probe.checked_box),
        },
        "host_probe_error": probe_error,
        "remote_summary": remote_summary,
    }
    (local_artifact_dir / "host-summary.json").write_text(
        json.dumps(host_summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(host_summary, indent=2, sort_keys=True))
    return 0 if host_ok else 1


def capture_vm_framebuffer(vm_name: str, libvirt_uri: str, output_path: Path) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["virsh", "--connect", libvirt_uri, "screenshot", vm_name, str(output_path)],
        cwd=REPO_ROOT,
        check=True,
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
    payload = json.loads(completed.stdout)
    if not isinstance(payload, dict):
        raise RuntimeError(f"remote JSON was not an object: {payload!r}")
    return payload


def requested_point_from_summary(summary: dict[str, object]) -> tuple[float, float]:
    point = summary.get("requested_point")
    if isinstance(point, dict):
        x = point.get("x")
        y = point.get("y")
        if isinstance(x, int | float) and isinstance(y, int | float):
            return (float(x), float(y))
    return KWIN_EFFECT_SYSTEM_POINT


if __name__ == "__main__":
    raise SystemExit(main())
