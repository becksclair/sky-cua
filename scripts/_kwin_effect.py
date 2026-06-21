#!/usr/bin/env python3
"""Shared KWin effect build/install/reload primitives for the cursor shim.

Production deploys use rotating effect ids. Each build is installed under the
next generated id (``sky-cua-agent-cursor-000001``, ``...-000002``, ...),
KWin unloads the previously active id to release the stable DBus object path,
then loads the new id and verifies the unchanged ``BuildId()`` DBus contract.
The old same-id hot-reload path is intentionally not used: KWin does not dlclose
replaced effect libraries.
"""

from __future__ import annotations

import hashlib
import os
import re
import shlex
import subprocess
import sys
import time
from collections.abc import Callable, Iterable, Mapping, Sequence
from contextlib import suppress
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
KWIN_EFFECT_BASE_ID = "sky-cua-agent-cursor"
KWIN_EFFECT_ID = KWIN_EFFECT_BASE_ID
KWIN_EFFECT_RELOAD_STRATEGY = "rotating_effect_id"
KWIN_EFFECT_SOURCE_DIR = REPO_ROOT / "resources" / "kwin" / "effects" / KWIN_EFFECT_BASE_ID
KWIN_USER_SERVICE = "plasma-kwin_wayland.service"
KWIN_AGENT_CURSOR_PATH = "/com/skycua/AgentCursor"
KWIN_AGENT_CURSOR_INTERFACE = "com.skycua.AgentCursor"
UNKNOWN_BUILD_ID = "unknown"
KWIN_PLUGIN_RELATIVE_DIR = Path("lib/qt6/plugins/kwin/effects/plugins")
KWIN_METADATA_RELATIVE_DIRS = (
    Path("share/kwin/effects"),
    Path("share/kwin-wayland/effects"),
)
KWINRC_PATH = Path.home() / ".config" / "kwinrc"
EFFECT_ID_RE = re.compile(r"^sky-cua-agent-cursor(?:-(?P<generation>[0-9]{6}))?$")
KWINRC_EFFECT_KEY_RE = re.compile(
    r"^(?P<effect_id>sky-cua-agent-cursor(?:-[0-9]{6})?)Enabled\s*=\s*(?P<value>.*)$"
)

# Source globs that feed the build-id content hash. The generated effect id is
# a configure-time input and is deliberately not part of this content hash.
BUILD_ID_SOURCE_PATTERNS = (
    "*.cpp",
    "*.h",
    "CMakeLists.txt",
    "metadata.json.in",
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
    effect_id: str = KWIN_EFFECT_ID
    active_effect_id: str | None = None
    previous_effect_ids: list[str] = field(default_factory=list)
    rollback_effect_id: str | None = None
    live_load_attempted: bool = False
    session_restart_required: bool = False
    notification_delivered: bool = False
    cleanup_warnings: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)
    steps: list[dict[str, Any]] = field(default_factory=list)


def print_kwin_effect_deploy_outcome(outcome: ReloadOutcome) -> None:
    """Print compatibility restart messages for unexpected legacy outcomes."""
    for warning in outcome.cleanup_warnings:
        print(f"KWin effect cleanup warning: {warning}", file=sys.stderr)
    if not outcome.session_restart_required:
        return
    if outcome.notification_delivered:
        print(
            "KWin effect updated; the new build activates after the next "
            "Plasma session restart (a desktop notification was shown)."
        )
    else:
        print(
            "KWin effect updated; the new build activates after the next "
            "Plasma session restart. The desktop notification could not "
            "be delivered - tell the user to restart their session when "
            "convenient."
        )


def kwin_effect_deploy_failed(outcome: ReloadOutcome) -> bool:
    """True when a live reload was attempted and failed to activate the new id."""
    return outcome.live_load_attempted and not outcome.converged


def is_sky_cua_effect_id(effect_id: str) -> bool:
    return EFFECT_ID_RE.fullmatch(effect_id) is not None


