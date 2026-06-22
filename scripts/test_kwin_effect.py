"""Tests for KWin effect build, install, rotating reload, and cleanup helpers."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

import _kwin_effect as kwin_effect


class FakeKwinRunner:
    """Small KWin DBus/cmake/rm simulator for reload policy tests."""

    def __init__(self) -> None:
        self.calls: list[list[str]] = []
        self.listed_effect_ids: set[str] = set()
        self.loaded_effect_ids: set[str] = set()
        self.running_build_id: str | None = None
        self.dbus_reachable = True
        self.service_active = True
        self.load_build_ids: dict[str, str] = {}
        self.load_fail_ids: set[str] = set()
        self.notification_succeeds = True
        self.cleanup_fails = False
        self.kwinrc_values: dict[str, str] = {}

    def __call__(self, command: list[str]) -> subprocess.CompletedProcess[str]:
        self.calls.append(list(command))
        joined = " ".join(command)
        if "org.freedesktop.DBus.Properties.Get" in joined:
            if not self.dbus_reachable:
                return _completed(command, returncode=1, stderr="kwin unavailable")
            property_name = command[-1]
            if property_name == "listOfEffects":
                return _completed(command, stdout="\n".join(sorted(self.listed_effect_ids)))
            if property_name == "loadedEffects":
                return _completed(command, stdout="\n".join(sorted(self.loaded_effect_ids)))
        if "isEffectLoaded" in joined:
            return _completed(
                command,
                stdout="true" if command[-1] in self.loaded_effect_ids else "false",
            )
        if "isEffectSupported" in joined:
            return _completed(command, stdout="true")
        if "BuildId" in joined:
            if self.running_build_id is None:
                return _completed(command, returncode=1, stderr="no such object")
            return _completed(command, stdout=self.running_build_id)
        if "currentDesktop" in joined:
            return _completed(command, returncode=0 if self.dbus_reachable else 1, stdout="1")
        if command[:3] == ["systemctl", "--user", "is-active"]:
            return _completed(
                command,
                returncode=0 if self.service_active else 3,
                stdout="active" if self.service_active else "inactive",
            )
        if command[0] in {"notify-send", "kdialog"}:
            return _completed(command, returncode=0 if self.notification_succeeds else 1)
        if "unloadEffect" in joined:
            effect_id = command[-1]
            self.loaded_effect_ids.discard(effect_id)
            if not self.loaded_effect_ids:
                self.running_build_id = None
            return _completed(command)
        if "loadEffect" in joined:
            effect_id = command[-1]
            if effect_id in self.load_fail_ids:
                return _completed(command, returncode=1, stderr="load failed")
            self.listed_effect_ids.add(effect_id)
            self.loaded_effect_ids.add(effect_id)
            if effect_id in self.load_build_ids:
                self.running_build_id = self.load_build_ids[effect_id]
            return _completed(command)
        if command and command[0] == "kwriteconfig6":
            key = command[command.index("--key") + 1]
            self.kwinrc_values[key] = command[-1]
            return _completed(command)
        if "rm" in command:
            return _completed(
                command,
                returncode=1 if self.cleanup_fails else 0,
                stderr="cleanup failed" if self.cleanup_fails else "",
            )
        return _completed(command)


def _completed(
    command: list[str], *, returncode: int = 0, stdout: str = "", stderr: str = ""
) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(command, returncode, stdout=stdout, stderr=stderr)


class _FakeClock:
    def __init__(self) -> None:
        self.now = 0.0

    def __call__(self) -> float:
        self.now += 0.6
        return self.now


def _write_fake_effect_tree(root: Path) -> None:
    root.mkdir(parents=True)
    (root / "main.cpp").write_text("int main() {}\n", encoding="utf-8")
    (root / "agentcursoreffect.h").write_text("#pragma once\n", encoding="utf-8")
    (root / "CMakeLists.txt").write_text("project(fake)\n", encoding="utf-8")
    (root / "metadata.json.in").write_text(
        '{"KPlugin":{"Id":"@SKY_CUA_EFFECT_ID@"}}\n',
        encoding="utf-8",
    )


def _method_effect_ids(runner: FakeKwinRunner, method: str) -> list[str]:
    needle = f"org.kde.kwin.Effects.{method}"
    return [call[-1] for call in runner.calls if needle in " ".join(call)]


def test_kwin_effect_id_generation_and_filtering() -> None:
    assert kwin_effect.effect_generation("sky-cua-agent-cursor") == 0
    assert kwin_effect.effect_generation("sky-cua-agent-cursor-000042") == 42
    assert kwin_effect.effect_generation("sky-cua-agent-cursor-42") is None
    assert kwin_effect.next_generated_effect_id([]) == "sky-cua-agent-cursor-000001"
    assert (
        kwin_effect.next_generated_effect_id(
            [
                "blur",
                "sky-cua-agent-cursor",
                "sky-cua-agent-cursor-000009",
            ]
        )
        == "sky-cua-agent-cursor-000010"
    )
    assert kwin_effect.sky_cua_effect_ids(
        ["showfps", "sky-cua-agent-cursor-000002", "sky-cua-agent-cursor"]
    ) == ["sky-cua-agent-cursor", "sky-cua-agent-cursor-000002"]


def test_kwin_effect_discovers_candidates_from_listing_install_and_kwinrc(tmp_path: Path) -> None:
    prefix = tmp_path / "prefix"
    plugin_dir = prefix / kwin_effect.KWIN_PLUGIN_RELATIVE_DIR
    plugin_dir.mkdir(parents=True)
    (plugin_dir / "sky-cua-agent-cursor-000003.so").write_text("", encoding="utf-8")
    metadata_dir = prefix / "share" / "kwin" / "effects" / "sky-cua-agent-cursor-000004"
    metadata_dir.mkdir(parents=True)
    kwinrc = tmp_path / "kwinrc"
    kwinrc.write_text(
        "\n".join(
            [
                "[Plugins]",
                "sky-cua-agent-cursorEnabled=true",
                "sky-cua-agent-cursor-000002Enabled=false",
                "sky-cua-agent-cursor-000005Enabled=true",
            ]
        ),
        encoding="utf-8",
    )
    runner = FakeKwinRunner()
    runner.listed_effect_ids = {"blur", "sky-cua-agent-cursor-000001"}

    assert kwin_effect.discover_candidate_effect_ids(
        runner=runner,
        install_prefix=prefix,
        kwinrc_path=kwinrc,
    ) == [
        "sky-cua-agent-cursor",
        "sky-cua-agent-cursor-000001",
        "sky-cua-agent-cursor-000003",
        "sky-cua-agent-cursor-000004",
        "sky-cua-agent-cursor-000005",
    ]


def test_kwin_effect_build_id_is_stable_content_hash(tmp_path: Path) -> None:
    source = tmp_path / "effect"
    _write_fake_effect_tree(source)

    first = kwin_effect.compute_effect_build_id(source)
    second = kwin_effect.compute_effect_build_id(source)
    assert first == second
    assert len(first) == 16

    (source / "main.cpp").write_text("int main() { return 1; }\n", encoding="utf-8")
    assert kwin_effect.compute_effect_build_id(source) != first


def test_kwin_effect_cmake_commands_carry_generated_id_and_build_id(tmp_path: Path) -> None:
    configure = kwin_effect.cmake_configure_command(
        tmp_path / "build",
        install_prefix=Path("/usr"),
        build_id="abc123",
        effect_id="sky-cua-agent-cursor-000123",
    )
    assert "-DCMAKE_INSTALL_PREFIX=/usr" in configure
    assert "-DSKY_CUA_EFFECT_BUILD_ID=abc123" in configure
    assert "-DSKY_CUA_EFFECT_ID=sky-cua-agent-cursor-000123" in configure
    assert not any(arg.startswith("-DSKY_CUA_CURSOR_ASSET=") for arg in configure)
    assert kwin_effect.cmake_build_command(tmp_path / "build")[:2] == ["cmake", "--build"]


def test_kwin_effect_metadata_template_renders_matching_plugin_id() -> None:
    template = (kwin_effect.KWIN_EFFECT_SOURCE_DIR / "metadata.json.in").read_text(encoding="utf-8")
    rendered = template.replace("@SKY_CUA_EFFECT_ID@", "sky-cua-agent-cursor-000777")
    assert json.loads(rendered)["KPlugin"]["Id"] == "sky-cua-agent-cursor-000777"


def test_kwin_effect_source_is_non_visual_pointer_signal_shim() -> None:
    source = kwin_effect.KWIN_EFFECT_SOURCE_DIR
    header = (source / "agentcursoreffect.h").read_text(encoding="utf-8")
    implementation = (source / "agentcursoreffect.cpp").read_text(encoding="utf-8")
    cmake = (source / "CMakeLists.txt").read_text(encoding="utf-8")

    combined = header + implementation + cmake
    assert "QuickSceneEffect" not in combined
    assert "QML" not in combined
    assert not (source / "qml" / "main.qml").exists()
    assert "void PointerMoved(double x, double y, qulonglong sequence);" in header
    assert "PointerStateJson" in implementation
    assert "ExportAllSignals" in implementation
    assert "SKY_CUA_KWIN_EFFECT_HAS_POINTER_MOTION" in cmake


def test_kwin_effect_install_command_uses_sudo_prefix(tmp_path: Path) -> None:
    default = kwin_effect.cmake_install_command(tmp_path / "build")
    assert default[:1] == ["sudo"]
    custom = kwin_effect.cmake_install_command(tmp_path / "build", sudo_cmd=["doas", "-n"])
    assert custom[:2] == ["doas", "-n"]
    bare = kwin_effect.cmake_install_command(tmp_path / "build", sudo_cmd=[])
    assert bare[:2] == ["cmake", "--install"]


def test_kwin_effect_enable_and_delete_config_command_shape() -> None:
    assert kwin_effect.effect_enabled_config_command(
        True,
        effect_id="sky-cua-agent-cursor-000123",
    ) == [
        "kwriteconfig6",
        "--file",
        "kwinrc",
        "--group",
        "Plugins",
        "--key",
        "sky-cua-agent-cursor-000123Enabled",
        "true",
    ]
    assert kwin_effect.effect_enabled_config_command(False)[-1] == "false"
    assert kwin_effect.effect_delete_config_command("sky-cua-agent-cursor-000123")[-1] == (
        "--delete"
    )


def test_kwin_effect_load_path_unloads_previous_before_new_load(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    runner = FakeKwinRunner()
    old_id = "sky-cua-agent-cursor-000001"
    new_id = "sky-cua-agent-cursor-000002"
    runner.listed_effect_ids = {old_id, new_id}
    runner.loaded_effect_ids = {old_id}
    runner.running_build_id = "old-build"
    runner.load_build_ids[new_id] = "new-build"

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new-build",
        effect_id=new_id,
        previous_effect_ids=[old_id],
        install_prefix=tmp_path / "prefix",
        sudo_cmd=[],
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
    )

    assert outcome.converged
    unload_calls = _method_effect_ids(runner, "unloadEffect")
    load_calls = _method_effect_ids(runner, "loadEffect")
    assert unload_calls[0] == old_id
    assert load_calls[0] == new_id
    old_unload = next(
        call for call in runner.calls if "org.kde.kwin.Effects.unloadEffect" in " ".join(call)
    )
    new_load = next(
        call for call in runner.calls if "org.kde.kwin.Effects.loadEffect" in " ".join(call)
    )
    assert runner.calls.index(old_unload) < runner.calls.index(new_load)


def test_kwin_effect_successful_rotating_deploy_cleans_old_ids_without_notify(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    runner = FakeKwinRunner()
    old_id = "sky-cua-agent-cursor-000007"
    new_id = "sky-cua-agent-cursor-000008"
    runner.listed_effect_ids = {old_id, new_id}
    runner.loaded_effect_ids = {old_id}
    runner.running_build_id = "old-build"
    runner.load_build_ids[new_id] = "new-build"
    prefix = tmp_path / "prefix"

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new-build",
        effect_id=new_id,
        previous_effect_ids=["sky-cua-agent-cursor", old_id],
        install_prefix=prefix,
        sudo_cmd=[],
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
    )

    assert outcome.converged
    assert not outcome.session_restart_required
    assert not outcome.cleanup_warnings
    assert not any(call[0] in {"notify-send", "kdialog"} for call in runner.calls)
    assert not any(call[:3] == ["systemctl", "--user", "restart"] for call in runner.calls)
    delete_keys = [
        call[call.index("--key") + 1]
        for call in runner.calls
        if call[:1] == ["kwriteconfig6"] and call[-1] == "--delete"
    ]
    assert "sky-cua-agent-cursorEnabled" in delete_keys
    assert f"{old_id}Enabled" in delete_keys
    rm_calls = [call for call in runner.calls if "rm" in call]
    assert rm_calls
    assert str(prefix / kwin_effect.KWIN_PLUGIN_RELATIVE_DIR / f"{old_id}.so") in rm_calls[-1]
    assert not any("*" in item for call in rm_calls for item in call)


def test_kwin_effect_failed_new_load_rolls_back_and_keeps_previous_files(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    runner = FakeKwinRunner()
    old_id = "sky-cua-agent-cursor-000003"
    new_id = "sky-cua-agent-cursor-000004"
    runner.listed_effect_ids = {old_id, new_id}
    runner.loaded_effect_ids = {old_id}
    runner.running_build_id = "old-build"
    runner.load_build_ids[old_id] = "old-build"

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new-build",
        effect_id=new_id,
        previous_effect_ids=[old_id],
        install_prefix=tmp_path / "prefix",
        sudo_cmd=[],
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
    )

    assert not outcome.converged
    assert outcome.rollback_effect_id == old_id
    assert outcome.active_effect_id == old_id
    assert outcome.live_load_attempted
    assert kwin_effect.kwin_effect_deploy_failed(outcome)
    assert runner.kwinrc_values[f"{new_id}Enabled"] == "false"
    assert runner.kwinrc_values[f"{old_id}Enabled"] == "true"
    assert not any("rm" in call for call in runner.calls)
    load_calls = _method_effect_ids(runner, "loadEffect")
    assert load_calls[:2] == [new_id, old_id]


def test_kwin_effect_dbus_unreachable_enables_new_id_without_live_load(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    runner = FakeKwinRunner()
    runner.dbus_reachable = False
    old_id = "sky-cua-agent-cursor-000009"
    new_id = "sky-cua-agent-cursor-000010"

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new-build",
        effect_id=new_id,
        previous_effect_ids=[old_id],
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
    )

    assert not outcome.converged
    assert not outcome.session_restart_required
    assert not outcome.live_load_attempted
    assert not kwin_effect.kwin_effect_deploy_failed(outcome)
    assert any("next Plasma session start" in note for note in outcome.notes)
    assert _method_effect_ids(runner, "loadEffect") == []
    assert _method_effect_ids(runner, "unloadEffect") == []
    assert runner.kwinrc_values[f"{new_id}Enabled"] == "true"
    assert runner.kwinrc_values[f"{old_id}Enabled"] == "false"


def test_kwin_effect_status_reports_rotating_fields(tmp_path: Path) -> None:
    runner = FakeKwinRunner()
    active = "sky-cua-agent-cursor-000002"
    runner.listed_effect_ids = {"sky-cua-agent-cursor", "sky-cua-agent-cursor-000001", active}
    runner.loaded_effect_ids = {active}
    runner.running_build_id = "active-build"
    kwinrc = tmp_path / "kwinrc"
    kwinrc.write_text("[Plugins]\nsky-cua-agent-cursorEnabled=true\n", encoding="utf-8")

    status = kwin_effect.effect_status(
        runner=runner,
        install_prefix=tmp_path / "prefix",
        kwinrc_path=kwinrc,
    )

    assert status["reload_strategy"] == "rotating_effect_id"
    assert status["base_effect_id"] == "sky-cua-agent-cursor"
    assert status["effect_id"] == active
    assert status["active_effect_id"] == active
    assert status["loaded"] is True
    assert status["running_build_id"] == "active-build"
    assert status["loaded_effect_ids"] == [active]
    assert status["stale_effect_ids"] == [
        "sky-cua-agent-cursor",
        "sky-cua-agent-cursor-000001",
    ]


def test_kwin_effect_deploy_picks_next_id_and_installs_generated_target(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    monkeypatch.setattr(kwin_effect, "kwin_effect_preconditions", lambda: [])
    monkeypatch.setattr(kwin_effect, "compute_effect_build_id", lambda: "new-build")
    monkeypatch.setattr(kwin_effect, "KWINRC_PATH", tmp_path / "kwinrc")
    runner = FakeKwinRunner()
    old_id = "sky-cua-agent-cursor-000003"
    new_id = "sky-cua-agent-cursor-000004"
    runner.listed_effect_ids = {"sky-cua-agent-cursor", old_id}
    runner.loaded_effect_ids = {old_id}
    runner.running_build_id = "old-build"
    runner.load_build_ids[new_id] = "new-build"

    outcome = kwin_effect.deploy_kwin_effect(
        build_dir=tmp_path / "build",
        install_prefix=tmp_path / "prefix",
        sudo_cmd=[],
        runner=runner,
        echo=lambda _message: None,
    )

    assert outcome.converged
    assert outcome.effect_id == new_id
    configure_calls = [call for call in runner.calls if call[:1] == ["cmake"] and "-S" in call]
    assert configure_calls
    assert f"-DSKY_CUA_EFFECT_ID={new_id}" in configure_calls[0]


def test_kwin_effect_legacy_build_id_reports_unknown() -> None:
    runner = FakeKwinRunner()
    runner.running_build_id = None
    assert kwin_effect.running_effect_build_id(runner=runner) == "unknown"


def test_kwin_effect_notify_helper_reports_unavailable_tools() -> None:
    runner = FakeKwinRunner()
    delivered, how = kwin_effect.notify_effect_update_pending(
        runner=runner,
        which=lambda _tool: None,
    )
    assert not delivered
    assert "no notification tool" in how


def test_kwin_effect_preconditions_report_missing_tools(tmp_path: Path) -> None:
    missing = kwin_effect.kwin_effect_preconditions(
        platform="linux",
        which=lambda tool: None if tool == "ninja" else f"/usr/bin/{tool}",
        kwin_header=tmp_path / "missing-header.h",
    )
    assert any("ninja" in item for item in missing)
    assert any("headers" in item for item in missing)
    assert not any("cursor asset" in item for item in missing)
    assert kwin_effect.kwin_effect_preconditions(platform="win32") == [
        "KWin effect deploy requires Linux (platform is win32)"
    ]
