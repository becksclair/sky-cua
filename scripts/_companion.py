#!/usr/bin/env python3
"""Build and stage the Android phone companion APK for plugin bundling.

The companion APK and its identity sidecar are bundled by ``build_plugin.py``
from ``resources/android/phone-companion.{apk,json}`` when present. Producing
those staged artifacts used to be a manual ``gradlew assembleDebug`` plus a
rename; this module makes it an automatic lane of the local deploy
(``deploy_plugin.py``), mirroring the ``_kwin_effect`` lane.

The build is gated so it never breaks a deploy on a machine without the Android
toolchain: when JDK 21 and the Android SDK are not both resolvable the lane
skips with a note (ADB-baseline phone-use is unaffected and any previously
staged APK is reused). When the toolchain is present the lane rebuilds only when
the companion sources changed since the last staged APK, so a pure-Rust deploy
is not slowed by Gradle. ``force`` overrides the freshness check.

Gradle emits the APK at ``android/phone-companion/app/build/outputs/apk/debug/
app-debug.apk`` and, via the ``emitBuildMetadata`` finalizer, the identity
sidecar at ``android/phone-companion/build-metadata.json``. This stages both to
``resources/android/phone-companion.{apk,json}``. All subprocess calls accept an
injectable runner so the decision logic stays unit-testable without Gradle.
"""

from __future__ import annotations

import json
import os
import shlex
import shutil
import subprocess
from collections.abc import Callable, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]

COMPANION_PROJECT_DIR = REPO_ROOT / "android" / "phone-companion"
COMPANION_GRADLEW = COMPANION_PROJECT_DIR / "gradlew"
COMPANION_BUILT_APK = (
    COMPANION_PROJECT_DIR / "app" / "build" / "outputs" / "apk" / "debug" / "app-debug.apk"
)
COMPANION_BUILT_METADATA = COMPANION_PROJECT_DIR / "build-metadata.json"

STAGED_APK = REPO_ROOT / "resources" / "android" / "phone-companion.apk"
STAGED_METADATA = REPO_ROOT / "resources" / "android" / "phone-companion.json"

# Source trees whose mtimes decide whether a rebuild is needed. Kept coarse on
# purpose: a touched build script, manifest, or any app/protocol source forces a
# rebuild; generated build output under `app/build` is excluded.
COMPANION_SOURCE_DIRS = (
    COMPANION_PROJECT_DIR / "app" / "src",
    COMPANION_PROJECT_DIR / "app" / "build.gradle.kts",
    COMPANION_PROJECT_DIR / "build.gradle.kts",
    COMPANION_PROJECT_DIR / "settings.gradle.kts",
    COMPANION_PROJECT_DIR / "gradle.properties",
    # The version catalog: a dependency/AGP bump here changes the built APK
    # without touching any source tree above, so it must force a rebuild too.
    COMPANION_PROJECT_DIR / "gradle" / "libs.versions.toml",
)

# Explicit overrides so a non-standard host can point the lane at its toolchain.
COMPANION_JAVA_HOME_ENV = "SKY_CUA_COMPANION_JAVA_HOME"
COMPANION_SDK_ROOT_ENV = "SKY_CUA_COMPANION_ANDROID_SDK_ROOT"

# JDK 21 candidates. AGP rejects the host default `java` (newer), so the lane
# resolves a 21 JDK explicitly and sets `JAVA_HOME` for the Gradle subprocess.
DEFAULT_JAVA_HOME_CANDIDATES = (
    Path("/usr/lib/jvm/java-21-openjdk"),
    Path("/usr/lib/jvm/java-21-openjdk-amd64"),
    Path("/usr/lib/jvm/jdk-21"),
)

Runner = Callable[[list[str], Mapping[str, str]], "subprocess.CompletedProcess[str]"]