def effect_generation(effect_id: str) -> int | None:
    match = EFFECT_ID_RE.fullmatch(effect_id)
    if match is None:
        return None
    generation = match.group("generation")
    return 0 if generation is None else int(generation)


def generated_effect_id(generation: int) -> str:
    if generation < 1 or generation > 999999:
        raise ValueError(f"effect generation out of range: {generation}")
    return f"{KWIN_EFFECT_BASE_ID}-{generation:06d}"


def sort_effect_ids(effect_ids: Iterable[str]) -> list[str]:
    valid_ids = [effect_id for effect_id in set(effect_ids) if is_sky_cua_effect_id(effect_id)]
    return sorted(valid_ids, key=lambda effect_id: (effect_generation(effect_id) or 0, effect_id))


def next_generated_effect_id(effect_ids: Iterable[str]) -> str:
    generations = [
        generation
        for effect_id in effect_ids
        if (generation := effect_generation(effect_id)) is not None
    ]
    return generated_effect_id((max(generations) if generations else 0) + 1)


def compute_effect_build_id(
    source_dir: Path = KWIN_EFFECT_SOURCE_DIR,
) -> str:
    """Content hash over effect sources (16 hex chars), independent of effect id."""
    digest = hashlib.sha256()
    paths: list[Path] = []
    for pattern in BUILD_ID_SOURCE_PATTERNS:
        paths.extend(source_dir.glob(pattern))
    for path in sorted(p for p in paths if p.is_file()):
        relative = path.relative_to(source_dir)
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
    effect_id: str = KWIN_EFFECT_ID,
    source_dir: Path = KWIN_EFFECT_SOURCE_DIR,
) -> list[str]:
    if not is_sky_cua_effect_id(effect_id):
        raise ValueError(f"invalid sky-cua KWin effect id: {effect_id}")
    return [
        "cmake",
        "-S",
        str(source_dir),
        "-B",
        str(build_dir),
        "-G",
        "Ninja",
        f"-DCMAKE_INSTALL_PREFIX={install_prefix}",
        f"-DSKY_CUA_EFFECT_BUILD_ID={build_id}",
        f"-DSKY_CUA_EFFECT_ID={effect_id}",
    ]


def cmake_build_command(build_dir: Path) -> list[str]:
    return ["cmake", "--build", str(build_dir)]


def cmake_install_command(build_dir: Path, *, sudo_cmd: list[str] | None = None) -> list[str]:
    return [*(sudo_cmd if sudo_cmd is not None else ["sudo"]), "cmake", "--install", str(build_dir)]


def effect_enabled_config_command(enabled: bool, *, effect_id: str = KWIN_EFFECT_ID) -> list[str]:
    if not is_sky_cua_effect_id(effect_id):
        raise ValueError(f"invalid sky-cua KWin effect id: {effect_id}")
    return [
        "kwriteconfig6",
        "--file",
        "kwinrc",
        "--group",
        "Plugins",
        "--key",
        f"{effect_id}Enabled",
        "true" if enabled else "false",
    ]


