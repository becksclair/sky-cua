"""Tests for KWin effect build, install, and reload helpers."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

import _kwin_effect as kwin_effect


class FakeKwinRunner:
    """Scriptable runner: maps command predicates to canned results."""

    def __init__(self) -> None:
        self.calls: list[list[str]] = []
        self.effect_loaded = False
        self.running_build_id: str | None = None
        self.dbus_reachable = True
        self.service_active = True
        self.hot_reload_updates_build_id = False
        self.hot_reload_target: str | None = None
        self.notification_succeeds = True

    def __call__(self, command: list[str]) -> subprocess.CompletedProcess[str]:
        self.calls.append(list(command))
        joined = " ".join(command)
        if "isEffectLoaded" in joined:
            return _completed(command, stdout="true" if self.effect_loaded else "false")
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
            self.effect_loaded = False
            return _completed(command)
        if "loadEffect" in joined:
            self.effect_loaded = True
            if self.hot_reload_updates_build_id:
                self.running_build_id = self.hot_reload_target
            return _completed(command)
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
    (root / "qml").mkdir(parents=True)
    (root / "main.cpp").write_text("int main() {}\n", encoding="utf-8")
    (root / "agentcursoreffect.h").write_text("#pragma once\n", encoding="utf-8")
    (root / "CMakeLists.txt").write_text("project(fake)\n", encoding="utf-8")
    (root / "metadata.json").write_text("{}\n", encoding="utf-8")
    (root / "qml" / "main.qml").write_text("Item {}\n", encoding="utf-8")


def test_kwin_effect_build_id_is_stable_content_hash(tmp_path: Path) -> None:
    source = tmp_path / "effect"
    _write_fake_effect_tree(source)
    asset = tmp_path / "cursor.png"
    asset.write_bytes(b"png-bytes")

    first = kwin_effect.compute_effect_build_id(source, asset)
    second = kwin_effect.compute_effect_build_id(source, asset)
    assert first == second
    assert len(first) == 16

    (source / "main.cpp").write_text("int main() { return 1; }\n", encoding="utf-8")
    assert kwin_effect.compute_effect_build_id(source, asset) != first


def test_kwin_effect_cmake_commands_carry_build_id_prefix_and_asset(tmp_path: Path) -> None:
    configure = kwin_effect.cmake_configure_command(
        tmp_path / "build", install_prefix=Path("/usr"), build_id="abc123"
    )
    assert "-DCMAKE_INSTALL_PREFIX=/usr" in configure
    assert "-DSKY_CUA_EFFECT_BUILD_ID=abc123" in configure
    assert any(arg.startswith("-DSKY_CUA_CURSOR_ASSET=") for arg in configure)
    assert kwin_effect.cmake_build_command(tmp_path / "build")[:2] == ["cmake", "--build"]


def test_kwin_effect_install_command_uses_sudo_prefix(tmp_path: Path) -> None:
    default = kwin_effect.cmake_install_command(tmp_path / "build")
    assert default[:1] == ["sudo"]
    custom = kwin_effect.cmake_install_command(tmp_path / "build", sudo_cmd=["doas", "-n"])
    assert custom[:2] == ["doas", "-n"]
    bare = kwin_effect.cmake_install_command(tmp_path / "build", sudo_cmd=[])
    assert bare[:2] == ["cmake", "--install"]


def test_kwin_effect_enable_config_command_shape() -> None:
    assert kwin_effect.effect_enabled_config_command(True) == [
        "kwriteconfig6",
        "--file",
        "kwinrc",
        "--group",
        "Plugins",
        "--key",
        "sky-cua-agent-cursorEnabled",
        "true",
    ]
    assert kwin_effect.effect_enabled_config_command(False)[-1] == "false"


def test_kwin_effect_update_notification_command_shapes() -> None:
    notify = kwin_effect.update_notification_command()
    assert notify[0] == "notify-send"
    assert "sky-cua KWin effect updated" in notify
    fallback = kwin_effect.update_notification_fallback_command()
    assert fallback[:2] == ["kdialog", "--title"]
    assert "--passivepopup" in fallback
    # Neither command may restart anything.
    assert "systemctl" not in notify and "systemctl" not in fallback


def test_kwin_effect_reload_converges_without_restart(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    runner = FakeKwinRunner()
    runner.effect_loaded = True
    runner.running_build_id = "old0000000000000"
    runner.hot_reload_updates_build_id = True
    runner.hot_reload_target = "new0000000000000"

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new0000000000000",
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
    )

    assert outcome.converged
    assert not outcome.session_restart_required
    assert not any(call[0] == "notify-send" for call in runner.calls)
    assert not any(call[:3] == ["systemctl", "--user", "restart"] for call in runner.calls)


def test_kwin_effect_reload_notifies_when_update_needs_session_restart(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    runner = FakeKwinRunner()
    runner.effect_loaded = True
    runner.running_build_id = "old0000000000000"

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new0000000000000",
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
        which=lambda tool: f"/usr/bin/{tool}" if tool == "notify-send" else None,
    )

    assert not outcome.converged
    assert outcome.session_restart_required
    assert any(call[0] == "notify-send" for call in runner.calls)
    # The deploy must never restart KWin itself.
    assert not any(call[:3] == ["systemctl", "--user", "restart"] for call in runner.calls)
    assert any("user notified via notify-send" in note for note in outcome.notes)
    assert outcome.notification_delivered


def test_kwin_effect_reload_notification_falls_back_to_kdialog(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    runner = FakeKwinRunner()
    runner.effect_loaded = True
    runner.running_build_id = "old0000000000000"

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new0000000000000",
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
        which=lambda tool: f"/usr/bin/{tool}" if tool == "kdialog" else None,
    )

    assert outcome.session_restart_required
    kdialog_calls = [call for call in runner.calls if call[0] == "kdialog"]
    assert kdialog_calls and "--passivepopup" in kdialog_calls[0]
    assert any("kdialog passive popup" in note for note in outcome.notes)


def test_kwin_effect_reload_respects_no_notify(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    runner = FakeKwinRunner()
    runner.effect_loaded = True
    runner.running_build_id = "old0000000000000"

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new0000000000000",
        notify=False,
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
    )

    assert outcome.session_restart_required
    assert not outcome.notification_delivered
    assert not any(call[0] in {"notify-send", "kdialog"} for call in runner.calls)


def test_kwin_effect_reload_skips_when_kwin_dbus_unreachable() -> None:
    runner = FakeKwinRunner()
    runner.dbus_reachable = False

    outcome = kwin_effect.reload_effect_until_converged(
        expected_build_id="new0000000000000",
        runner=runner,
        sleep=lambda _s: None,
        clock=_FakeClock(),
    )

    assert not outcome.converged
    assert not outcome.session_restart_required
    assert any("next Plasma session start" in note for note in outcome.notes)
    # kwinrc enable still happens so the effect autoloads later
    assert any(call[0] == "kwriteconfig6" for call in runner.calls)


def test_kwin_effect_legacy_build_id_reports_unknown() -> None:
    runner = FakeKwinRunner()
    runner.running_build_id = None
    assert kwin_effect.running_effect_build_id(runner=runner) == "unknown"


def test_kwin_effect_notify_helper_reports_unavailable_tools() -> None:
    runner = FakeKwinRunner()
    delivered, how = kwin_effect.notify_effect_update_pending(
        runner=runner, which=lambda _tool: None
    )
    assert not delivered
    assert "no notification tool" in how


def test_kwin_effect_preconditions_report_missing_tools(tmp_path: Path) -> None:
    missing = kwin_effect.kwin_effect_preconditions(
        platform="linux",
        which=lambda tool: None if tool == "ninja" else f"/usr/bin/{tool}",
        kwin_header=tmp_path / "missing-header.h",
        cursor_asset=tmp_path / "missing-cursor.png",
    )
    assert any("ninja" in item for item in missing)
    assert any("headers" in item for item in missing)
    assert any("cursor asset" in item for item in missing)
    assert kwin_effect.kwin_effect_preconditions(platform="win32") == [
        "KWin effect deploy requires Linux (platform is win32)"
    ]
