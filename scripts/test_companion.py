"""Unit tests for the Android companion build/stage lane (`_companion`).

The decision logic (toolchain resolution, change detection, staging, and the
orchestrator's skip/build/fail branches) is tested without running Gradle.
"""

from __future__ import annotations

import os
import subprocess
from collections.abc import Mapping
from pathlib import Path

import pytest

import _companion


def make_jdk(root: Path) -> Path:
    (root / "bin").mkdir(parents=True)
    (root / "bin" / "javac").write_text("", encoding="utf-8")
    return root


def test_resolve_java_home_prefers_explicit_override(tmp_path: Path) -> None:
    jdk = make_jdk(tmp_path / "jdk21")
    env = {_companion.COMPANION_JAVA_HOME_ENV: str(jdk)}
    assert _companion.resolve_java_home(env, candidates=()) == jdk


def test_resolve_java_home_trusts_a_21_shaped_java_home(tmp_path: Path) -> None:
    jdk = make_jdk(tmp_path / "java-21-openjdk")
    assert _companion.resolve_java_home({"JAVA_HOME": str(jdk)}, candidates=()) == jdk


def test_resolve_java_home_ignores_a_non_21_java_home(tmp_path: Path) -> None:
    # The host default JDK is newer than 21 and rejected by AGP; an inherited
    # JAVA_HOME pointing at it must not be selected.
    jdk = make_jdk(tmp_path / "java-26-openjdk")
    assert _companion.resolve_java_home({"JAVA_HOME": str(jdk)}, candidates=()) is None


def test_resolve_java_home_falls_back_to_distro_candidates(tmp_path: Path) -> None:
    jdk = make_jdk(tmp_path / "jdk-21")
    resolved = _companion.resolve_java_home({}, candidates=(tmp_path / "missing", jdk))
    assert resolved == jdk


def test_resolve_java_home_none_when_absent(tmp_path: Path) -> None:
    assert _companion.resolve_java_home({}, candidates=(tmp_path / "nope",)) is None


def test_resolve_android_sdk_root_prefers_env(tmp_path: Path) -> None:
    sdk = tmp_path / "sdk"
    sdk.mkdir()
    resolved = _companion.resolve_android_sdk_root(
        {"ANDROID_SDK_ROOT": str(sdk)}, local_properties=tmp_path / "none"
    )
    assert resolved == sdk


def test_resolve_android_sdk_root_default_home(tmp_path: Path) -> None:
    sdk = tmp_path / "Android" / "Sdk"
    sdk.mkdir(parents=True)
    resolved = _companion.resolve_android_sdk_root(
        {"HOME": str(tmp_path)}, local_properties=tmp_path / "none"
    )
    assert resolved == sdk


def test_resolve_android_sdk_root_local_properties(tmp_path: Path) -> None:
    sdk = tmp_path / "android-sdk"
    sdk.mkdir()
    props = tmp_path / "local.properties"
    props.write_text(f"sdk.dir={sdk}\n", encoding="utf-8")
    assert _companion.resolve_android_sdk_root({}, local_properties=props) == sdk


def test_resolve_android_sdk_root_none(tmp_path: Path) -> None:
    assert _companion.resolve_android_sdk_root({}, local_properties=tmp_path / "none") is None


def test_companion_sources_changed_missing_apk(tmp_path: Path) -> None:
    assert _companion.companion_sources_changed(
        staged_apk=tmp_path / "missing.apk", source_paths=()
    )


def test_companion_sources_changed_detects_newer_source(tmp_path: Path) -> None:
    apk = tmp_path / "staged.apk"
    apk.write_bytes(b"x")
    source_dir = tmp_path / "src"
    source_dir.mkdir()
    kt = source_dir / "Main.kt"
    kt.write_text("a", encoding="utf-8")
    newer = apk.stat().st_mtime + 10
    os.utime(kt, (newer, newer))
    assert _companion.companion_sources_changed(staged_apk=apk, source_paths=(source_dir,))


def test_companion_sources_unchanged(tmp_path: Path) -> None:
    source_dir = tmp_path / "src"
    source_dir.mkdir()
    kt = source_dir / "Main.kt"
    kt.write_text("a", encoding="utf-8")
    apk = tmp_path / "staged.apk"
    apk.write_bytes(b"x")
    newer = kt.stat().st_mtime + 10
    os.utime(apk, (newer, newer))
    assert not _companion.companion_sources_changed(staged_apk=apk, source_paths=(source_dir,))