def effect_delete_config_command(effect_id: str) -> list[str]:
    if not is_sky_cua_effect_id(effect_id):
        raise ValueError(f"invalid sky-cua KWin effect id: {effect_id}")
    return [
        "kwriteconfig6",
        "--file",
        "kwinrc",
        "--group",
        "Plugins",
        "--key",
        f"{effect_id}Enabled",
        "--delete",
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


def sky_cua_effect_ids(effect_ids: Iterable[str]) -> list[str]:
    return sort_effect_ids(effect_id for effect_id in effect_ids if is_sky_cua_effect_id(effect_id))


def kwinrc_enabled_effect_ids(config_path: Path | None = None) -> list[str]:
    path = config_path or KWINRC_PATH
    if not path.exists():
        return []
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return []

    in_plugins = False
    effect_ids: list[str] = []
    for raw_line in lines:
        line = raw_line.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("[") and line.endswith("]"):
            in_plugins = line == "[Plugins]"
            continue
        if not in_plugins:
            continue
        match = KWINRC_EFFECT_KEY_RE.fullmatch(line)
        if match is None:
            continue
        value = match.group("value").strip().lower()
        if value == "true":
            effect_ids.append(match.group("effect_id"))
    return sort_effect_ids(effect_ids)


def installed_effect_ids(install_prefix: Path = Path("/usr")) -> list[str]:
    effect_ids: set[str] = set()
    plugin_dir = install_prefix / KWIN_PLUGIN_RELATIVE_DIR
    with suppress(OSError):
        for child in plugin_dir.iterdir():
            if child.is_file() and child.suffix == ".so" and is_sky_cua_effect_id(child.stem):
                effect_ids.add(child.stem)
    for relative_dir in KWIN_METADATA_RELATIVE_DIRS:
        root = install_prefix / relative_dir
        with suppress(OSError):
            for child in root.iterdir():
                if child.is_dir() and is_sky_cua_effect_id(child.name):
                    effect_ids.add(child.name)
    return sort_effect_ids(effect_ids)


def discover_candidate_effect_ids(
    *,
    runner: Runner = default_runner,
    install_prefix: Path = Path("/usr"),
    kwinrc_path: Path | None = None,
) -> list[str]:
    listed = listed_effect_ids(runner=runner)
    installed = installed_effect_ids(install_prefix=install_prefix)
    enabled = kwinrc_enabled_effect_ids(kwinrc_path)
    return sort_effect_ids([*listed, *installed, *enabled])


def effect_install_paths(effect_id: str, install_prefix: Path = Path("/usr")) -> list[Path]:
    if not is_sky_cua_effect_id(effect_id):
        raise ValueError(f"invalid sky-cua KWin effect id: {effect_id}")
    return [
        install_prefix / KWIN_PLUGIN_RELATIVE_DIR / f"{effect_id}.so",
        *[
            install_prefix / relative_dir / effect_id
            for relative_dir in KWIN_METADATA_RELATIVE_DIRS
        ],
    ]


def effect_remove_command(
    effect_id: str,
    *,
    install_prefix: Path = Path("/usr"),
    sudo_cmd: list[str] | None = None,
) -> list[str]:
    return [
        *(sudo_cmd if sudo_cmd is not None else ["sudo"]),
        "rm",
        "-rf",
        *[str(path) for path in effect_install_paths(effect_id, install_prefix=install_prefix)],
    ]


def kwin_effect_preconditions(
    *,
    platform: str = sys.platform,
    which: Callable[[str], str | None] | None = None,
    kwin_header: Path = Path("/usr/include/kwin/effect/effect.h"),
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
    return missing


def run_kwin_effect_command(
    method: str,
    *,
    effect_id: str = KWIN_EFFECT_ID,
    runner: Runner = default_runner,
) -> subprocess.CompletedProcess[str]:
    if not is_sky_cua_effect_id(effect_id):
        raise ValueError(f"invalid sky-cua KWin effect id: {effect_id}")
    return runner(
        [
            "qdbus6",
            "org.kde.KWin",
            "/Effects",
            f"org.kde.kwin.Effects.{method}",
            effect_id,
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


def listed_effect_ids(*, runner: Runner = default_runner) -> list[str]:
    listing = run_kwin_effects_property("listOfEffects", runner=runner)
    return sky_cua_effect_ids(parse_kwin_effect_list(listing.stdout))


def loaded_effect_ids(*, runner: Runner = default_runner) -> list[str]:
    loaded = run_kwin_effects_property("loadedEffects", runner=runner)
    return sky_cua_effect_ids(parse_kwin_effect_list(loaded.stdout))


def run_kwin_reconfigure(*, runner: Runner = default_runner) -> subprocess.CompletedProcess[str]:
    return runner(["qdbus6", "org.kde.KWin", "/KWin", "reconfigure"])


def kwin_effect_loaded(*, effect_id: str = KWIN_EFFECT_ID, runner: Runner = default_runner) -> bool:
    status = run_kwin_effect_command("isEffectLoaded", effect_id=effect_id, runner=runner)
    return status.stdout.strip().lower() == "true"


def kwin_effect_supported(
    *, effect_id: str = KWIN_EFFECT_ID, runner: Runner = default_runner
) -> bool:
    status = run_kwin_effect_command("isEffectSupported", effect_id=effect_id, runner=runner)
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
    effect_id: str = KWIN_EFFECT_ID,
    runner: Runner = default_runner,
) -> subprocess.CompletedProcess[str]:
    return runner(effect_enabled_config_command(enabled, effect_id=effect_id))


def delete_effect_enabled_config(
    effect_id: str, *, runner: Runner = default_runner
) -> subprocess.CompletedProcess[str]:
    return runner(effect_delete_config_command(effect_id))


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
    """Legacy best-effort notification helper for non-rotating fallback paths."""
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
    effect_id: str = KWIN_EFFECT_ID,
    runner: Runner = default_runner,
    echo: Callable[[str], None] = print,
) -> list[Path]:
    """Configure and build as the user, install via sudo; return manifest paths."""
    configure = cmake_configure_command(
        build_dir,
        install_prefix=install_prefix,
        build_id=build_id,
        effect_id=effect_id,
    )
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
    echo(f"Installing KWin effect {effect_id} (may prompt for credentials): {shlex.join(install)}")
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


def _record_cleanup_warning(
    outcome: ReloadOutcome,
    label: str,
    effect_id: str,
    result: subprocess.CompletedProcess[str],
) -> None:
    _record(outcome, f"{label} {effect_id}", result)
    if result.returncode != 0:
        detail = (
            result.stderr.strip() or result.stdout.strip() or f"return code {result.returncode}"
        )
        outcome.cleanup_warnings.append(f"{label} {effect_id}: {detail}")


def _poll_for_convergence(
    outcome: ReloadOutcome,
    *,
    effect_id: str,
    expected_build_id: str,
    deadline_s: float,
    runner: Runner,
    sleep: Callable[[float], None],
    clock: Callable[[], float],
) -> bool:
    deadline = clock() + deadline_s
    while True:
        loaded = kwin_effect_loaded(effect_id=effect_id, runner=runner)
        running = running_effect_build_id(runner=runner) if loaded else UNKNOWN_BUILD_ID
        outcome.loaded = loaded
        outcome.running_build_id = running
        outcome.active_effect_id = effect_id if loaded else None
        if loaded and running == expected_build_id:
            return True
        if clock() >= deadline:
            return False
        sleep(0.5)


def _disable_previous_effect_configs(
    effect_ids: Sequence[str],
    *,
    runner: Runner,
    outcome: ReloadOutcome,
) -> None:
    for old_id in sort_effect_ids(effect_ids):
        _record(
            outcome,
            f"disable kwinrc Plugins entry {old_id}",
            set_effect_enabled_config(False, effect_id=old_id, runner=runner),
        )


def _cleanup_old_effect_ids(
    effect_ids: Sequence[str],
    *,
    install_prefix: Path,
    sudo_cmd: list[str] | None,
    runner: Runner,
    outcome: ReloadOutcome,
) -> None:
    for old_id in sort_effect_ids(effect_ids):
        _record_cleanup_warning(
            outcome,
            "unload old effect",
            old_id,
            run_kwin_effect_command("unloadEffect", effect_id=old_id, runner=runner),
        )
        _record_cleanup_warning(
            outcome,
            "delete kwinrc Plugins entry",
            old_id,
            delete_effect_enabled_config(old_id, runner=runner),
        )
        _record_cleanup_warning(
            outcome,
            "remove old effect files",
            old_id,
            runner(effect_remove_command(old_id, install_prefix=install_prefix, sudo_cmd=sudo_cmd)),
        )


def reload_effect_until_converged(
    *,
    expected_build_id: str,
    effect_id: str = KWIN_EFFECT_ID,
    previous_effect_ids: Sequence[str] | None = None,
    install_prefix: Path = Path("/usr"),
    sudo_cmd: list[str] | None = None,
    notify: bool = True,
    enable_persistently: bool = True,
    runner: Runner = default_runner,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
    which: Callable[[str], str | None] | None = None,
) -> ReloadOutcome:
    """Load a freshly installed generated effect id and clean stale ids on success."""
    del notify, which
    if not is_sky_cua_effect_id(effect_id):
        raise ValueError(f"invalid sky-cua KWin effect id: {effect_id}")

    old_ids = sort_effect_ids(id_ for id_ in (previous_effect_ids or []) if id_ != effect_id)
    outcome = ReloadOutcome(
        converged=False,
        loaded=False,
        expected_build_id=expected_build_id,
        running_build_id=UNKNOWN_BUILD_ID,
        effect_id=effect_id,
        previous_effect_ids=old_ids,
    )

    session = detect_kwin_session(runner=runner)
    if not session.kwin_dbus_reachable:
        if enable_persistently:
            _record(
                outcome,
                f"enable kwinrc Plugins entry {effect_id}",
                set_effect_enabled_config(True, effect_id=effect_id, runner=runner),
            )
            _disable_previous_effect_configs(old_ids, runner=runner, outcome=outcome)
        outcome.notes.append(
            "KWin DBus is not reachable; the generated effect id is installed and "
            "enabled for the next Plasma session start."
        )
        return outcome

    loaded_before = loaded_effect_ids(runner=runner)
    unload_before_load = sort_effect_ids([*old_ids, *loaded_before])
    previous_active_id = loaded_before[-1] if loaded_before else (old_ids[-1] if old_ids else None)

    if enable_persistently:
        _record(
            outcome,
            f"enable kwinrc Plugins entry {effect_id}",
            set_effect_enabled_config(True, effect_id=effect_id, runner=runner),
        )

    if kwin_effect_loaded(effect_id=effect_id, runner=runner):
        running = running_effect_build_id(runner=runner)
        outcome.loaded = True
        outcome.active_effect_id = effect_id
        outcome.running_build_id = running
        if running == expected_build_id:
            outcome.converged = True
            _cleanup_old_effect_ids(
                [old_id for old_id in unload_before_load if old_id != effect_id],
                install_prefix=install_prefix,
                sudo_cmd=sudo_cmd,
                runner=runner,
                outcome=outcome,
            )
            outcome.notes.append("running generated effect id already matches the installed build")
            return outcome

    for old_id in [old_id for old_id in unload_before_load if old_id != effect_id]:
        _record(
            outcome,
            f"unloadEffect {old_id}",
            run_kwin_effect_command("unloadEffect", effect_id=old_id, runner=runner),
        )

    _record(outcome, "reconfigure", run_kwin_reconfigure(runner=runner))
    outcome.live_load_attempted = True
    _record(
        outcome,
        f"loadEffect {effect_id}",
        run_kwin_effect_command("loadEffect", effect_id=effect_id, runner=runner),
    )
    if _poll_for_convergence(
        outcome,
        effect_id=effect_id,
        expected_build_id=expected_build_id,
        deadline_s=5.0,
        runner=runner,
        sleep=sleep,
        clock=clock,
    ):
        outcome.converged = True
        outcome.active_effect_id = effect_id
        _cleanup_old_effect_ids(
            [old_id for old_id in unload_before_load if old_id != effect_id],
            install_prefix=install_prefix,
            sudo_cmd=sudo_cmd,
            runner=runner,
            outcome=outcome,
        )
        outcome.notes.append("rotating effect id loaded without a KWin restart")
        return outcome

    _record(
        outcome,
        f"unload failed new effect {effect_id}",
        run_kwin_effect_command("unloadEffect", effect_id=effect_id, runner=runner),
    )
    outcome.loaded = False
    outcome.active_effect_id = None
    if enable_persistently:
        _record(
            outcome,
            f"disable failed kwinrc Plugins entry {effect_id}",
            set_effect_enabled_config(False, effect_id=effect_id, runner=runner),
        )
    if previous_active_id is not None:
        outcome.rollback_effect_id = previous_active_id
        if enable_persistently:
            _record(
                outcome,
                f"re-enable previous kwinrc Plugins entry {previous_active_id}",
                set_effect_enabled_config(True, effect_id=previous_active_id, runner=runner),
            )
        _record(outcome, "rollback reconfigure", run_kwin_reconfigure(runner=runner))
        _record(
            outcome,
            f"reload previous effect {previous_active_id}",
            run_kwin_effect_command(
                "loadEffect",
                effect_id=previous_active_id,
                runner=runner,
            ),
        )
        outcome.active_effect_id = previous_active_id
        outcome.notes.append(
            f"generated effect id {effect_id} did not converge; restored {previous_active_id}"
        )
    else:
        outcome.notes.append(f"generated effect id {effect_id} did not converge")
    return outcome


def _active_effect_id(loaded_ids: Sequence[str]) -> str | None:
    sorted_loaded = sort_effect_ids(loaded_ids)
    return sorted_loaded[-1] if sorted_loaded else None


def effect_status(
    *,
    runner: Runner = default_runner,
    install_prefix: Path = Path("/usr"),
    kwinrc_path: Path | None = None,
) -> dict[str, Any]:
    session = detect_kwin_session(runner=runner)
    listed_ids = listed_effect_ids(runner=runner)
    loaded_ids = loaded_effect_ids(runner=runner)
    installed_ids = installed_effect_ids(install_prefix=install_prefix)
    enabled_ids = kwinrc_enabled_effect_ids(kwinrc_path)
    candidate_ids = sort_effect_ids([*listed_ids, *loaded_ids, *installed_ids, *enabled_ids])
    active_id = _active_effect_id(loaded_ids)
    expected_effect_id = active_id or next_generated_effect_id(candidate_ids)
    loaded = active_id == expected_effect_id
    return {
        "effect_id": expected_effect_id,
        "base_effect_id": KWIN_EFFECT_BASE_ID,
        "active_effect_id": active_id,
        "candidate_effect_ids": candidate_ids,
        "loaded_effect_ids": loaded_ids,
        "stale_effect_ids": [
            candidate_id for candidate_id in candidate_ids if candidate_id != active_id
        ],
        "reload_strategy": KWIN_EFFECT_RELOAD_STRATEGY,
        "session_type": session.session_type,
        "kwin_service_active": session.kwin_service_active,
        "kwin_dbus_reachable": session.kwin_dbus_reachable,
        "listed": expected_effect_id in listed_ids,
        "supported": kwin_effect_supported(effect_id=expected_effect_id, runner=runner),
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
    """Full rotating deploy: build next id, load it, then clean stale ids."""
    missing = kwin_effect_preconditions()
    if missing:
        raise RuntimeError(
            "KWin effect deploy prerequisites are missing:\n  - " + "\n  - ".join(missing)
        )

    previous_ids = discover_candidate_effect_ids(runner=runner, install_prefix=install_prefix)
    effect_id = next_generated_effect_id(previous_ids)
    build_id = compute_effect_build_id()
    echo(f"KWin effect build id: {build_id}")
    echo(f"KWin effect rotating id: {effect_id}")
    build_and_install_effect(
        build_dir,
        install_prefix=install_prefix,
        sudo_cmd=sudo_cmd,
        build_id=build_id,
        effect_id=effect_id,
        runner=runner,
        echo=echo,
    )
    outcome = reload_effect_until_converged(
        expected_build_id=build_id,
        effect_id=effect_id,
        previous_effect_ids=previous_ids,
        install_prefix=install_prefix,
        sudo_cmd=sudo_cmd,
        notify=notify,
        enable_persistently=enable_persistently,
        runner=runner,
    )
    for note in outcome.notes:
        echo(f"kwin-effect: {note}")
    for warning in outcome.cleanup_warnings:
        echo(f"kwin-effect cleanup warning: {warning}")
    return outcome
