#!/usr/bin/env python3
"""Shared primitives for building, installing, and reloading the sky-cua KWin effect.

Used by the local deploy lanes (``install_kwin_effect.py``,
``install_mcp_server.py --kwin-effect``, ``deploy_plugin.py --kwin-effect``)
and by the KDE VM smoke. All subprocess calls accept an injectable runner so the
decision logic stays unit-testable without a running KWin.

KWin only discovers effect plugins from system paths (see
``docs/research/2026-05-kwin-effect-discovery.md``), so production installs go
under ``/usr`` via ``sudo cmake --install``. A replaced ``.so`` does not
hot-reload into a running KWin (no dlclose); convergence is verified at
runtime through the effect's ``BuildId()`` DBus slot. When the running build
stays stale, the deploy notifies the user to restart the Plasma session when
convenient — it never restarts KWin itself, because a compositor restart can
take the whole session down (verified the hard way on 2026-06-10).
"""

from __future__ import annotations

import hashlib
import os
import shlex
import subprocess
import sys
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
KWIN_EFFECT_ID = "sky-cua-agent-cursor"
KWIN_EFFECT_SOURCE_DIR = REPO_ROOT / "resources" / "kwin" / "effects" / KWIN_EFFECT_ID
KWIN_EFFECT_CURSOR_ASSET = (
    REPO_ROOT / "crates" / "sky-cua-overlay-host" / "assets" / "cursor-chat.png"
)
KWIN_USER_SERVICE = "plasma-kwin_wayland.service"
KWIN_AGENT_CURSOR_PATH = "/com/skycua/AgentCursor"
KWIN_AGENT_CURSOR_INTERFACE = "com.skycua.AgentCursor"
UNKNOWN_BUILD_ID = "unknown"

# Source globs that feed the build-id content hash. Keep in sync with what
# actually changes the compiled plugin or its runtime assets.
BUILD_ID_SOURCE_PATTERNS = (
    "*.cpp",
    "*.h",
    "CMakeLists.txt",
    "metadata.json",
    "qml/*.qml",
)

Runner = Callable[[list[str]], "subprocess.CompletedProcess[str]"]


def default_runner(command: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
    )


@dataclass(frozen=True)
class KwinSession:
    session_type: str | None
    kwin_service_active: bool
    kwin_dbus_reachable: bool


@dataclass
class ReloadOutcome:
    converged: bool
    loaded: bool
    expected_build_id: str
    running_build_id: str
    session_restart_required: bool = False
    notification_delivered: bool = False
    notes: list[str] = field(default_factory=list)
    steps: list[dict[str, Any]] = field(default_factory=list)