def test_stage_companion_artifacts_copies_apk_and_metadata(tmp_path: Path) -> None:
    built_apk = tmp_path / "app-debug.apk"
    built_apk.write_bytes(b"apk-bytes")
    built_metadata = tmp_path / "build-metadata.json"
    built_metadata.write_text('{"package":"x"}', encoding="utf-8")
    dest_apk = tmp_path / "out" / "phone-companion.apk"
    dest_metadata = tmp_path / "out" / "phone-companion.json"
    _companion.stage_companion_artifacts(
        built_apk=built_apk,
        built_metadata=built_metadata,
        staged_apk=dest_apk,
        staged_metadata=dest_metadata,
    )
    assert dest_apk.read_bytes() == b"apk-bytes"
    assert dest_metadata.read_text(encoding="utf-8") == '{"package":"x"}'


def test_stage_companion_artifacts_drops_stale_sidecar_when_no_metadata(
    tmp_path: Path,
) -> None:
    # A build that emits an APK but no metadata must not leave a stale sidecar
    # next to the fresh APK: the runtime signature gate would compare the wrong
    # cert/APK hash. The staged sidecar is removed so the runtime falls back to
    # all-None instead.
    built_apk = tmp_path / "app-debug.apk"
    built_apk.write_bytes(b"apk-bytes")
    dest_apk = tmp_path / "out" / "phone-companion.apk"
    dest_metadata = tmp_path / "out" / "phone-companion.json"
    dest_metadata.parent.mkdir(parents=True)
    dest_metadata.write_text('{"signing_cert_sha256":"stale"}', encoding="utf-8")
    _companion.stage_companion_artifacts(
        built_apk=built_apk,
        built_metadata=tmp_path / "absent-build-metadata.json",
        staged_apk=dest_apk,
        staged_metadata=dest_metadata,
    )
    assert dest_apk.read_bytes() == b"apk-bytes"
    assert not dest_metadata.exists()


def test_stage_companion_artifacts_missing_apk_raises(tmp_path: Path) -> None:
    with pytest.raises(RuntimeError, match="no APK"):
        _companion.stage_companion_artifacts(
            built_apk=tmp_path / "nope.apk",
            built_metadata=tmp_path / "m.json",
            staged_apk=tmp_path / "a.apk",
            staged_metadata=tmp_path / "m2.json",
        )


def _toolchain() -> _companion.CompanionToolchain:
    return _companion.CompanionToolchain(
        java_home=Path("/opt/jdk21"), android_sdk_root=Path("/opt/android-sdk")
    )


def _explode_runner(
    _command: list[str], _env: Mapping[str, str]
) -> subprocess.CompletedProcess[str]:
    raise AssertionError("Gradle must not run on a skip path")


