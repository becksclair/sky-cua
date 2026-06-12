"""Tests for the one-shot installer orchestration logic."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

import installer


def completed(returncode: int = 0) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(args=[], returncode=returncode, stdout="", stderr="")


def test_detect_package_manager_prefers_pacman() -> None:
    assert installer.detect_package_manager(lambda name: "/usr/bin/" + name) == "pacman"
    assert (
        installer.detect_package_manager(
            lambda name: "/usr/bin/apt-get" if name == "apt-get" else None
        )
        == "apt"
    )
    assert installer.detect_package_manager(lambda name: None) is None


def test_missing_packages_filters_installed() -> None:
    packages = {"pacman": ("alpha", "beta", "gamma")}
    missing = installer.missing_packages("pacman", lambda package: package == "beta", packages)
    assert missing == ["alpha", "gamma"]
    assert installer.missing_packages("apt", lambda package: False, packages) == []


def test_detect_agents_from_home_layout(tmp_path: Path) -> None:
    (tmp_path / ".claude").mkdir()
    (tmp_path / ".pi" / "agent").mkdir(parents=True)
    (tmp_path / ".config" / "Claude").mkdir(parents=True)

    detected = installer.detect_agents(home=tmp_path, which=lambda name: None)
    assert detected["claude-code"] is True
    assert detected["claude-desktop"] is True
    assert detected["pi"] is True
    assert detected["codex"] is False
    assert detected["opencode"] is False
    assert detected["openclaw"] is False


def test_detect_agents_from_path_commands(tmp_path: Path) -> None:
    available = {"codex", "opencode", "openclaw"}
    detected = installer.detect_agents(
        home=tmp_path,
        which=lambda name: "/usr/bin/" + name if name in available else None,
    )
    assert detected["codex"] is True
    assert detected["opencode"] is True
    assert detected["openclaw"] is True
    assert detected["claude-code"] is False
    assert detected["pi"] is False


def test_select_agents_explicit_request_wins() -> None:
    detected: dict[str, bool] = dict.fromkeys(installer.KNOWN_AGENTS, True)
    assert installer.select_agents("codex, pi", detected) == ["codex", "pi"]
    assert installer.select_agents("codex,codex", detected) == ["codex"]


def test_select_agents_rejects_unknown_names() -> None:
    with pytest.raises(ValueError, match="unknown agent 'cursor'"):
        installer.select_agents("cursor", {})


def test_select_agents_defaults_to_detected_order() -> None:
    detected = {"pi": True, "codex": True, "opencode": False}
    assert installer.select_agents(None, detected) == ["codex", "pi"]


def test_missing_required_commands() -> None:
    assert installer.missing_required_commands(lambda name: None) == ["cargo", "git"]
    assert installer.missing_required_commands(lambda name: "/usr/bin/" + name) == []


def test_build_phase_reports_failure() -> None:
    calls: list[list[str]] = []

    def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return completed(returncode=1)

    result = installer.run_build_phase(skip=False, runner=runner)
    assert result.failed
    assert calls and calls[0][1].endswith("build_plugin.py")


def test_build_phase_skip() -> None:
    result = installer.run_build_phase(skip=True)
    assert result.status == "skipped"


def test_codex_phase_skipped_when_not_selected() -> None:
    result = installer.run_codex_phase(
        enabled=False,
        codex_home=Path("/tmp/codex"),
        marketplace_root=None,
        marketplace_source=None,
    )
    assert result.status == "skipped"


def test_codex_phase_passes_marketplace_arguments(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(installer.shutil, "which", lambda name: "/usr/bin/codex")
    calls: list[list[str]] = []

    def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return completed()

    result = installer.run_codex_phase(
        enabled=True,
        codex_home=Path("/tmp/codex-home"),
        marketplace_root=Path("/tmp/marketplace"),
        marketplace_source="example/source",
        runner=runner,
    )
    assert result.status == "ok"
    command = calls[0]
    assert command[1].endswith("setup_heliasar_marketplace.py")
    assert "--codex-home" in command and "/tmp/codex-home" in command
    assert "--marketplace-root" in command and "/tmp/marketplace" in command
    assert "--marketplace-source" in command and "example/source" in command


def test_agent_phase_invokes_install_mcp_server() -> None:
    calls: list[list[str]] = []

    def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return completed()

    result = installer.run_agent_phase("pi", target_dir=Path("/tmp/target"), runner=runner)
    assert result.status == "ok"
    command = calls[0]
    assert command[1].endswith("install_mcp_server.py")
    assert command[command.index("--host") : command.index("--host") + 2] == ["--host", "pi"]
    assert "--restart-runtime" in command


def test_kwin_phase_skipped_by_default(tmp_path: Path) -> None:
    result = installer.run_kwin_phase(enabled=False, target_dir=tmp_path)
    assert result.status == "skipped"


def test_main_dry_run_lists_phases(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    def no_agents_detected(*args: object, **kwargs: object) -> dict[str, bool]:
        return dict.fromkeys(installer.KNOWN_AGENTS, False)

    monkeypatch.setattr(installer, "detect_agents", no_agents_detected)
    exit_code = installer.main(["--dry-run", "--agents", "codex,pi"])
    output = capsys.readouterr().out
    assert exit_code == 0
    assert "codex" in output
    assert "agent:pi" in output
    assert "health" in output


def test_main_rejects_unknown_agent(capsys: pytest.CaptureFixture[str]) -> None:
    exit_code = installer.main(["--dry-run", "--agents", "nope"])
    assert exit_code == 2
    assert "unknown agent" in capsys.readouterr().err