def default_runner(command: list[str], env: Mapping[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
        env={**os.environ, **env},
    )


@dataclass(frozen=True)
class CompanionToolchain:
    """Resolved paths the Gradle build needs."""

    java_home: Path
    android_sdk_root: Path


@dataclass
class CompanionBuildOutcome:
    """Result of a companion build/stage attempt."""

    status: str  # "built" | "skipped_unchanged" | "skipped_no_toolchain" | "failed"
    notes: list[str] = field(default_factory=list)
    staged_apk: Path | None = None
    staged_metadata: Path | None = None

    @property
    def built(self) -> bool:
        return self.status == "built"


def _java_home_candidates(env: Mapping[str, str], candidates: tuple[Path, ...]) -> list[Path]:
    """Ordered JDK-21 candidates: explicit override, then a 21-shaped JAVA_HOME,
    then the well-known distro paths."""
    resolved: list[Path] = []
    override = env.get(COMPANION_JAVA_HOME_ENV, "").strip()
    if override:
        resolved.append(Path(override))
    existing = env.get("JAVA_HOME", "").strip()
    # Only trust an inherited JAVA_HOME when it is clearly a 21 JDK; the host
    # default is a newer JDK that AGP rejects.
    if existing and "21" in Path(existing).name:
        resolved.append(Path(existing))
    resolved.extend(candidates)
    return resolved


def resolve_java_home(
    env: Mapping[str, str] | None = None,
    *,
    candidates: tuple[Path, ...] = DEFAULT_JAVA_HOME_CANDIDATES,
) -> Path | None:
    """First JDK-21 candidate that looks like a usable JDK (`bin/javac`)."""
    environment = env if env is not None else os.environ
    for candidate in _java_home_candidates(environment, candidates):
        if (candidate / "bin" / "javac").exists():
            return candidate
    return None


def resolve_android_sdk_root(
    env: Mapping[str, str] | None = None,
    *,
    local_properties: Path = COMPANION_PROJECT_DIR / "local.properties",
) -> Path | None:
    """Resolve the Android SDK: explicit override, the standard env vars, the
    default `~/Android/Sdk`, then `local.properties` `sdk.dir`."""
    environment = env if env is not None else os.environ
    for key in (COMPANION_SDK_ROOT_ENV, "ANDROID_SDK_ROOT", "ANDROID_HOME"):
        value = environment.get(key, "").strip()
        if value and Path(value).is_dir():
            return Path(value)
    home = environment.get("HOME", "").strip()
    if home:
        default_sdk = Path(home) / "Android" / "Sdk"
        if default_sdk.is_dir():
            return default_sdk
    sdk_dir = _sdk_dir_from_local_properties(local_properties)
    if sdk_dir is not None and sdk_dir.is_dir():
        return sdk_dir
    return None


def _sdk_dir_from_local_properties(path: Path) -> Path | None:
    if not path.exists():
        return None
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("sdk.dir="):
            # `local.properties` escapes `:` on some platforms; Linux paths do not.
            return Path(stripped.split("=", 1)[1].strip().replace("\\:", ":"))
    return None


def resolve_companion_toolchain(env: Mapping[str, str] | None = None) -> CompanionToolchain | None:
    """The JDK 21 + Android SDK the Gradle build needs, or None when either is
    missing (the lane then skips gracefully)."""
    java_home = resolve_java_home(env)
    sdk_root = resolve_android_sdk_root(env)
    if java_home is None or sdk_root is None:
        return None
    return CompanionToolchain(java_home=java_home, android_sdk_root=sdk_root)


def companion_sources_changed(
    *,
    staged_apk: Path = STAGED_APK,
    source_paths: tuple[Path, ...] = COMPANION_SOURCE_DIRS,
) -> bool:
    """Whether any companion source is newer than the staged APK.

    A missing staged APK always counts as changed (nothing to reuse). A source
    file/dir that does not exist is ignored. Directories are scanned recursively.
    """
    if not staged_apk.exists():
        return True
    staged_mtime = staged_apk.stat().st_mtime
    return any(_newest_mtime(source) > staged_mtime for source in source_paths)


def _newest_mtime(path: Path) -> float:
    if path.is_file():
        return path.stat().st_mtime
    if path.is_dir():
        newest = 0.0
        for child in path.rglob("*"):
            if child.is_file():
                newest = max(newest, child.stat().st_mtime)
        return newest
    return 0.0


def gradle_assemble_command() -> list[str]:
    """`gradlew -p android/phone-companion :app:assembleDebug` (metadata sidecar
    is emitted by the `emitBuildMetadata` finalizer)."""
    return [
        str(COMPANION_GRADLEW),
        "-p",
        str(COMPANION_PROJECT_DIR),
        ":app:assembleDebug",
        "--console=plain",
    ]


def gradle_env(toolchain: CompanionToolchain) -> dict[str, str]:
    return {
        "JAVA_HOME": str(toolchain.java_home),
        "ANDROID_SDK_ROOT": str(toolchain.android_sdk_root),
    }


def stage_companion_artifacts(
    *,
    built_apk: Path = COMPANION_BUILT_APK,
    built_metadata: Path = COMPANION_BUILT_METADATA,
    staged_apk: Path = STAGED_APK,
    staged_metadata: Path = STAGED_METADATA,
) -> None:
    """Copy the Gradle outputs to the staged names `build_plugin.py` bundles."""
    if not built_apk.exists():
        raise RuntimeError(f"companion build produced no APK at {built_apk}")
    staged_apk.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(built_apk, staged_apk)
    if built_metadata.exists():
        shutil.copy2(built_metadata, staged_metadata)
    elif staged_metadata.exists():
        # The build emitted no metadata sidecar but a stale one is staged next to
        # the freshly copied APK. Leaving it would feed the runtime signature gate
        # the wrong cert/APK hash; drop it so the runtime falls back to all-None.
        staged_metadata.unlink()


def build_and_stage_companion(
    *,
    force: bool = False,
    runner: Runner = default_runner,
    echo: Callable[[str], None] = print,
    env: Mapping[str, str] | None = None,
) -> CompanionBuildOutcome:
    """Build the companion APK with Gradle and stage it for bundling.

    Skips gracefully when the Android toolchain is unavailable, and (unless
    ``force``) when the sources are unchanged since the last staged APK.
    """
    toolchain = resolve_companion_toolchain(env)
    if toolchain is None:
        note = (
            "Android toolchain not found (need JDK 21 + Android SDK); skipping "
            "companion build. ADB-baseline phone-use is unaffected; any previously "
            "staged APK is reused."
        )
        echo(f"companion: {note}")
        return CompanionBuildOutcome(status="skipped_no_toolchain", notes=[note])

    if not force and not companion_sources_changed():
        note = "companion sources unchanged since the staged APK; skipping rebuild."
        echo(f"companion: {note}")
        return CompanionBuildOutcome(
            status="skipped_unchanged",
            notes=[note],
            staged_apk=STAGED_APK if STAGED_APK.exists() else None,
            staged_metadata=STAGED_METADATA if STAGED_METADATA.exists() else None,
        )

    command = gradle_assemble_command()
    echo(f"companion: building APK ({shlex.join(command)})")
    result = runner(command, gradle_env(toolchain))
    if result.returncode != 0:
        raise RuntimeError(
            f"companion Gradle build failed ({shlex.join(command)}):\n{result.stderr.strip()}"
        )

    stage_companion_artifacts()
    echo(f"companion: staged APK at {STAGED_APK}")
    return CompanionBuildOutcome(
        status="built",
        notes=[f"built and staged companion APK from {COMPANION_BUILT_APK}"],
        staged_apk=STAGED_APK,
        staged_metadata=STAGED_METADATA if STAGED_METADATA.exists() else None,
    )


def print_companion_build_outcome(outcome: CompanionBuildOutcome) -> None:
    """User-facing summary after a companion build lane runs."""
    if outcome.status == "built":
        print(f"Companion APK rebuilt and staged at {outcome.staged_apk}.")
    elif outcome.status == "skipped_unchanged":
        print("Companion APK unchanged; reused the existing staged build.")
    elif outcome.status == "skipped_no_toolchain":
        print("Companion build skipped (no Android toolchain); using any staged APK as-is.")


# ---------------------------------------------------------------------------
# Device-setup handoff
# ---------------------------------------------------------------------------
# A build-bearing deploy stages + bundles the companion host-side but does not
# install it onto a phone or enable its services: that is a runtime step
# (phone_connect -> phone_install_companion, which installs over ADB and flips
# the accessibility + notification-listener grants). To keep the human in the
# loop, the deploy emits a status the calling agent acts on — it lists the
# currently connected devices and tells the agent to ask the user which one(s)
# to set up, rather than silently pushing the APK to whatever is on the bus.

_ADB_DEVICE_STATES = frozenset(
    {
        "device",
        "offline",
        "unauthorized",
        "bootloader",
        "recovery",
        "sideload",
        "connecting",
        "authorizing",
    }
)


@dataclass(frozen=True)
class AdbDevice:
    serial: str
    state: str
    model: str | None


@dataclass(frozen=True)
class CompanionSetupStatus:
    """What a deploy hands the agent so it can finish device setup."""

    staged: bool
    version_name: str | None
    version_code: int | None
    apk_sha256: str | None
    devices: tuple[AdbDevice, ...]


def _as_str(value: object) -> str | None:
    return value if isinstance(value, str) else None


def read_staged_companion_metadata(path: Path = STAGED_METADATA) -> dict[str, Any] | None:
    """Parse the staged identity sidecar, or None when absent/unreadable."""
    if not path.exists():
        return None
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return None
    return loaded if isinstance(loaded, dict) else None


def parse_adb_devices(stdout: str) -> list[AdbDevice]:
    """Parse `adb devices -l` into typed device lines (serial, state, model)."""
    devices: list[AdbDevice] = []
    for line in stdout.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith(("List of devices", "*")):
            continue
        parts = stripped.split()
        if len(parts) < 2 or parts[1] not in _ADB_DEVICE_STATES:
            continue
        model = next(
            (token.split(":", 1)[1] for token in parts[2:] if token.startswith("model:")),
            None,
        )
        devices.append(AdbDevice(serial=parts[0], state=parts[1], model=model))
    return devices


def list_adb_devices(
    *,
    runner: Runner = default_runner,
    env: Mapping[str, str] | None = None,
    which: Callable[[str], str | None] = shutil.which,
) -> list[AdbDevice]:
    """Best-effort `adb devices -l` enumeration; empty when adb is unavailable."""
    environment = env if env is not None else os.environ
    adb = environment.get("SKY_CUA_ADB", "").strip() or which("adb")
    if not adb:
        return []
    try:
        result = runner([adb, "devices", "-l"], {})
    except OSError:
        return []
    if result.returncode != 0:
        return []
    return parse_adb_devices(result.stdout)


def companion_setup_status(
    *,
    staged_apk: Path = STAGED_APK,
    staged_metadata: Path = STAGED_METADATA,
    runner: Runner = default_runner,
    env: Mapping[str, str] | None = None,
) -> CompanionSetupStatus:
    """Assemble the post-deploy device-setup handoff: the staged companion
    identity plus the currently connected devices.

    Devices are only enumerated when a companion is actually staged; with nothing
    to install there is no reason to probe adb.
    """
    if not staged_apk.exists():
        return CompanionSetupStatus(
            staged=False,
            version_name=None,
            version_code=None,
            apk_sha256=None,
            devices=(),
        )
    metadata = read_staged_companion_metadata(staged_metadata) or {}
    version_code = metadata.get("version_code")
    return CompanionSetupStatus(
        staged=True,
        version_name=_as_str(metadata.get("version_name")),
        version_code=version_code if isinstance(version_code, int) else None,
        apk_sha256=_as_str(metadata.get("apk_sha256")),
        devices=tuple(list_adb_devices(runner=runner, env=env)),
    )


def print_companion_setup_status(status: CompanionSetupStatus) -> None:
    """Emit the device-setup handoff the calling agent acts on.

    Lists connected devices and instructs the agent to ask the user which one(s)
    to set up, then run the runtime install workflow. Silent when no companion is
    bundled (nothing to install).
    """
    if not status.staged:
        return
    version = status.version_name or "unknown"
    sha = f" sha {status.apk_sha256[:16]}…" if status.apk_sha256 else ""
    print(
        f"[companion] staged + bundled: version {version} (versionCode {status.version_code}){sha}."
    )
    if not status.devices:
        print(
            "[companion] no adb devices connected. To finish setup, connect a device "
            "and run the runtime workflow: phone_connect then phone_install_companion "
            "(installs the APK over ADB and enables the accessibility + "
            "notification-listener services)."
        )
        return
    print(f"[companion] {len(status.devices)} adb device(s) connected:")
    for device in status.devices:
        model = f", {device.model}" if device.model else ""
        print(f"[companion]   - {device.serial} ({device.state}{model})")
    print(
        "[companion] NEXT (agent): installing the companion onto a phone and enabling "
        "its services is a runtime step, not done by this deploy. Ask the user which "
        "device(s) to set up; for each chosen device run phone_connect(serial=…) then "
        "phone_install_companion — that installs the staged APK over ADB and "
        "auto-enables the accessibility + notification-listener services. Do not "
        "auto-install on every connected device."
    )