def test_build_and_stage_skips_without_toolchain(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(_companion, "resolve_companion_toolchain", lambda env=None: None)
    outcome = _companion.build_and_stage_companion(runner=_explode_runner, echo=lambda _m: None)
    assert outcome.status == "skipped_no_toolchain"
    assert not outcome.built


def test_build_and_stage_skips_when_unchanged(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(_companion, "resolve_companion_toolchain", lambda env=None: _toolchain())
    monkeypatch.setattr(_companion, "companion_sources_changed", lambda **_k: False)
    outcome = _companion.build_and_stage_companion(runner=_explode_runner, echo=lambda _m: None)
    assert outcome.status == "skipped_unchanged"


def test_build_and_stage_builds_and_stages(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(_companion, "resolve_companion_toolchain", lambda env=None: _toolchain())
    monkeypatch.setattr(_companion, "companion_sources_changed", lambda **_k: True)
    staged: dict[str, bool] = {}
    monkeypatch.setattr(
        _companion, "stage_companion_artifacts", lambda **_k: staged.__setitem__("called", True)
    )
    recorded: dict[str, object] = {}

    def runner(command: list[str], env: Mapping[str, str]) -> subprocess.CompletedProcess[str]:
        recorded["command"] = command
        recorded["env"] = dict(env)
        return subprocess.CompletedProcess(command, 0, "", "")

    outcome = _companion.build_and_stage_companion(runner=runner, echo=lambda _m: None)
    assert outcome.status == "built"
    assert staged.get("called")
    assert ":app:assembleDebug" in recorded["command"]  # type: ignore[operator]
    assert recorded["env"] == {"JAVA_HOME": "/opt/jdk21", "ANDROID_SDK_ROOT": "/opt/android-sdk"}


def test_build_and_stage_raises_on_gradle_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(_companion, "resolve_companion_toolchain", lambda env=None: _toolchain())

    def runner(command: list[str], _env: Mapping[str, str]) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(command, 1, "", "compile error")

    with pytest.raises(RuntimeError, match="Gradle build failed"):
        _companion.build_and_stage_companion(runner=runner, echo=lambda _m: None, force=True)


_DEVICES_LISTING = (
    "List of devices attached\n"
    "* daemon started successfully\n"
    "emulator-5554          device product:sdk_gphone64 model:sdk_gphone64_x86_64 "
    "device:emu64xa transport_id:2\n"
    "100.70.24.74:41937     device product:m3qxeea model:SM_S948B device:m3q transport_id:3\n"
    "11223344               unauthorized usb:1-1\n"
)


def test_parse_adb_devices_extracts_serial_state_model() -> None:
    devices = _companion.parse_adb_devices(_DEVICES_LISTING)
    assert [(d.serial, d.state, d.model) for d in devices] == [
        ("emulator-5554", "device", "sdk_gphone64_x86_64"),
        ("100.70.24.74:41937", "device", "SM_S948B"),
        ("11223344", "unauthorized", None),
    ]


def test_parse_adb_devices_ignores_header_and_daemon_lines() -> None:
    assert _companion.parse_adb_devices("List of devices attached\n* daemon not running\n") == []


def test_read_staged_companion_metadata_reads_json(tmp_path: Path) -> None:
    meta = tmp_path / "phone-companion.json"
    meta.write_text('{"version_name": "0.1.0", "version_code": 1}', encoding="utf-8")
    parsed = _companion.read_staged_companion_metadata(meta)
    assert parsed is not None
    assert parsed["version_name"] == "0.1.0"


def test_read_staged_companion_metadata_missing_or_bad(tmp_path: Path) -> None:
    assert _companion.read_staged_companion_metadata(tmp_path / "absent.json") is None
    bad = tmp_path / "bad.json"
    bad.write_text("not json", encoding="utf-8")
    assert _companion.read_staged_companion_metadata(bad) is None


def test_list_adb_devices_none_when_adb_unavailable() -> None:
    devices = _companion.list_adb_devices(runner=_explode_runner, env={}, which=lambda _name: None)
    assert devices == []


def test_companion_setup_status_assembles_identity_and_devices(tmp_path: Path) -> None:
    apk = tmp_path / "phone-companion.apk"
    apk.write_bytes(b"apk")
    meta = tmp_path / "phone-companion.json"
    meta.write_text(
        '{"version_name": "0.1.0", "version_code": 1, "apk_sha256": "abc123"}',
        encoding="utf-8",
    )

    def runner(_command: list[str], _env: Mapping[str, str]) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(_command, 0, _DEVICES_LISTING, "")

    status = _companion.companion_setup_status(
        staged_apk=apk,
        staged_metadata=meta,
        runner=runner,
        env={"SKY_CUA_ADB": "adb"},
    )
    assert status.staged
    assert status.version_name == "0.1.0"
    assert status.version_code == 1
    assert len(status.devices) == 3


def test_companion_setup_status_not_staged_when_apk_missing(tmp_path: Path) -> None:
    status = _companion.companion_setup_status(
        staged_apk=tmp_path / "absent.apk",
        staged_metadata=tmp_path / "absent.json",
        runner=_explode_runner,
        env={},
    )
    assert not status.staged


def test_print_companion_setup_status_directs_the_agent(capsys: pytest.CaptureFixture[str]) -> None:
    status = _companion.CompanionSetupStatus(
        staged=True,
        version_name="0.1.0",
        version_code=1,
        apk_sha256="b0158e07c3db6359810ec7ef4bd566c7dcbdb942effc6223844246454fc895bb",
        devices=(_companion.AdbDevice("emulator-5554", "device", "sdk_gphone64_x86_64"),),
    )
    _companion.print_companion_setup_status(status)
    out = capsys.readouterr().out
    assert "emulator-5554" in out
    assert "phone_install_companion" in out
    assert "Do not\nauto-install" in out or "auto-install on every connected device" in out


def test_print_companion_setup_status_silent_without_bundle(
    capsys: pytest.CaptureFixture[str],
) -> None:
    status = _companion.CompanionSetupStatus(
        staged=False, version_name=None, version_code=None, apk_sha256=None, devices=()
    )
    _companion.print_companion_setup_status(status)
    assert capsys.readouterr().out == ""