def compute_effect_build_id(
    source_dir: Path = KWIN_EFFECT_SOURCE_DIR,
    cursor_asset: Path = KWIN_EFFECT_CURSOR_ASSET,
) -> str:
    """Content hash over the effect sources and cursor asset (16 hex chars)."""
    digest = hashlib.sha256()
    paths: list[Path] = []
    for pattern in BUILD_ID_SOURCE_PATTERNS:
        paths.extend(source_dir.glob(pattern))
    paths.append(cursor_asset)
    for path in sorted(p for p in paths if p.is_file()):
        relative = (
            path.relative_to(source_dir) if path.is_relative_to(source_dir) else Path(path.name)
        )
        digest.update(relative.as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()[:16]


def cmake_configure_command(
    build_dir: Path,
    *,
    install_prefix: Path,
    build_id: str,
    source_dir: Path = KWIN_EFFECT_SOURCE_DIR,
    cursor_asset: Path = KWIN_EFFECT_CURSOR_ASSET,
) -> list[str]:
    return [
        "cmake",
        "-S",
        str(source_dir),
        "-B",
        str(build_dir),
        "-G",
        "Ninja",
        f"-DCMAKE_INSTALL_PREFIX={install_prefix}",
        f"-DSKY_CUA_CURSOR_ASSET={cursor_asset}",
        f"-DSKY_CUA_EFFECT_BUILD_ID={build_id}",
    ]


def cmake_build_command(build_dir: Path) -> list[str]:
    return ["cmake", "--build", str(build_dir)]


def cmake_install_command(build_dir: Path, *, sudo_cmd: list[str] | None = None) -> list[str]:
    return [*(sudo_cmd if sudo_cmd is not None else ["sudo"]), "cmake", "--install", str(build_dir)]


def effect_enabled_config_command(enabled: bool) -> list[str]:
    return [
        "kwriteconfig6",
        "--file",
        "kwinrc",
        "--group",
        "Plugins",
        "--key",
        f"{KWIN_EFFECT_ID}Enabled",
        "true" if enabled else "false",
    ]


UPDATE_PENDING_MESSAGE = (
    "The sky-cua agent-cursor KWin effect was updated. The new build loads "
    "when you restart your Plasma session (log out and back in) at your "
    "convenience; the current session keeps the previous build."
)


def update_notification_command() -> list[str]:
    return [
        "notify-send",
        "--app-name=sky-cua",
        "--icon=preferences-desktop-effects",
        "sky-cua KWin effect updated",
        UPDATE_PENDING_MESSAGE,
    ]


def update_notification_fallback_command() -> list[str]:
    return [
        "kdialog",
        "--title",
        "sky-cua KWin effect",
        "--passivepopup",
        UPDATE_PENDING_MESSAGE,
        "30",
    ]


def parse_kwin_effect_list(stdout: str) -> list[str]:
    return [line.strip() for line in stdout.splitlines() if line.strip()]


def kwin_effect_preconditions(
    *,
    platform: str = sys.platform,
    which: Callable[[str], str | None] | None = None,
    kwin_header: Path = Path("/usr/include/kwin/effect/effect.h"),
    cursor_asset: Path = KWIN_EFFECT_CURSOR_ASSET,
) -> list[str]:
    """Return human-readable blockers for building/installing the effect."""
    import shutil

    resolve = which or shutil.which
    missing: list[str] = []
    if platform != "linux":
        missing.append(f"KWin effect deploy requires Linux (platform is {platform})")
        return missing
    for tool, hint in (
        ("cmake", "install the cmake package"),
        ("ninja", "install the ninja package"),
        ("qdbus6", "install qt6-tools (qdbus6)"),
        ("kwriteconfig6", "install kconfig (kwriteconfig6)"),
    ):
        if resolve(tool) is None:
            missing.append(f"{tool} is not on PATH ({hint})")
    if not kwin_header.exists():
        missing.append(f"KWin development headers are missing: {kwin_header} (install kwin)")
    if not cursor_asset.exists():
        missing.append(f"cursor asset is missing: {cursor_asset}")
    return missing


def run_kwin_effect_command(
    method: str,
    *,
    runner: Runner = default_runner,
) -> subprocess.CompletedProcess[str]:
    return runner(
        [
            "qdbus6",
            "org.kde.KWin",
            "/Effects",
            f"org.kde.kwin.Effects.{method}",
            KWIN_EFFECT_ID,
        ]
    )


def run_kwin_effects_property(
    property_name: str,
    *,
    runner: Runner = default_runner,
) -> subprocess.CompletedProcess[str]:
    return runner(
        [
            "qdbus6",
            "org.kde.KWin",
            "/Effects",
            "org.freedesktop.DBus.Properties.Get",
            "org.kde.kwin.Effects",
            property_name,
        ]
    )


def run_kwin_reconfigure(*, runner: Runner = default_runner) -> subprocess.CompletedProcess[str]:
    return runner(["qdbus6", "org.kde.KWin", "/KWin", "reconfigure"])


def kwin_effect_loaded(*, runner: Runner = default_runner) -> bool:
    status = run_kwin_effect_command("isEffectLoaded", runner=runner)
    return status.stdout.strip().lower() == "true"


def kwin_effect_supported(*, runner: Runner = default_runner) -> bool:
    status = run_kwin_effect_command("isEffectSupported", runner=runner)
    return status.stdout.strip().lower() == "true"


def running_effect_build_id(*, runner: Runner = default_runner) -> str:
    """BuildId reported by the loaded effect; "unknown" for legacy/unreachable."""
    result = runner(
        [
            "qdbus6",
            "org.kde.KWin",
            KWIN_AGENT_CURSOR_PATH,
            f"{KWIN_AGENT_CURSOR_INTERFACE}.BuildId",
        ]
    )
    if result.returncode != 0:
        return UNKNOWN_BUILD_ID
    value = result.stdout.strip()
    return value or UNKNOWN_BUILD_ID


def set_effect_enabled_config(
    enabled: bool,
    *,
    runner: Runner = default_runner,
) -> subprocess.CompletedProcess[str]:
    return runner(effect_enabled_config_command(enabled))


def detect_kwin_session(
    *,
    runner: Runner = default_runner,
    env: Mapping[str, str] | None = None,
) -> KwinSession:
    environment = env if env is not None else os.environ
    session_type = environment.get("XDG_SESSION_TYPE") or None
    service = runner(["systemctl", "--user", "is-active", KWIN_USER_SERVICE])
    service_active = service.returncode == 0 and service.stdout.strip() == "active"
    ping = runner(["qdbus6", "org.kde.KWin", "/KWin", "org.kde.KWin.currentDesktop"])
    return KwinSession(
        session_type=session_type,
        kwin_service_active=service_active,
        kwin_dbus_reachable=ping.returncode == 0,
    )


def notify_effect_update_pending(
    *,
    runner: Runner = default_runner,
    which: Callable[[str], str | None] | None = None,
) -> tuple[bool, str]:
    """Tell the user a session restart will activate the updated effect.

    Best effort: desktop notification first, kdialog passive popup as
    fallback; returns (delivered, how).
    """
    import shutil

    resolve = which or shutil.which
    if resolve("notify-send") is not None:
        result = runner(update_notification_command())
        if result.returncode == 0:
            return True, "notify-send"
    if resolve("kdialog") is not None:
        result = runner(update_notification_fallback_command())
        if result.returncode == 0:
            return True, "kdialog passive popup"
    return False, "no notification tool available"


def build_and_install_effect(
    build_dir: Path,
    *,
    install_prefix: Path = Path("/usr"),
    sudo_cmd: list[str] | None = None,
    build_id: str,
    runner: Runner = default_runner,
    echo: Callable[[str], None] = print,
) -> list[Path]:
    """Configure and build as the user, install via sudo; return manifest paths."""
    configure = cmake_configure_command(build_dir, install_prefix=install_prefix, build_id=build_id)
    for label, command in (
        ("configure", configure),
        ("build", cmake_build_command(build_dir)),
    ):
        result = runner(command)
        if result.returncode != 0:
            raise RuntimeError(
                f"KWin effect {label} failed ({shlex.join(command)}):\n{result.stderr.strip()}"
            )

    install = cmake_install_command(build_dir, sudo_cmd=sudo_cmd)
    echo(f"Installing the KWin effect (may prompt for credentials): {shlex.join(install)}")
    result = runner(install)
    if result.returncode != 0:
        raise RuntimeError(
            f"KWin effect install failed ({shlex.join(install)}):\n{result.stderr.strip()}"
        )

    manifest = build_dir / "install_manifest.txt"
    if not manifest.exists():
        return []
    return [
        Path(line) for line in manifest.read_text(encoding="utf-8").splitlines() if line.strip()
    ]


def _record(
    outcome: ReloadOutcome, label: str, result: subprocess.CompletedProcess[str] | None = None
) -> None:
    step: dict[str, Any] = {"step": label}
    if result is not None:
        step["returncode"] = result.returncode
        if result.stdout.strip():
            step["stdout"] = result.stdout.strip()
        if result.stderr.strip():
            step["stderr"] = result.stderr.strip()
    outcome.steps.append(step)


def _poll_for_convergence(
    outcome: ReloadOutcome,
    *,
    expected_build_id: str,
    deadline_s: float,
    runner: Runner,
    sleep: Callable[[float], None],
    clock: Callable[[], float],
) -> bool:
    deadline = clock() + deadline_s
    while True:
        loaded = kwin_effect_loaded(runner=runner)
        running = running_effect_build_id(runner=runner) if loaded else UNKNOWN_BUILD_ID
        outcome.loaded = loaded
        outcome.running_build_id = running
        if loaded and running == expected_build_id:
            return True
        if clock() >= deadline:
            return False
        sleep(0.5)


def reload_effect_until_converged(
    *,
    expected_build_id: str,
    notify: bool = True,
    enable_persistently: bool = True,
    runner: Runner = default_runner,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
    which: Callable[[str], str | None] | None = None,
) -> ReloadOutcome:
    """Drive the running KWin to the freshly installed effect build.

    Tries a hot unload/reconfigure/load cycle first; when the running build id
    stays stale (plugin loaders do not dlclose), reports that a Plasma session
    restart is pending and notifies the user. KWin is never restarted by this
    code: a compositor restart can take the whole session down.
    """
    outcome = ReloadOutcome(
        converged=False,
        loaded=False,
        expected_build_id=expected_build_id,
        running_build_id=UNKNOWN_BUILD_ID,
    )

    session = detect_kwin_session(runner=runner)
    if not session.kwin_dbus_reachable:
        if enable_persistently:
            _record(
                outcome,
                "enable kwinrc Plugins entry",
                set_effect_enabled_config(True, runner=runner),
            )
        outcome.notes.append(
            "KWin DBus is not reachable; the effect is installed and enabled and "
            "will load on the next Plasma session start."
        )
        return outcome

    if enable_persistently:
        _record(
            outcome, "enable kwinrc Plugins entry", set_effect_enabled_config(True, runner=runner)
        )

    if kwin_effect_loaded(runner=runner):
        running = running_effect_build_id(runner=runner)
        outcome.loaded = True
        outcome.running_build_id = running
        if running == expected_build_id:
            outcome.converged = True
            outcome.notes.append("running effect already matches the installed build")
            return outcome

    _record(outcome, "unloadEffect", run_kwin_effect_command("unloadEffect", runner=runner))
    _record(outcome, "reconfigure", run_kwin_reconfigure(runner=runner))
    _record(outcome, "loadEffect", run_kwin_effect_command("loadEffect", runner=runner))
    if _poll_for_convergence(
        outcome,
        expected_build_id=expected_build_id,
        deadline_s=5.0,
        runner=runner,
        sleep=sleep,
        clock=clock,
    ):
        outcome.converged = True
        outcome.notes.append("hot reload converged without a KWin restart")
        return outcome

    outcome.session_restart_required = True
    outcome.notes.append(
        "the replaced effect binary cannot hot-reload into the running KWin; "
        "the new build activates on the next Plasma session restart"
    )
    if notify:
        delivered, how = notify_effect_update_pending(runner=runner, which=which)
        outcome.notification_delivered = delivered
        outcome.notes.append(
            f"user notified via {how}" if delivered else f"notification not delivered ({how})"
        )
    return outcome


def effect_status(*, runner: Runner = default_runner) -> dict[str, Any]:
    session = detect_kwin_session(runner=runner)
    listing = run_kwin_effects_property("listOfEffects", runner=runner)
    effects = parse_kwin_effect_list(listing.stdout)
    loaded = kwin_effect_loaded(runner=runner)
    return {
        "effect_id": KWIN_EFFECT_ID,
        "session_type": session.session_type,
        "kwin_service_active": session.kwin_service_active,
        "kwin_dbus_reachable": session.kwin_dbus_reachable,
        "listed": KWIN_EFFECT_ID in effects,
        "supported": kwin_effect_supported(runner=runner),
        "loaded": loaded,
        "running_build_id": running_effect_build_id(runner=runner) if loaded else UNKNOWN_BUILD_ID,
        "expected_build_id": compute_effect_build_id(),
    }


def deploy_kwin_effect(
    *,
    build_dir: Path,
    install_prefix: Path = Path("/usr"),
    sudo_cmd: list[str] | None = None,
    notify: bool = True,
    enable_persistently: bool = True,
    runner: Runner = default_runner,
    echo: Callable[[str], None] = print,
) -> ReloadOutcome:
    """Full deploy: preconditions, build, sudo install, reload until converged."""
    missing = kwin_effect_preconditions()
    if missing:
        raise RuntimeError(
            "KWin effect deploy prerequisites are missing:\n  - " + "\n  - ".join(missing)
        )
    build_id = compute_effect_build_id()
    echo(f"KWin effect build id: {build_id}")
    build_and_install_effect(
        build_dir,
        install_prefix=install_prefix,
        sudo_cmd=sudo_cmd,
        build_id=build_id,
        runner=runner,
        echo=echo,
    )
    outcome = reload_effect_until_converged(
        expected_build_id=build_id,
        notify=notify,
        enable_persistently=enable_persistently,
        runner=runner,
    )
    for note in outcome.notes:
        echo(f"kwin-effect: {note}")
    return outcome
